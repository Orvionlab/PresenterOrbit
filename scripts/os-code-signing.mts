import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdtemp, readFile, readdir, rmdir } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

export type SigningPlatform = "windows" | "macos";
type Environment = Record<string, string | undefined>;
type Run = (command: string, args: string[], env?: Environment) => Promise<string>;

export function macAdHocEnvironment(env: Environment): Environment {
  const buildEnv: Environment = { ...env, APPLE_SIGNING_IDENTITY: "-" };
  for (const name of [
    "APPLE_CERTIFICATE",
    "APPLE_CERTIFICATE_PASSWORD",
    "APPLE_ID",
    "APPLE_PASSWORD",
    "APPLE_TEAM_ID",
    "APPLE_API_ISSUER",
    "APPLE_API_KEY",
    "APPLE_API_KEY_PATH",
  ])
    delete buildEnv[name];
  return buildEnv;
}

function required(env: Environment, name: string) {
  const value = env[name]?.trim();
  if (!value) throw new Error(`OS-signed packaging requires ${name}.`);
  return value;
}

export function signingConfiguration(platform: SigningPlatform, env: Environment) {
  if (platform === "windows") {
    const certificateThumbprint = required(env, "PRESENTER_ORBIT_WINDOWS_CERTIFICATE_THUMBPRINT").toUpperCase();
    if (!/^[0-9A-F]{40}$/u.test(certificateThumbprint))
      throw new Error("The Windows certificate thumbprint must contain 40 hexadecimal characters.");
    const timestampUrl = required(env, "PRESENTER_ORBIT_WINDOWS_TIMESTAMP_URL");
    const url = new URL(timestampUrl);
    if (!["http:", "https:"].includes(url.protocol) || url.username || url.password || url.hash)
      throw new Error("The Windows RFC 3161 timestamp URL must be HTTP(S) without credentials or a fragment.");
    return {
      signer: certificateThumbprint,
      bundle: { windows: { certificateThumbprint, timestampUrl, digestAlgorithm: "sha256", tsp: true } },
    };
  }
  const signingIdentity = required(env, "APPLE_SIGNING_IDENTITY");
  if (!/^Developer ID Application: .+ \([A-Z0-9]{10}\)$/u.test(signingIdentity))
    throw new Error("macOS distribution requires a Developer ID Application signing identity.");
  const apiNames = ["APPLE_API_ISSUER", "APPLE_API_KEY", "APPLE_API_KEY_PATH"];
  const idNames = ["APPLE_ID", "APPLE_PASSWORD", "APPLE_TEAM_ID"];
  const api = apiNames.some((name) => env[name]?.trim());
  for (const name of api ? apiNames : idNames) required(env, name);
  if (!api && !signingIdentity.endsWith(`(${env.APPLE_TEAM_ID!.trim()})`))
    throw new Error("The Apple notarization team must match the signing identity.");
  return { signer: signingIdentity, bundle: { macOS: { signingIdentity, hardenedRuntime: true } } };
}

export function validateAuthenticode(output: string, expectedThumbprint: string) {
  const signature = JSON.parse(output) as { status?: string; signer?: string; timestamped?: boolean };
  if (signature.status !== "Valid")
    throw new Error(`Authenticode verification failed (${signature.status ?? "missing status"}).`);
  if (signature.signer?.toUpperCase() !== expectedThumbprint.toUpperCase())
    throw new Error("The Authenticode signer does not match the configured release certificate.");
  if (signature.timestamped !== true) throw new Error("The Authenticode signature has no trusted timestamp.");
}

export async function signWindowsPortable(path: string, env: Environment, run: Run = runNative) {
  const configuration = signingConfiguration("windows", env);
  const tool =
    env.TAURI_WINDOWS_SIGNTOOL_PATH?.trim() ||
    (
      await run("powershell.exe", [
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        [
          "$ErrorActionPreference = 'Stop'",
          "$root = (Get-ItemProperty -LiteralPath 'HKLM:\\SOFTWARE\\Microsoft\\Windows Kits\\Installed Roots' -Name KitsRoot10).KitsRoot10",
          "$tools = @(Get-ChildItem -LiteralPath (Join-Path $root 'bin') -Directory | Where-Object { $_.Name -match '^10\\.0\\.[0-9]+\\.0$' } | Sort-Object { [version]$_.Name } -Descending | ForEach-Object { Join-Path $_.FullName 'x64/signtool.exe' } | Where-Object { Test-Path -LiteralPath $_ })",
          "if ($tools.Count -eq 0) { throw 'Windows SDK SignTool was not found; set TAURI_WINDOWS_SIGNTOOL_PATH.' }",
          "$tools[0]",
        ].join("; "),
      ])
    ).trim();
  if (!tool) throw new Error("Windows SDK SignTool returned no executable path.");
  await run(tool, [
    "sign",
    "/fd",
    "sha256",
    "/sha1",
    configuration.signer,
    "/d",
    "Presenter Orbit",
    "/tr",
    configuration.bundle.windows!.timestampUrl,
    "/td",
    "sha256",
    path,
  ]);
  await verifyOsSignature("windows", path, configuration.signer, run);
}

