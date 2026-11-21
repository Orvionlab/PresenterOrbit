import { createHash } from "node:crypto";
import { spawn } from "node:child_process";
import { cp, mkdir, mkdtemp, readFile, readdir, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, isAbsolute, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

type Platform = "windows" | "macos";
type Architecture = "x64" | "arm64";
export const ffmpegSources = {
  ffmpeg: {
    version: "8.1.1",
    url: "https://ffmpeg.org/releases/ffmpeg-8.1.1.tar.xz",
    sha256: "b6863adde98898f42602017462871b5f6333e65aec803fdd7a6308639c52edf3",
  },
  x264: {
    commit: "b35605ace3ddf7c1a5d67a2eb553f034aef41d55",
    url: "https://github.com/mirror/x264.git",
    upstream: "https://code.videolan.org/videolan/x264.git",
    sha256: "e92edc35efe3f8c3effb0c04854de0f8428cc09a218b2062ec3fe44f8ad2aa4e",
  },
  zlib: {
    version: "1.3.1",
    url: "https://zlib.net/fossils/zlib-1.3.1.tar.gz",
    sha256: "9a93b2b7dfdac77ceba5a558a580e74667dd6fede4585b91eefb60f03b72df23",
  },
} as const;

async function run(command: string, args: string[], cwd?: string) {
  return await new Promise<string>((accept, reject) => {
    const child = spawn(command, args, { cwd, shell: false, windowsHide: true, stdio: ["ignore", "pipe", "inherit"] });
    let output = "";
    child.stdout.on("data", (chunk) => {
      output += String(chunk);
    });
    child.once("error", reject);
    child.once("exit", (code) =>
      code === 0 ? accept(output) : reject(new Error(`${command} exited with ${code}.\n${output.slice(-16000)}`)),
    );
  });
}

export async function verifyFfmpegSource(sourceDirectory: string) {
  for (const [name, expected] of [
    ["ffmpeg-8.1.1.tar.xz", ffmpegSources.ffmpeg.sha256],
    ["x264.tar", ffmpegSources.x264.sha256],
    ["zlib-1.3.1.tar.gz", ffmpegSources.zlib.sha256],
  ] as const) {
    const actual = createHash("sha256")
      .update(await readFile(join(sourceDirectory, name)))
      .digest("hex");
    if (actual !== expected) throw new Error(`Bundled ${name} source checksum does not match the pinned source.`);
  }
}

export function verifyBinaryArchitecture(binary: Buffer, platform: Platform, architecture: Architecture) {
  if (platform === "windows") {
    if (binary.toString("ascii", 0, 2) !== "MZ") throw new Error("FFmpeg must be a PE executable.");
    const header = binary.readUInt32LE(0x3c);
    if (
      binary.readUInt32LE(header) !== 0x4550 ||
      binary.readUInt16LE(header + 4) !== (architecture === "x64" ? 0x8664 : 0xaa64)
    )
      throw new Error("FFmpeg PE architecture does not match the package architecture.");
  } else if (
    binary.readUInt32LE(0) !== 0xfeedfacf ||
    binary.readUInt32LE(4) !== (architecture === "x64" ? 0x01000007 : 0x0100000c)
  ) {
    throw new Error("FFmpeg Mach-O architecture does not match the package architecture.");
  }
}

// Windows imports are read directly so verifying a package needs no compiler or SDK.
export function windowsImports(binary: Buffer) {
  const pe = binary.readUInt32LE(0x3c);
  const sections = binary.readUInt16LE(pe + 6);
  const optional = pe + 24;
  if (binary.readUInt16LE(optional) !== 0x20b) throw new Error("Expected a 64-bit FFmpeg PE executable.");
  const sectionOffset = optional + binary.readUInt16LE(pe + 20);
  const offset = (rva: number) => {
    for (let i = 0; i < sections; i++) {
      const row = sectionOffset + i * 40;
      const start = binary.readUInt32LE(row + 12);
      if (rva >= start && rva < start + Math.max(binary.readUInt32LE(row + 8), binary.readUInt32LE(row + 16)))
        return rva - start + binary.readUInt32LE(row + 20);
    }
    throw new Error("Invalid FFmpeg PE import address.");
  };
  const importRva = binary.readUInt32LE(optional + 112 + 8);
  if (!importRva) return [];
  const names: string[] = [];
  for (let row = offset(importRva); binary.readUInt32LE(row + 12); row += 20) {
    const name = offset(binary.readUInt32LE(row + 12));
    names.push(binary.toString("ascii", name, binary.indexOf(0, name)).toLowerCase());
  }
  return names;
}

export async function verifyBundledFfmpeg(
  executablePath: string,
  platform: Platform,
  architecture: Architecture,
  sourceDirectory: string,
  options: { execute?: boolean } = {},
) {
  await verifyFfmpegSource(sourceDirectory);
  const manifest = JSON.parse(await readFile(join(sourceDirectory, "manifest.json"), "utf8"));
  if (
    manifest.platform !== platform ||
    manifest.architecture !== architecture ||
    JSON.stringify(manifest.sources) !== JSON.stringify(ffmpegSources)
  )
    throw new Error("FFmpeg source manifest does not match the package and pinned source versions.");
  const recipe = await readFile(join(sourceDirectory, "build-package-ffmpeg.sh"));
  if (createHash("sha256").update(recipe).digest("hex") !== manifest.recipeSha256)
    throw new Error("FFmpeg build recipe checksum is invalid.");
  for (const license of ["FFmpeg-COPYING.GPLv2", "FFmpeg-LICENSE.md", "x264-COPYING", "zlib-LICENSE", "NOTICE.txt"]) {
    if (!(await readFile(join(sourceDirectory, license), "utf8")).trim())
      throw new Error(`Missing FFmpeg license: ${license}.`);
  }
  const runtimeLicense = await readFile(join(sourceDirectory, "toolchain-licenses", "LLVM-RUNTIME-LICENSE.txt"));
  if (
    createHash("sha256").update(runtimeLicense).digest("hex") !==
    "1a8f1058753f1ba890de984e48f0242a3a5c29a6a8f2ed9fd813f36985387e8d"
  )
    throw new Error("The bundled LLVM runtime license does not match the upstream license.");
  if (platform === "windows" && (await readdir(join(sourceDirectory, "toolchain-licenses"))).length < 3)
    throw new Error("The bundled Windows compiler/runtime license notices are incomplete.");
  if (!(await readFile(join(sourceDirectory, "compiler-version.txt"), "utf8")).trim())
    throw new Error("The FFmpeg compiler version was not recorded.");
  const dependencies: Record<string, string[]> = {};
  for (const name of ["ffmpeg", "ffprobe"]) {
    const path = join(dirname(executablePath), `${name}${platform === "windows" ? ".exe" : ""}`);
    const binary = await readFile(path);
    verifyBinaryArchitecture(binary, platform, architecture);
    for (const option of [
      "--disable-autodetect",
      "--enable-gpl",
      "--enable-libx264",
      "--enable-zlib",
      "--enable-static",
      "--disable-shared",
      "--disable-nonfree",
      "--disable-version3",
    ])
      if (!binary.includes(Buffer.from(option))) throw new Error(`${name} is missing required build option ${option}.`);
    if (platform === "windows") {
      dependencies[name] = windowsImports(binary);
      const allowed =
        /^(?:advapi32|avicap32|avrt|bcrypt|crypt32|d3d11|d3d12|dbghelp|dxgi|dxva2|gdi32|kernel32|mfplat|mfuuid|msvcrt|ncrypt|ntdll|ole32|oleaut32|psapi|secur32|shell32|shlwapi|ucrtbase|user32|vfw32|winmm|ws2_32|wsock32)\.dll$/u;
      for (const dependency of dependencies[name]!)
        if (!allowed.test(dependency) && !/^api-ms-win-[a-z0-9-]+\.dll$/u.test(dependency))
          throw new Error(`FFmpeg has an unbundled/non-system DLL dependency: ${dependency}.`);
    } else {
      dependencies[name] = (await run("otool", ["-L", path]))
        .split("\n")
        .slice(1)
        .map((line) => line.trim().split(" (")[0]!)
        .filter(Boolean);
      for (const dependency of dependencies[name]!)
        if (!dependency.startsWith("/System/Library/") && !dependency.startsWith("/usr/lib/"))
          throw new Error(`FFmpeg has an unbundled/non-system dylib dependency: ${dependency}.`);
    }
    if (options.execute !== false) {
      const version = await run(path, ["-version"]);
      if (
        !version.startsWith(`${name} version ${ffmpegSources.ffmpeg.version}`) ||
        !version.includes("--enable-libx264") ||
        !version.includes("--enable-gpl") ||
        version.includes("--enable-nonfree") ||
        version.includes("--enable-version3")
      )
        throw new Error(`${name} version or license/build configuration does not match the bundled build contract.`);
    }
  }
  return {
    platform,
    architecture,
    version: ffmpegSources.ffmpeg.version,
    sources: ffmpegSources,
    dependencies,
    executed: options.execute !== false,
  };
}

export async function preparePackageFfmpeg(
  projectRoot: string,
  platform: Platform,
  architecture: Architecture,
  target: string,
) {
  if (process.env.PRESENTER_ORBIT_FFMPEG_PREPARED_ROOT && !isAbsolute(process.env.PRESENTER_ORBIT_FFMPEG_PREPARED_ROOT))
    throw new Error("PRESENTER_ORBIT_FFMPEG_PREPARED_ROOT must be an absolute path.");
  const output = process.env.PRESENTER_ORBIT_FFMPEG_PREPARED_ROOT
    ? resolve(process.env.PRESENTER_ORBIT_FFMPEG_PREPARED_ROOT)
    : join(projectRoot, "artifacts", "package-ffmpeg", `${platform}-${architecture}`);
  const source = join(output, "ffmpeg-source");
  const extension = platform === "windows" ? ".exe" : "";
  const executable = join(output, `ffmpeg${extension}`);
  const recipe = join(projectRoot, "scripts", "build-package-ffmpeg.sh");
  const recipeContents = (await readFile(recipe, "utf8")).replace(/\r\n/gu, "\n");
  const recipeSha256 = createHash("sha256").update(recipeContents).digest("hex");
  const execute =
    (platform === "windows" ? process.platform === "win32" : process.platform === "darwin") &&
    process.arch === architecture;
  const existing = await stat(join(source, "manifest.json")).catch(() => undefined);
  if (existing) {
    const manifest = JSON.parse(await readFile(join(source, "manifest.json"), "utf8"));
    if (manifest.recipeSha256 !== recipeSha256)
      throw new Error(`FFmpeg build recipe changed; remove only the generated cache ${output} and rebuild.`);
    await verifyBundledFfmpeg(executable, platform, architecture, source, { execute });
  } else {
    await mkdir(source, { recursive: true });
    await mkdir(join(source, "toolchain-licenses"), { recursive: true });
    await writeFile(
      join(source, "toolchain-licenses", "LLVM-RUNTIME-LICENSE.txt"),
      (await readFile(join(projectRoot, "docs", "licenses", "LLVM-RUNTIME-LICENSE.txt"), "utf8")).replace(
        /\r\n/gu,
        "\n",
      ),
    );
    const response = await fetch(ffmpegSources.ffmpeg.url, { signal: AbortSignal.timeout(120000) });
    if (!response.ok) throw new Error(`FFmpeg source download failed: HTTP ${response.status}.`);
    await writeFile(join(source, "ffmpeg-8.1.1.tar.xz"), Buffer.from(await response.arrayBuffer()));
    const zlibResponse = await fetch(ffmpegSources.zlib.url, { signal: AbortSignal.timeout(120000) });
    if (!zlibResponse.ok) throw new Error(`zlib source download failed: HTTP ${zlibResponse.status}.`);
    await writeFile(join(source, "zlib-1.3.1.tar.gz"), Buffer.from(await zlibResponse.arrayBuffer()));
    const checkout = await mkdtemp(join(tmpdir(), "orbit-x264-source-"));
    try {
      await run("git", ["init", "--quiet", checkout]);
      await run("git", [
        "-C",
        checkout,
        "fetch",
        "--quiet",
        "--depth",
        "1",
        ffmpegSources.x264.url,
        ffmpegSources.x264.commit,
      ]);
      await run("git", [
        "-C",
        checkout,
        "-c",
        "core.autocrlf=false",
        "archive",
        "--format=tar",
        `--output=${join(source, "x264.tar")}`,
        ffmpegSources.x264.commit,
      ]);
    } finally {
      await rm(checkout, { recursive: true, force: true });
    }
    await verifyFfmpegSource(source);
    await writeFile(join(source, "build-package-ffmpeg.sh"), recipeContents);
    if (platform === "windows" && process.platform === "win32") {
      const distribution = process.env.PRESENTER_ORBIT_FFMPEG_WSL_DISTRIBUTION || "Ubuntu-24.04";
      const convert = async (path: string) =>
        (await run("wsl.exe", ["-d", distribution, "--exec", "wslpath", "-u", path])).trim();
      await run("wsl.exe", [
        "-d",
        distribution,
        "--exec",
        "bash",
        await convert(join(source, "build-package-ffmpeg.sh")),
        await convert(source),
        await convert(output),
        platform,
        architecture,
      ]);
    } else await run("bash", [join(source, "build-package-ffmpeg.sh"), source, output, platform, architecture]);
    await writeFile(
      join(source, "NOTICE.txt"),
      "Presenter Orbit invokes FFmpeg and ffprobe as separate processes; it does not link their libraries. These executables include x264 and zlib and are distributed under GPL-2.0-or-later. Their complete corresponding unmodified sources, license texts and build recipe are included in this directory. You may modify and redistribute these separate components under their licenses. See manifest.json for exact upstream revisions and source checksums.\n",
    );
    await writeFile(
      join(source, "manifest.json"),
      `${JSON.stringify({ platform, architecture, license: "GPL-2.0-or-later", sources: ffmpegSources, recipeSha256 }, null, 2)}\n`,
    );
    await verifyBundledFfmpeg(executable, platform, architecture, source, { execute });
  }
  for (const name of ["ffmpeg", "ffprobe"])
    await cp(join(output, `${name}${extension}`), join(output, `${name}-${target}${extension}`));
  return {
    output,
    source,
    executable,
    externalBin: [join(output, "ffmpeg"), join(output, "ffprobe")],
    resources: { [source]: "ffmpeg-source/" },
  };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const platformIndex = process.argv.indexOf("--platform");
  const platform =
    platformIndex >= 0
      ? (process.argv[platformIndex + 1] as Platform)
      : process.platform === "win32"
        ? "windows"
        : process.platform === "darwin"
          ? "macos"
          : undefined;
  if (platform !== "windows" && platform !== "macos")
    throw new Error("Select --platform windows or --platform macos for the FFmpeg package build.");
  if (process.argv.includes("--arm64") && process.argv.includes("--x64"))
    throw new Error("Select one FFmpeg build architecture.");
  const architecture = process.argv.includes("--arm64")
    ? "arm64"
    : process.argv.includes("--x64")
      ? "x64"
      : process.arch === "arm64"
        ? "arm64"
        : "x64";
  const target =
    platform === "windows"
      ? architecture === "x64"
        ? "x86_64-pc-windows-msvc"
        : "aarch64-pc-windows-msvc"
      : architecture === "x64"
        ? "x86_64-apple-darwin"
        : "aarch64-apple-darwin";
  console.log(
    await preparePackageFfmpeg(resolve(fileURLToPath(new URL("..", import.meta.url))), platform, architecture, target),
  );
}
