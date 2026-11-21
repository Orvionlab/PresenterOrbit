import { spawnSync } from "node:child_process";

// Cargo links the app for the integration tests. Pass that path to the GPUI harness too.
const command = ["test", "-p", "orbit-app"];
const build = spawnSync("cargo", [...command, "--no-run", "--message-format=json-render-diagnostics"], {
  encoding: "utf8",
  maxBuffer: 64 * 1024 * 1024,
  stdio: ["inherit", "pipe", "inherit"],
});
if (build.error) throw build.error;
if (build.status !== 0) process.exit(build.status ?? 1);

const artifacts = build.stdout
  .trim()
  .split("\n")
  .map((line) => JSON.parse(line));
const app = artifacts.find(
  (artifact) =>
    artifact.reason === "compiler-artifact" &&
    artifact.target.name === "orbit-app" &&
    artifact.target.kind.includes("bin") &&
    !artifact.profile.test &&
    artifact.executable,
)?.executable;
if (!app) throw new Error("Cargo did not report the orbit-app executable for end-to-end tests.");

const tests = spawnSync("cargo", [...command, "--", "--test-threads=1"], {
  stdio: "inherit",
  env: { ...process.env, PRESENTER_ORBIT_E2E_EXECUTABLE: app },
});
if (tests.error) throw tests.error;
process.exitCode = tests.status ?? 1;
