// Starts a packaged GPUI application (an unpacked portable folder, or any folder with the executable) against a
// private data directory and checks that it stays up, with its bundled files next to it.
// Usage: node scripts/gpui-smoke.mts --dir <folder with "Presenter Orbit[.exe]"> [--workspace <workspace.json>] [--seconds 15]
// The data directory is a new temporary folder, so the user's settings are never read or written.
import { spawn, spawnSync } from "node:child_process";
import { existsSync, mkdtempSync, readdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const args = process.argv.slice(2);
const option = (name: string) => (args.includes(name) ? args[args.indexOf(name) + 1] : undefined);
const directory = option("--dir");
if (!directory) throw new Error("Usage: gpui-smoke.mts --dir <folder> [--workspace <workspace.json>] [--seconds 15]");
const seconds = Number(option("--seconds") ?? 15);
const windows = process.platform === "win32";
const executable = resolve(directory, windows ? "Presenter Orbit.exe" : "presenter-orbit");
const extension = windows ? ".exe" : "";

const required = [
  executable,
  join(directory, "resources", "Sample Content", "Themes", "index.json"),
  join(directory, "THIRD-PARTY-NOTICES.txt"),
  join(directory, "LICENSE"),
  ...(process.platform === "linux"
    ? []
    : [
        join(directory, `ffmpeg${extension}`),
        join(directory, `ffprobe${extension}`),
        join(directory, "ffmpeg-source"),
      ]),
];
const missing = required.filter((path) => !existsSync(path));
if (missing.length) throw new Error(`Package files are missing:\n${missing.join("\n")}`);
if (process.platform !== "linux") {
  const version = spawnSync(join(directory, `ffmpeg${extension}`), ["-version"], { encoding: "utf8" });
  if (version.status !== 0 || !version.stdout.startsWith("ffmpeg version"))
    throw new Error("The bundled ffmpeg does not run.");
}

const data = mkdtempSync(join(tmpdir(), "orbit-gpui-smoke-"));
const child = spawn(
  executable,
  ["--data-dir", data, ...(option("--workspace") ? ["--workspace", resolve(option("--workspace")!)] : [])],
  { stdio: "ignore" },
);
let exited: number | null | undefined;
child.once("exit", (code) => (exited = code));
await new Promise((done) => setTimeout(done, seconds * 1000));
const alive = exited === undefined;
if (windows && child.pid) spawnSync("taskkill", ["/F", "/T", "/PID", String(child.pid)]);
else child.kill("SIGKILL");
const files = readdirSync(data);
rmSync(data, { recursive: true, force: true });
if (!alive) throw new Error(`The application exited with code ${exited} before ${seconds} seconds.`);
console.log(`GPUI smoke passed: ${executable} ran ${seconds}s; data directory held ${files.join(", ") || "nothing"}.`);
