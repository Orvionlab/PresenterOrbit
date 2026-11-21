import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { chmodSync, existsSync, mkdirSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../", import.meta.url));
const version = "0.7.10";
const releases: Record<string, [string, string, string]> = {
  "win32-x64": ["x86_64-pc-windows-msvc", "zip", "45eb4f87b7f86fdba8508ee7e0ef1f9e13cf8f2ffcc6a2d5858d0e8ea7dd1531"],
  "win32-arm64": ["aarch64-pc-windows-msvc", "zip", "77e807cc7e5e036b3283186efd13fc2c2501f085ef93aaf817fd1728392f29a3"],
  "darwin-x64": ["x86_64-apple-darwin", "tar.gz", "e3edbba285fa23a834aa7ceb558c9d8088cb3fbdcb48fb8c619532ed4d91cab7"],
  "darwin-arm64": [
    "aarch64-apple-darwin",
    "tar.gz",
    "342c92bc33dca60dadd725ed5e249923f049ae2e1e50b7477f4968d56695b425",
  ],
  "linux-x64": [
    "x86_64-unknown-linux-gnu",
    "tar.gz",
    "4363e4ed2a3f1eb7f4d38d2d59aed59ce43271c44c16b425e92c89a64761fbe7",
  ],
  "linux-arm64": [
    "aarch64-unknown-linux-gnu",
    "tar.gz",
    "8f1a17d3218700ffbe15e6540d936a178b2556fc801121a31082e3ba4ab9ef55",
  ],
};
const release = releases[`${process.platform}-${process.arch}`];
if (!release) throw new Error(`Hot reload tooling is unavailable for ${process.platform}/${process.arch}.`);
const tools = join(root, "target", "dev-tools", `dioxus-${version}`);
const executable = process.env.PRESENTER_ORBIT_DX ?? join(tools, process.platform === "win32" ? "dx.exe" : "dx");
if (!existsSync(executable)) {
  if (process.env.PRESENTER_ORBIT_DX) throw new Error(`Dioxus CLI not found: ${executable}`);
  const [triple, extension, checksum] = release;
  console.log(`Installing Dioxus CLI ${version} into ${tools}`);
  const response = await fetch(
    `https://github.com/DioxusLabs/dioxus/releases/download/v${version}/dx-${triple}.${extension}`,
    { signal: AbortSignal.timeout(60_000) },
  );
  if (!response.ok) throw new Error(`Dioxus download failed: HTTP ${response.status}`);
  const bytes = Buffer.from(await response.arrayBuffer());
  if (createHash("sha256").update(bytes).digest("hex") !== checksum)
    throw new Error("Dioxus archive checksum mismatch.");
  mkdirSync(tools, { recursive: true });
  const archive = join(tools, `dx.${extension}`);
  writeFileSync(archive, bytes);
  const extracted = spawnSync("tar", ["-xf", archive, "-C", tools], { stdio: "inherit" });
  if (extracted.error) throw extracted.error;
  if (extracted.status !== 0) throw new Error("Dioxus archive extraction failed.");
  if (process.platform !== "win32") chmodSync(executable, 0o755);
}
const detected = spawnSync(executable, ["--version"], { encoding: "utf8" });
if (detected.error) throw detected.error;
if (detected.status !== 0 || !detected.stdout.startsWith(`dioxus ${version} `))
  throw new Error(`Hot reload requires Dioxus CLI ${version}.`);
const args = process.argv.slice(2);
if (args[0] === "--setup") {
  if (args.length !== 1) throw new Error("--setup does not accept application arguments.");
  console.log(detected.stdout.trim());
} else {
  const data = join(root, "target", "hot-reload", "data");
  if (!args.includes("--data-dir")) {
    mkdirSync(data, { recursive: true });
    const settings = join(data, "settings.json");
    if (!existsSync(settings))
      writeFileSync(
        settings,
        JSON.stringify({
          presenterOrbitWorkspacePath: join(root, "target", "hot-reload", "workspace", "workspace.json"),
        }),
        { flag: "wx" },
      );
    args.unshift("--data-dir", data);
  }
  console.log(`Starting GPUI hot reload; app arguments: ${JSON.stringify(args)}`);
  const child = spawn(
    executable,
    [
      "serve",
      "--package",
      "orbit-app",
      "--features",
      "dev-hot-reload",
      "--hot-patch",
      "--platform",
      { win32: "windows", darwin: "macos", linux: "linux" }[process.platform]!,
      "--always-on-top",
      "false",
    ],
    {
      cwd: root,
      stdio: "inherit",
      env: {
        ...process.env,
        CARGO_TARGET_DIR: resolve(root, "target/hot-reload/build"),
        PRESENTER_ORBIT_HOT_ARGS: JSON.stringify(args),
      },
    },
  );
  child.on("error", (error) => {
    console.error(error);
    process.exitCode = 1;
  });
  child.on("exit", (code, signal) => {
    process.exitCode = code ?? (signal === "SIGINT" ? 0 : 1);
  });
  process.on("SIGINT", () => child.kill("SIGINT"));
  process.on("SIGTERM", () => child.kill("SIGTERM"));
}
