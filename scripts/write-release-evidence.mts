import { createHash } from "node:crypto";
import { readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const outputDirectory = join(process.cwd(), "dist");
const version = requireVersion();
const tag = process.env.GITHUB_REF_NAME || `local-${version}`;
const commit = process.env.GITHUB_SHA || "local-uncommitted";
const ciRunUrl =
  process.env.GITHUB_SERVER_URL && process.env.GITHUB_REPOSITORY && process.env.GITHUB_RUN_ID
    ? `${process.env.GITHUB_SERVER_URL}/${process.env.GITHUB_REPOSITORY}/actions/runs/${process.env.GITHUB_RUN_ID}`
    : "local";
const artifacts = readdirSync(outputDirectory)
  .filter((name) => /\.(?:exe|blockmap|dmg|zip|appimage|deb)$/i.test(name))
  .sort()
  .map((name) => ({
    name,
    sha256: createHash("sha256")
      .update(readFileSync(join(outputDirectory, name)))
      .digest("hex"),
  }));

if (artifacts.length === 0) {
  throw new Error("Release evidence requires at least one packaged application artifact.");
}

writeFileSync(
  join(outputDirectory, "release-evidence.json"),
  `${JSON.stringify(
    {
      artifacts,
      ciRunUrl,
      commit,
      generatedAt: new Date().toISOString(),
      node: process.version,
      packageManager: process.env.npm_config_user_agent || "unknown",
      tag,
      version,
    },
    null,
    2,
  )}\n`,
);
console.log(`Release evidence written for ${tag} (${artifacts.length} artifacts).`);

function requireVersion() {
  return JSON.parse(readFileSync("package.json", "utf8")).version as string;
}
