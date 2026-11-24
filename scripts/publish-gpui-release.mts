import { spawnSync } from "node:child_process";
import { createHash, createPublicKey, verify } from "node:crypto";
import { readFile, readdir, rename, stat } from "node:fs/promises";
import { basename, join } from "node:path";
import { signUpdaterData } from "./updater-signing.mts";

const targets = {
  "windows-x64": "windows-x86_64",
  "macos-arm64": "darwin-aarch64",
  "macos-x64": "darwin-x86_64",
  "linux-x64": "linux-x86_64",
} as const;

const versionPattern = /^(\d+)\.(\d+)\.(\d+)(?:-([\da-z-]+(?:\.[\da-z-]+)*))?$/iu;
const { version } = JSON.parse(await readFile("package.json", "utf8")) as { version: string };
const tag = `v${version}`;
if (!versionPattern.test(version) || process.env.GITHUB_REF !== `refs/tags/${tag}`) {
  throw new Error(`Run this workflow on the version tag ${tag}.`);
}

function required(name: string) {
  const value = process.env[name]?.trim();
  if (!value) throw new Error(`Set ${name} in the GitHub release environment.`);
  return value;
}

const base = new URL(required("PRESENTER_ORBIT_UPDATE_BASE_URL"));
if (base.protocol !== "https:" || base.username || base.password || base.search || base.hash) {
  throw new Error("The updater requires a public HTTPS base URL.");
}
const baseUrl = base.href.replace(/\/$/u, "");
const prefix = base.pathname.slice(1).replace(/\/$/u, "");
const objectPrefix = prefix ? `${decodeURIComponent(prefix)}/` : "";
const migratingLegacyFeed = baseUrl === "https://presenter-update.orvionlab.com" && version === "0.0.1-alpha.1";
// Only the known private manifests can bypass the public feed's version ordering during the cutover.
const legacyManifests = {
  "windows-x64": "8c5393d02d96d4fa62d40ac2e4a5ec2e760d0453de2633c1a5452f93f2299e64",
  "macos-arm64": "6d0987913dd5e80bff6efc7af71dc5a46727a2aec8b1808680dbba35f1b06856",
  "macos-x64": "205a1502423808aa8be9ab77f7bc326926572b7958bface4bfa7223e68a1fc12",
  "linux-x64": "de11e760ee1a14c2758be2c537231264eff8fa2d2e8a14a2b6c4f13548757d04",
};

const publicText = Buffer.from(required("PRESENTER_ORBIT_UPDATE_PUBLIC_KEY"), "base64").toString("utf8");
const publicKey = Buffer.from(publicText.split(/\r?\n/u)[1] ?? "", "base64");
if (publicKey.length !== 42 || publicKey.toString("ascii", 0, 2) !== "Ed") {
  throw new Error("The update public key is not a Tauri-encoded Minisign key.");
}
const key = createPublicKey({
  key: Buffer.concat([Buffer.from("302a300506032b6570032100", "hex"), publicKey.subarray(10)]),
  type: "spki",
  format: "der",
});

function verifySignature(bytes: Buffer, encoded: string) {
  const [, signature, comment, global] = Buffer.from(encoded, "base64").toString("utf8").split(/\r?\n/u);
  const raw = Buffer.from(signature ?? "", "base64");
  if (
    raw.length !== 74 ||
    raw.toString("ascii", 0, 2) !== "ED" ||
    !raw.subarray(2, 10).equals(publicKey.subarray(2, 10)) ||
    !comment?.startsWith("trusted comment: ") ||
    !verify(null, createHash("blake2b512").update(bytes).digest(), key, raw.subarray(10)) ||
    !verify(
      null,
      Buffer.concat([raw.subarray(10), Buffer.from(comment.slice("trusted comment: ".length))]),
      key,
      Buffer.from(global ?? "", "base64"),
    )
  )
    throw new Error("Updater signature does not match the release public key.");
}

