import { readdir, readFile } from "node:fs/promises";
import toolchainConfig from "./maintenance/toolchain-config.cts";

const { findRuntimeMismatches } = toolchainConfig;

const expectedNode = "26.5.1";
const expectedNpm = "12.0.2";
const packageJson = JSON.parse(await readFile("package.json", "utf8")) as {
  engines?: { node?: string; npm?: string };
  packageManager?: string;
};

const checks: Array<readonly [string, string | undefined, string]> = [
  ["package.json engines.node", packageJson.engines?.node, expectedNode],
  ["package.json engines.npm", packageJson.engines?.npm, expectedNpm],
  ["package.json packageManager", packageJson.packageManager, `npm@${expectedNpm}`],
  [".node-version", (await readFile(".node-version", "utf8")).trim(), expectedNode],
  [".nvmrc", (await readFile(".nvmrc", "utf8")).trim(), expectedNode],
];
checks.push(
  ...findRuntimeMismatches(
    {
      nodeVersion: process.versions.node,
      npmUserAgent: process.env.npm_config_user_agent,
    },
    expectedNode,
    expectedNpm,
  ),
);

const workflowDirectory = ".github/workflows";
const workflowPaths = (await readdir(workflowDirectory))
  .filter((fileName) => /\.ya?ml$/i.test(fileName))
  .map((fileName) => `${workflowDirectory}/${fileName}`);
for (const workflowPath of workflowPaths) {
  const workflow = await readFile(workflowPath, "utf8");
  checks.push(
    [`${workflowPath} Node.js`, workflow.includes(`node-version: ${expectedNode}`) ? expectedNode : "", expectedNode],
    [`${workflowPath} npm`, workflow.includes(`npm@${expectedNpm}`) ? expectedNpm : "", expectedNpm],
  );
}

const mismatches = checks.filter(([, actual, expected]) => actual !== expected);
if (mismatches.length > 0) {
  for (const [location, actual, expected] of mismatches) {
    console.error(`${location}: expected ${expected}, found ${actual || "no matching value"}`);
  }
  process.exitCode = 1;
} else {
  console.log(`Toolchain configuration OK (Node.js ${expectedNode}, npm ${expectedNpm}).`);
}
