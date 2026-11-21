import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { join } from "node:path";

// orbit-app and orbit-ui must have no clippy findings. orbit-host has a known backlog that may not grow
// (`orbitHostClippyWarnings` in scripts/maintenance-baseline.json, per target as clippy counts it on Windows).
const projectRoot = join(import.meta.dirname, "..");
const baseline = JSON.parse(readFileSync(join(projectRoot, "scripts", "maintenance-baseline.json"), "utf8"));
const result = spawnSync("cargo", ["clippy", "--workspace", "--all-targets", "--message-format", "short"], {
  cwd: projectRoot,
  encoding: "utf8",
  maxBuffer: 256 * 1024 * 1024,
  shell: false,
});
if (result.error) throw result.error;
const output = `${result.stdout}${result.stderr}`;
const counts = new Map<string, number>();
for (const match of output.matchAll(/^warning: `([\w-]+)` \(([^)]*)\) generated (\d+) warnings?/gm))
  counts.set(`${match[1]} (${match[2]})`, Number(match[3]));
const failures: string[] = [];
if (result.status !== 0)
  failures.push(`cargo clippy failed (exit ${result.status}):\n${output.split("\n").slice(-30).join("\n")}`);
for (const [target, count] of counts) {
  const crate = target.split(" ")[0];
  const allowed = crate === "orbit-host" ? baseline.orbitHostClippyWarnings[target.slice(crate.length + 2, -1)] : 0;
  if (allowed === undefined) failures.push(`${target} has ${count} clippy warnings and no baseline.`);
  else if (count > allowed) failures.push(`${target} has ${count} clippy warnings, limit ${allowed}.`);
  else if (count < allowed)
    failures.push(`${target} fell to ${count} clippy warnings; lower its baseline from ${allowed}.`);
}
for (const [key, allowed] of Object.entries(baseline.orbitHostClippyWarnings))
  if (!counts.has(`orbit-host (${key})`))
    failures.push(`orbit-host (${key}) reported no warnings; remove its baseline of ${allowed}.`);

if (failures.length > 0) {
  console.error(failures.join("\n"));
  process.exitCode = 1;
} else {
  console.log(
    `Clippy OK (orbit-app and orbit-ui clean; orbit-host ${[...counts.values()].join(" / ")} within its baseline).`,
  );
}
