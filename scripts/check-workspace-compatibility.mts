import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";

const projectRoot = join(import.meta.dirname, "..");
const constantsPath = join(projectRoot, "crates", "orbit-host", "src", "workspace.rs");
const constantsSource = readFileSync(constantsPath, "utf8");
const versionMatch = constantsSource.match(/const WORKSPACE_VERSION:\s*u64\s*=\s*(\d+)/);

if (!versionMatch) {
  throw new Error("Could not read WORKSPACE_VERSION from crates/orbit-host/src/workspace.rs.");
}

const workspaceVersion = Number(versionMatch[1]);
const releaseCheck = process.argv.includes("--release");

if (workspaceVersion > 1) {
  const requiredPaths = [
    "crates/orbit-host/src/workspace_migrations.rs",
    ...Array.from(
      { length: workspaceVersion - 1 },
      (_, index) => `tests/fixtures/workspaces/v${index + 1}/workspace.json`,
    ),
  ];
  const missingPaths = requiredPaths.filter((relativePath) => !existsSync(join(projectRoot, relativePath)));

  if (missingPaths.length > 0) {
    console.error(
      `Workspace version ${workspaceVersion} requires migration implementation, tests, and old-version fixtures:\n${missingPaths.join("\n")}`,
    );
    process.exit(1);
  }

  const migrationTests = readFileSync(
    join(projectRoot, "crates", "orbit-host", "src", "workspace_migrations.rs"),
    "utf8",
  );
  const requiredScenarios = [
    "backs_up_a_workspace_before_migration",
    "migrates_every_supported_workspace_fixture",
    "repeated_migration_loads_are_idempotent",
  ];
  const missingScenarios = requiredScenarios.filter((scenario) => !migrationTests.includes(`fn ${scenario}(`));

  if (missingScenarios.length > 0) {
    console.error(`Workspace migration tests are missing required scenarios: ${missingScenarios.join(", ")}.`);
    process.exit(1);
  }
}

if (releaseCheck) {
  const releaseFixture = `tests/fixtures/workspaces/v${workspaceVersion}/workspace.json`;
  const missingReleaseEvidence = [releaseFixture].filter(
    (relativePath) => !existsSync(join(projectRoot, relativePath)),
  );

  if (missingReleaseEvidence.length > 0) {
    console.error(`Workspace release evidence is missing:\n${missingReleaseEvidence.join("\n")}`);
    process.exit(1);
  }

  const packageJson = JSON.parse(readFileSync(join(projectRoot, "package.json"), "utf8"));
  const releaseTag = process.env.GITHUB_REF_NAME;
  if (releaseTag?.startsWith("v") && releaseTag !== `v${packageJson.version}`) {
    console.error(`Release tag ${releaseTag} does not match package version v${packageJson.version}.`);
    process.exit(1);
  }
}

const evidence = workspaceVersion > 1 ? " with migration evidence" : "";
const releaseEvidence = releaseCheck ? " and release fixture" : "";
console.log(`Workspace compatibility OK (version ${workspaceVersion}${evidence}${releaseEvidence}).`);
