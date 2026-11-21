// Packages the GPUI application (crates/orbit-app).
//   Windows (verified): NSIS installer + portable ZIP.
//   macOS (written, untested): .app + .dmg.   Linux (written, untested): AppImage + .deb.
// Usage: node scripts/package-gpui.mts --platform windows|macos|linux [--x64|--arm64] [--updater] [--os-signed]
//          [--out <dir>] [--skip-build] [--makensis <path>]
// Without signing secrets in the environment the artifacts are unsigned, and the script says so.
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { chmod, cp, mkdir, readdir, readFile, rm, stat, symlink, writeFile } from "node:fs/promises";
import { existsSync } from "node:fs";
import { basename, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { packageBuildDirectory } from "./package-paths.mts";
import { signUpdaterArtifact } from "./updater-signing.mts";
import { packageLinux, packageMacos } from "./package-gpui-unix.mts";
import { preparePackageFfmpeg, verifyBundledFfmpeg } from "./package-ffmpeg.mts";
import {
  macAdHocEnvironment,
  signWindowsPortable,
  signingConfiguration,
  verifyMacAdHocBundle,
  verifyOsSignature,
} from "./os-code-signing.mts";

type Platform = "windows" | "macos" | "linux";
type Architecture = "x64" | "arm64";

const projectRoot = resolve(fileURLToPath(new URL("..", import.meta.url)));
const packaging = join(projectRoot, "crates", "orbit-app", "packaging");
const args = process.argv.slice(2);
const option = (name: string) => (args.includes(name) ? args[args.indexOf(name) + 1] : undefined);
const platform = option("--platform") as Platform | undefined;
const architecture: Architecture = args.includes("--arm64")
  ? "arm64"
  : args.includes("--x64")
    ? "x64"
    : process.arch === "arm64"
      ? "arm64"
      : "x64";
const updater = args.includes("--updater");
const skipBuild = args.includes("--skip-build");
const outputRoot = resolve(option("--out") ?? join(projectRoot, "dist", "gpui"));
const requireOsSigning = args.includes("--os-signed") || process.env.PRESENTER_ORBIT_REQUIRE_OS_SIGNING === "true";

if (!platform || !["windows", "macos", "linux"].includes(platform))
  throw new Error("Usage: package-gpui.mts --platform windows|macos|linux [--x64|--arm64] [--updater] [--os-signed]");
const hostPlatform = { win32: "windows", darwin: "macos", linux: "linux" }[process.platform];
if (hostPlatform !== platform) throw new Error(`A ${platform} package must be built on a ${platform} machine.`);
if (requireOsSigning && platform === "linux")
  throw new Error("OS code signing is supported for Windows and macOS only.");

const cargoToml = await readFile(join(projectRoot, "Cargo.toml"), "utf8");
const version = /\[workspace\.package\][^[]*?version\s*=\s*"([^"]+)"/u.exec(cargoToml)?.[1];
if (!version) throw new Error("The workspace version is missing from Cargo.toml.");
const packageVersion = JSON.parse(await readFile(join(projectRoot, "package.json"), "utf8")).version;
if (packageVersion !== version)
  throw new Error(`package.json (${packageVersion}) and Cargo.toml (${version}) versions differ.`);

const updateBaseUrl = process.env.PRESENTER_ORBIT_UPDATE_BASE_URL?.replace(/\/$/u, "");
const updatePublicKey = process.env.PRESENTER_ORBIT_UPDATE_PUBLIC_KEY?.trim();
if (updater) {
  if (!updateBaseUrl || !updatePublicKey || !process.env.TAURI_SIGNING_PRIVATE_KEY)
    throw new Error(
      "Updater builds require PRESENTER_ORBIT_UPDATE_BASE_URL, PRESENTER_ORBIT_UPDATE_PUBLIC_KEY and TAURI_SIGNING_PRIVATE_KEY.",
    );
  const url = new URL(updateBaseUrl);
  if (url.protocol !== "https:" || url.username || url.password || url.search || url.hash)
    throw new Error("The update base URL must be an HTTPS URL without credentials, query or fragment.");
}
const osSigning = requireOsSigning ? signingConfiguration(platform as "windows" | "macos", process.env) : undefined;

