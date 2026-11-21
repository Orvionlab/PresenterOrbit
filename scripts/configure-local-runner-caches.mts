import { appendFile, mkdir, realpath } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";

if (process.env.GITHUB_ACTIONS !== "true" || process.env.RUNNER_ENVIRONMENT !== "self-hosted")
  throw new Error("Local build caches require a self-hosted GitHub Actions job.");
const { GITHUB_WORKSPACE: workspace, RUNNER_WORKSPACE: runnerWorkspace, GITHUB_ENV: environmentFile } = process.env;
if (!workspace || !runnerWorkspace || !environmentFile)
  throw new Error("The runner workspace and environment file must be provided by GitHub Actions.");
const directory = await realpath(workspace);
if (directory !== (await realpath(process.cwd())) || dirname(directory) !== (await realpath(runnerWorkspace)))
  throw new Error("Local caches must belong to the runner's checked-out repository workspace.");
const cache = join(resolve(runnerWorkspace), ".orbit-build-cache");
if (/[\r\n]/u.test(cache)) throw new Error("The build cache path must not contain line breaks.");
await mkdir(cache, { recursive: true });
await appendFile(
  environmentFile,
  [
    `NPM_CONFIG_CACHE=${join(cache, "npm")}`,
    `CARGO_HOME=${join(cache, "cargo-home")}`,
    `CARGO_TARGET_DIR=${join(cache, "cargo-target")}`,
    "",
  ].join("\n"),
);
console.log("Configured persistent repository-local npm and Cargo caches.");
