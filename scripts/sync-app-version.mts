import { readFile, writeFile } from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

const projectRoot = fileURLToPath(new URL("..", import.meta.url));
const { version } = JSON.parse(await readFile(resolve(projectRoot, "package.json"), "utf8")) as { version: string };
const manifestPath = resolve(projectRoot, "Cargo.toml");
const manifest = await readFile(manifestPath, "utf8");
const packageVersion = /^version = "[^"]+"/mu;
if (!packageVersion.test(manifest)) throw new Error("Cargo package version is missing.");
await writeFile(manifestPath, manifest.replace(packageVersion, `version = "${version}"`));
const result = spawnSync("cargo", ["update", "--offline", "--workspace"], {
  cwd: projectRoot,
  stdio: ["ignore", "ignore", "inherit"],
});
if (result.error) throw result.error;
if (result.status !== 0) throw new Error("Unable to synchronize the Cargo lockfile version.");
console.log(`App version synchronized: ${version}.`);