const productName = "Presenter Orbit";
const rustTarget =
  platform === "windows"
    ? architecture === "arm64"
      ? "aarch64-pc-windows-msvc"
      : "x86_64-pc-windows-msvc"
    : platform === "macos"
      ? architecture === "arm64"
        ? "aarch64-apple-darwin"
        : "x86_64-apple-darwin"
      : undefined;
const nativeArch = process.arch === architecture;
const releaseDirectory = packageBuildDirectory(
  projectRoot,
  nativeArch ? undefined : rustTarget,
  process.env.CARGO_TARGET_DIR,
);
const exeName = platform === "windows" ? "orbit-app.exe" : "orbit-app";
const staging = join(outputRoot, "stage");
const notices = join(outputRoot, "THIRD-PARTY-NOTICES.txt");
const report: Record<string, unknown> = {
  platform,
  architecture,
  version,
  osSigned: Boolean(osSigning),
  updater,
  artifacts: [] as unknown[],
};

await rm(outputRoot, { recursive: true, force: true });
await mkdir(outputRoot, { recursive: true });
if (!skipBuild) await cargoBuild();
const binary = join(releaseDirectory, exeName);
if (!(await stat(binary).catch(() => undefined))?.isFile())
  throw new Error(`The release executable is missing: ${binary}`);
await run(process.execPath, [
  join(projectRoot, "scripts", "generate-rust-notices.mts"),
  "--out",
  notices,
  ...(rustTarget ? ["--target", rustTarget] : []),
]);

const unixContext = {
  projectRoot,
  packaging,
  version,
  architecture,
  updater,
  osSigning,
  outputRoot,
  staging,
  productName,
  binary,
  stageApplication,
  record,
  run,
  writeUpdateRelease,
};
if (platform === "windows") await packageWindows();
else if (platform === "macos") await packageMacos(unixContext);
else await packageLinux(unixContext);

await writeFile(
  join(outputRoot, `package-report-${platform}-${architecture}.json`),
  `${JSON.stringify(report, null, 2)}\n`,
);
console.log(
  `GPUI ${platform} package complete (${architecture}, ${version}). ${osSigning ? "OS code signing applied." : "UNSIGNED: no OS code signing was requested."} ${updater ? "Updater artifacts written." : "No updater artifacts (--updater not given)."}`,
);

async function cargoBuild() {
  const env = { ...process.env };
  // The feed and key are built into the executable only for an updater build.
  if (!updater) {
    delete env.PRESENTER_ORBIT_UPDATE_BASE_URL;
    delete env.PRESENTER_ORBIT_UPDATE_PUBLIC_KEY;
  }
  await run(
    "cargo",
    ["build", "--release", "-p", "orbit-app", ...(nativeArch || !rustTarget ? [] : ["--target", rustTarget])],
    projectRoot,
    platform === "macos" && !osSigning ? macAdHocEnvironment(env) : env,
  );
}

async function stageApplication(
  root: string,
  layout: { resources: string; ffmpegDirectory?: string; ffmpegSources?: string },
) {
  await mkdir(root, { recursive: true });
  await mkdir(layout.resources, { recursive: true });
  await cp(join(projectRoot, "LICENSE"), join(layout.resources, "LICENSE"));
  await cp(notices, join(layout.resources, "THIRD-PARTY-NOTICES.txt"));
  await cp(join(projectRoot, "resources"), join(layout.resources, "resources"), { recursive: true });
  if (layout.ffmpegDirectory) {
    const ffmpeg = await preparePackageFfmpeg(projectRoot, platform as "windows" | "macos", architecture, rustTarget!);
    const extension = platform === "windows" ? ".exe" : "";
    for (const name of ["ffmpeg", "ffprobe"])
      await cp(join(ffmpeg.output, `${name}${extension}`), join(layout.ffmpegDirectory, `${name}${extension}`));
    await cp(ffmpeg.source, layout.ffmpegSources!, { recursive: true });
    await verifyBundledFfmpeg(
      join(layout.ffmpegDirectory, `ffmpeg${extension}`),
      platform as "windows" | "macos",
      architecture,
      layout.ffmpegSources!,
      { execute: nativeArch },
    );
  }
}