if (process.argv[2] === "--check") {
  const bytes = Buffer.from("Presenter Orbit release signing check");
  const signature = signUpdaterData(
    bytes,
    "signing-check",
    required("TAURI_SIGNING_PRIVATE_KEY"),
    process.env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD ?? "",
  );
  verifySignature(bytes, signature);
  console.log(`Validated signing configuration for ${tag}.`);
  process.exit(0);
}
const directory = process.argv[2];
if (!directory) throw new Error("Pass the directory containing the release build artifacts.");

const bucket = required("CLOUDFLARE_R2_BUCKET");
const account = required("CLOUDFLARE_ACCOUNT_ID");
if (!/^[a-f0-9]{32}$/u.test(account)) throw new Error("Invalid Cloudflare account ID.");
required("AWS_ACCESS_KEY_ID");
required("AWS_SECRET_ACCESS_KEY");
const endpoint = `https://${account}.r2.cloudflarestorage.com`;
const repo = required("GITHUB_REPOSITORY");

function command(executable: string, args: string[], allowedError?: RegExp) {
  const result = spawnSync(executable, args, { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] });
  if (result.error) throw result.error;
  if (result.status === 0) return result.stdout;
  if (allowedError?.test(result.stderr)) return null;
  throw new Error(`${executable} ${args.slice(0, 2).join(" ")} failed: ${result.stderr.trim()}`);
}

function aws(action: string, ...args: string[]) {
  return command(
    "aws",
    ["s3api", action, "--endpoint-url", endpoint, "--bucket", bucket, ...args],
    action === "head-object" ? /\((?:404|NotFound)\)/u : undefined,
  );
}

async function getManifest(url: string) {
  const response = await fetch(url, { cache: "no-store", signal: AbortSignal.timeout(30000) });
  if (response.status === 404) return null;
  if (!response.ok) throw new Error(`Could not read ${url}: HTTP ${response.status}.`);
  const bytes = Buffer.from(await response.arrayBuffer());
  const manifest = JSON.parse(bytes.toString("utf8")) as {
    version: string;
    platforms: Record<string, { signature: string; url: string }>;
  };
  return { ...manifest, sha256: createHash("sha256").update(bytes).digest("hex") };
}

function compareVersions(left: string, right: string) {
  const a = versionPattern.exec(left);
  const b = versionPattern.exec(right);
  if (!a || !b) throw new Error("Invalid version in an update manifest.");
  for (let i = 1; i <= 3; i++) {
    const difference = Number(a[i]) - Number(b[i]);
    if (difference) return difference;
  }
  if (!a[4] || !b[4]) return Number(!a[4]) - Number(!b[4]);
  const aParts = a[4].split(".");
  const bParts = b[4].split(".");
  for (let i = 0; i < Math.min(aParts.length, bParts.length); i++) {
    const x = aParts[i]!;
    const y = bParts[i]!;
    if (x === y) continue;
    const xNumber = /^\d+$/u.test(x);
    const yNumber = /^\d+$/u.test(y);
    if (xNumber && yNumber) return Number(x) - Number(y);
    if (xNumber !== yNumber) return xNumber ? -1 : 1;
    return x < y ? -1 : 1;
  }
  return aParts.length - bParts.length;
}