export function validateMacIdentity(output: string, expectedIdentity: string) {
  const lines = output.split(/\r?\n/u);
  if (!lines.includes(`Authority=${expectedIdentity}`))
    throw new Error("The macOS signature does not contain the configured Developer ID Application authority.");
  if (!lines.includes("Identifier=com.presenterorbit.desktop"))
    throw new Error("The signed macOS bundle has an unexpected application identifier.");
  if (!lines.some((line) => /^CodeDirectory /u.test(line) && /\bruntime\b/u.test(line)))
    throw new Error("The signed macOS bundle does not enable hardened runtime.");
}

export async function verifyMacAdHocBundle(path: string, run: Run = runNative) {
  await run("codesign", ["--verify", "--deep", "--strict", "--verbose=2", path]);
  const output = await run("codesign", ["--display", "--verbose=4", path]);
  const lines = output.split(/\r?\n/u);
  if (!lines.includes("Signature=adhoc") || !lines.includes("Identifier=com.presenterorbit.desktop"))
    throw new Error("Expected an ad-hoc signed Presenter Orbit application bundle.");
  return { path, codesign: true, mode: "ad-hoc", developerId: false, notarized: false };
}

export async function verifyOsSignature(platform: SigningPlatform, path: string, signer: string, run: Run = runNative) {
  if (platform === "windows") {
    const output = await run(
      "powershell.exe",
      [
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        [
          "$ErrorActionPreference = 'Stop'",
          "Import-Module (Join-Path $PSHOME 'Modules/Microsoft.PowerShell.Security/Microsoft.PowerShell.Security.psd1') -ErrorAction Stop",
          "$signature = Get-AuthenticodeSignature -LiteralPath $env:PRESENTER_ORBIT_SIGNATURE_FILE",
          "@{ status = $signature.Status.ToString(); signer = $signature.SignerCertificate.Thumbprint; timestamped = ($null -ne $signature.TimeStamperCertificate) } | ConvertTo-Json -Compress",
        ].join("; "),
      ],
      { PRESENTER_ORBIT_SIGNATURE_FILE: path },
    );
    validateAuthenticode(output, signer);
    return { path, authenticode: true, timestamped: true };
  }
  await run("codesign", ["--verify", "--deep", "--strict", "--verbose=2", path]);
  const identity = await run("codesign", ["--display", "--verbose=4", path]);
  validateMacIdentity(identity, signer);
  await run("spctl", ["--assess", "--type", "execute", "--verbose=2", path]);
  await run("xcrun", ["stapler", "validate", path]);
  return { path, codesign: true, hardenedRuntime: true, gatekeeper: true, stapled: true };
}

export async function verifyMacDiskImage(path: string, signer: string | undefined) {
  await runNative("hdiutil", ["verify", path]);
  const mount = await mkdtemp(join(tmpdir(), "orbit-signed-dmg-"));
  let attached = false;
  try {
    await runNative("hdiutil", ["attach", path, "-readonly", "-nobrowse", "-noautoopen", "-mountpoint", mount]);
    attached = true;
    const apps = (await readdir(mount)).filter((name) => name.endsWith(".app"));
    if (apps.length !== 1 || apps[0] !== "Presenter Orbit.app")
      throw new Error("The disk image must contain exactly the Presenter Orbit application bundle.");
    const payload = signer
      ? await verifyOsSignature("macos", join(mount, apps[0]), signer)
      : await verifyMacAdHocBundle(join(mount, apps[0]));
    return { ...payload, path, sha256: await fileHash(path), diskImage: true, payloadSignature: true };
  } finally {
    if (attached) await runNative("hdiutil", ["detach", mount]);
    // Never recursively remove a mount point, including after a failed attach/detach.
    await rmdir(mount);
  }
}

async function fileHash(path: string) {
  return createHash("sha256")
    .update(await readFile(path))
    .digest("hex");
}

async function runNative(command: string, args: string[], env: Environment = {}) {
  return new Promise<string>((resolveRun, rejectRun) => {
    const child = spawn(command, args, {
      env: { ...process.env, ...env },
      shell: false,
      stdio: ["ignore", "pipe", "pipe"],
    });
    let output = "";
    child.stdout.on("data", (chunk: Buffer) => {
      output += chunk.toString();
    });
    child.stderr.on("data", (chunk: Buffer) => {
      output += chunk.toString();
    });
    const timeout = setTimeout(() => {
      child.kill();
    }, 120_000);
    child.once("error", (error) => {
      clearTimeout(timeout);
      rejectRun(error);
    });
    child.once("close", (code, signal) => {
      clearTimeout(timeout);
      if (code === 0 && !signal) resolveRun(output.trim());
      else rejectRun(new Error(`${command} signature verification failed (${signal ?? code}): ${output.trim()}`));
    });
  });
}