// ---------------------------------------------------------------- Windows

async function packageWindows() {
  const app = join(staging, "app");
  await stageApplication(app, { resources: app, ffmpegDirectory: app, ffmpegSources: join(app, "ffmpeg-source") });
  await cp(binary, join(app, `${productName}.exe`));
  if (osSigning) await signWindowsPortable(join(app, `${productName}.exe`), process.env);

  const portableName = `${productName}_${version}_${architecture}-portable`;
  const portable = join(staging, portableName);
  await cp(app, portable, { recursive: true });
  await mkdir(join(portable, "data"));
  await writeFile(
    join(portable, "data", "README.txt"),
    "You can choose this folder for a Presenter Orbit workspace. Application settings are stored in the current Windows user profile.\n",
  );
  const zip = join(outputRoot, `${portableName}.zip`);
  await run(join(process.env.SystemRoot ?? "C:\\Windows", "System32", "tar.exe"), [
    "-a",
    "-c",
    "-f",
    zip,
    "-C",
    portable,
    ".",
  ]);
  await record(zip);

  const lists = await writeNsisLists(app);
  const setup = join(outputRoot, `${productName}_${version}_${architecture}-setup.exe`);
  const makensis = await findMakensis();
  await run(makensis, [
    "/V2",
    `/DVERSION=${version}`,
    `/DVERSIONNUM=${version.replace(/-.*$/u, "")}.0`,
    `/DARCH=${architecture}`,
    `/DOUTFILE=${setup}`,
    `/DICON=${join(packaging, "icons", "icon.ico")}`,
    `/DLICENSEFILE=${join(projectRoot, "LICENSE")}`,
    `/DFILES_INSTALL=${lists.install}`,
    `/DFILES_UNINSTALL=${lists.uninstall}`,
    join(packaging, "windows", "installer.nsi"),
  ]);
  if (osSigning) await signWindowsPortable(setup, process.env);
  await record(setup);
  if (osSigning) {
    for (const path of [join(app, `${productName}.exe`), setup])
      await verifyOsSignature("windows", path, osSigning.signer);
  }
  if (updater) await writeUpdateRelease(setup, `${basename(setup)}`);
}

/** The install and uninstall file lists the NSIS script includes, one SetOutPath per folder. */
async function writeNsisLists(app: string) {
  const files: string[] = [];
  const folders = new Set<string>();
  const walk = async (directory: string) => {
    for (const entry of await readdir(directory, { withFileTypes: true })) {
      const path = join(directory, entry.name);
      if (entry.isDirectory()) {
        folders.add(relative(app, path));
        await walk(path);
      } else files.push(relative(app, path));
    }
  };
  await walk(app);
  const escape = (value: string) => value.replaceAll("$", "$$$$");
  const target = (folder: string) => `"$INSTDIR${folder ? `\\${escape(folder)}` : ""}"`;
  const folderOf = (path: string) => (path.includes("\\") ? path.slice(0, path.lastIndexOf("\\")) : "");
  const install: string[] = [];
  let current: string | undefined;
  for (const file of files.sort((a, b) => folderOf(a).localeCompare(folderOf(b)) || a.localeCompare(b))) {
    const folder = folderOf(file);
    if (folder !== current) {
      install.push(`SetOutPath ${target(folder)}`);
      current = folder;
    }
    install.push(`File "${escape(join(app, file))}"`);
  }
  install.push(`SetOutPath "$INSTDIR"`);
  const uninstall = [
    ...files.map((file) => `Delete ${target(file)}`),
    ...[...folders].sort((a, b) => b.length - a.length).map((folder) => `RMDir ${target(folder)}`),
  ];
  const result = { install: join(staging, "files-install.nsh"), uninstall: join(staging, "files-uninstall.nsh") };
  await writeFile(result.install, `${install.join("\r\n")}\r\n`);
  await writeFile(result.uninstall, `${uninstall.join("\r\n")}\r\n`);
  return result;
}