const assets = [];
const feeds = [];
for (const target of Object.keys(targets) as (keyof typeof targets)[]) {
  const platform = targets[target];
  const output = join(directory, `release-package-${target}`);
  const downloads = (await readdir(output)).filter((file) => /\.(?:exe|zip|dmg|AppImage|deb)$/u.test(file));
  const expected = target.startsWith("windows") ? 2 : target.startsWith("macos") ? 1 : 2;
  if (downloads.length !== expected) throw new Error(`Missing release downloads for ${target}.`);

  const manifest = join(output, "updates", target, "latest.json");
  const feed = JSON.parse(await readFile(manifest, "utf8"));
  const entry = feed.platforms?.[platform];
  if (feed.version !== version || !entry) throw new Error(`Invalid release manifest for ${target}.`);
  const fileName = decodeURIComponent(basename(new URL(entry.url).pathname));
  const versionPath = `${target}/${version}`;
  const artifact = join(output, "updates", versionPath, fileName);
  if (
    fileName !== basename(fileName) ||
    entry.url !== `${baseUrl}/${versionPath}/${encodeURIComponent(fileName)}` ||
    entry.signature !== (await readFile(`${artifact}.sig`, "utf8")).trim()
  )
    throw new Error(`Update artifact does not match the manifest for ${target}.`);
  const bytes = await readFile(artifact);
  verifySignature(bytes, entry.signature);
  const downloadName = downloads.find((name) => name.replaceAll(" ", "-") === fileName);
  if (downloadName && !bytes.equals(await readFile(join(output, downloadName)))) {
    throw new Error(`GitHub and updater downloads differ for ${target}.`);
  }

  const manifestKey = `${objectPrefix}${target}/latest.json`;
  const previousObject = aws("head-object", "--key", manifestKey);
  const manifestUrl = `${baseUrl}/${target}/latest.json`;
  const previous = await getManifest(manifestUrl);
  if (Boolean(previous) !== Boolean(previousObject)) throw new Error(`R2 and public feed disagree for ${target}.`);
  if (previous && !(migratingLegacyFeed && previous.sha256 === legacyManifests[target])) {
    const order = compareVersions(version, previous.version);
    if (order < 0 || (order === 0 && previous.platforms[platform]?.signature !== entry.signature)) {
      throw new Error(`Refusing to replace the newer or different update already served for ${target}.`);
    }
  }
  feeds.push({
    target,
    manifest,
    manifestKey,
    manifestUrl,
    previousObject,
    artifact,
    versionPath,
    fileName,
    url: entry.url,
    platform,
    signature: entry.signature,
  });

  for (const name of downloads) {
    const path = join(output, name);
    const normalized = join(output, name.replaceAll(" ", "-"));
    if (normalized !== path) await rename(path, normalized);
    assets.push(normalized);
  }
}

for (const feed of feeds) {
  for (const suffix of ["", ".sig"]) {
    const path = feed.artifact + suffix;
    const objectKey = `${objectPrefix}${feed.versionPath}/${feed.fileName}${suffix}`;
    const checksum = createHash("sha256")
      .update(await readFile(path))
      .digest("hex");
    const existing = aws("head-object", "--key", objectKey);
    if (existing) {
      if (JSON.parse(existing).Metadata?.sha256 !== checksum) {
        throw new Error(`Versioned update already exists with different contents: ${objectKey}.`);
      }
    } else {
      aws(
        "put-object",
        "--key",
        objectKey,
        "--body",
        path,
        "--if-none-match",
        "*",
        "--metadata",
        `sha256=${checksum}`,
        "--cache-control",
        "public, max-age=31536000, immutable",
      );
    }
    const response = await fetch(feed.url + suffix, { method: "HEAD", signal: AbortSignal.timeout(30000) });
    if (!response.ok || Number(response.headers.get("content-length")) !== (await stat(path)).size) {
      throw new Error(`Versioned update is unavailable through the public domain: ${feed.url}${suffix}.`);
    }
  }
}