async function findMakensis() {
  const candidates = [
    option("--makensis"),
    process.env.PRESENTER_ORBIT_MAKENSIS,
    ...(process.env.PATH ?? "")
      .split(process.platform === "win32" ? ";" : ":")
      .map((directory) => join(directory, "makensis.exe")),
    join(process.env["ProgramFiles(x86)"] ?? "C:\\Program Files (x86)", "NSIS", "makensis.exe"),
    join(process.env.ProgramFiles ?? "C:\\Program Files", "NSIS", "makensis.exe"),
  ].filter((candidate): candidate is string => Boolean(candidate));
  const found = candidates.find((candidate) => existsSync(candidate));
  if (!found)
    throw new Error(
      "makensis.exe was not found. Install NSIS (choco install nsis or winget install NSIS.NSIS), put it on PATH, or pass --makensis / PRESENTER_ORBIT_MAKENSIS.",
    );
  return found;
}

// ---------------------------------------------------------------- Updater artifacts

/** Signs `artifact` with the updater key (Tauri-compatible minisign) and writes `updates/<platform>-<arch>/latest.json`. */
async function writeUpdateRelease(artifact: string, fileName: string) {
  signUpdaterArtifact(artifact);
  const signature = (await readFile(`${artifact}.sig`, "utf8")).trim();
  if (!signature) throw new Error("The updater signature is empty.");
  const folder = `${platform}-${architecture}`;
  const versionFolder = join(outputRoot, "updates", folder, version);
  await mkdir(versionFolder, { recursive: true });
  await cp(artifact, join(versionFolder, fileName));
  await cp(`${artifact}.sig`, join(versionFolder, `${fileName}.sig`));
  const os = platform === "macos" ? "darwin" : platform;
  const arch = architecture === "arm64" ? "aarch64" : "x86_64";
  const manifest = {
    version,
    notes: process.env.PRESENTER_ORBIT_UPDATE_NOTES ?? "",
    pub_date: new Date().toISOString(),
    platforms: {
      [`${os}-${arch}`]: {
        signature,
        url: `${updateBaseUrl}/${folder}/${encodeURIComponent(version)}/${encodeURIComponent(fileName)}`,
      },
    },
  };
  await writeFile(join(outputRoot, "updates", folder, "latest.json"), `${JSON.stringify(manifest, null, 2)}\n`);
}

// ---------------------------------------------------------------- Helpers

async function record(path: string) {
  (report.artifacts as unknown[]).push({
    file: relative(outputRoot, path),
    sha256: createHash("sha256")
      .update(await readFile(path))
      .digest("hex"),
  });
}

/** Runs a command with inherited output. */
async function run(
  command: string,
  commandArgs: string[],
  cwd: string = projectRoot,
  env: NodeJS.ProcessEnv = process.env,
) {
  await new Promise<void>((resolveRun, rejectRun) => {
    const child = spawn(command, commandArgs, {
      cwd,
      env,
      stdio: "inherit",
      shell: false,
    });
    child.once("error", rejectRun);
    child.once("exit", (code) =>
      code === 0 ? resolveRun() : rejectRun(new Error(`${basename(command)} failed with exit code ${code}.`)),
    );
  });
}