let releaseJson = command("gh", ["release", "view", tag, "--repo", repo, "--json", "isDraft"], /release not found/iu);
if (!releaseJson) {
  command("gh", [
    "release",
    "create",
    tag,
    "--repo",
    repo,
    "--verify-tag",
    "--draft",
    "--generate-notes",
    "--title",
    `Presenter Orbit ${tag}`,
    ...(version.includes("-") ? ["--prerelease"] : []),
  ]);
  releaseJson = command("gh", ["release", "view", tag, "--repo", repo, "--json", "isDraft"]);
}
const release = JSON.parse(releaseJson!);
const existingAssets = JSON.parse(
  command("gh", ["api", `repos/${repo}/releases`, "--jq", `.[] | select(.tag_name == "${tag}") | .assets`]) ?? "[]",
);
for (const path of assets) {
  const existing = existingAssets.find((asset: { name: string }) => asset.name === basename(path));
  if (existing) {
    const digest = `sha256:${createHash("sha256")
      .update(await readFile(path))
      .digest("hex")}`;
    if (existing.digest !== digest) {
      if (!migratingLegacyFeed) throw new Error(`GitHub has a different release asset named ${basename(path)}.`);
      command("gh", ["release", "upload", tag, path, "--repo", repo, "--clobber"]);
    }
  } else {
    command("gh", ["release", "upload", tag, path, "--repo", repo]);
  }
}
if (migratingLegacyFeed) {
  // Keep the alpha version in the tag while allowing GitHub to show this download in the repository sidebar.
  command("gh", ["release", "edit", tag, "--repo", repo, "--draft=false", "--prerelease=false", "--latest"]);
} else if (release.isDraft) {
  command("gh", ["release", "edit", tag, "--repo", repo, "--draft=false"]);
}

for (const feed of feeds) {
  const precondition = feed.previousObject
    ? ["--if-match", JSON.parse(feed.previousObject).ETag]
    : ["--if-none-match", "*"];
  aws(
    "put-object",
    "--key",
    feed.manifestKey,
    "--body",
    feed.manifest,
    ...precondition,
    "--content-type",
    "application/json",
    "--cache-control",
    "no-cache",
  );
  const published = await getManifest(feed.manifestUrl);
  if (published?.version !== version || published.platforms[feed.platform]?.signature !== feed.signature) {
    throw new Error(`Public update manifest verification failed for ${feed.manifestUrl}.`);
  }
}

function removeLegacyObject(objectKey: string) {
  if (!aws("head-object", "--key", objectKey)) return;
  aws("delete-object", "--key", objectKey);
  if (aws("head-object", "--key", objectKey)) throw new Error(`Legacy object still exists: ${objectKey}.`);
  console.log(`Removed legacy update object: ${objectKey}.`);
}

if (migratingLegacyFeed) {
  for (const feed of feeds) {
    const manifestKey = `gpui/${feed.target}/latest.json`;
    const manifestUrl = `${baseUrl}/${manifestKey}`;
    const previous = await getManifest(manifestUrl);
    if (previous && previous.version !== "0.0.1-alpha.1") {
      throw new Error(`Unexpected existing GPUI migration manifest for ${feed.target}.`);
    }
    removeLegacyObject(manifestKey);
    if (await getManifest(manifestUrl)) throw new Error(`Former GPUI feed is still available: ${manifestUrl}.`);
  }

  // These exact filenames belong to the retired feeds. New public artifacts use names without spaces.
  for (const oldVersion of ["0.0.1-alpha.1", "0.0.1-alpha.2", "0.0.1-alpha.3"]) {
    const wasPublic = oldVersion === "0.0.1-alpha.1";
    const oldFiles = {
      "windows-x64": `Presenter Orbit_${oldVersion}_x64-setup.exe`,
      "macos-arm64": "Presenter Orbit.app.tar.gz",
      "macos-x64": "Presenter Orbit.app.tar.gz",
      "linux-x64": `Presenter Orbit_${oldVersion}_${wasPublic ? "x64" : "amd64"}.AppImage`,
    };
    for (const [target, name] of Object.entries(oldFiles)) {
      for (const suffix of ["", ".sig"]) {
        removeLegacyObject(`${wasPublic ? "gpui/" : ""}${target}/${oldVersion}/${name}${suffix}`);
      }
    }
  }
}
console.log(`Published ${tag} and verified all four public update manifests.`);
