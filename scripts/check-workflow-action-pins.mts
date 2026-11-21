import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import workflowActionPins from "./maintenance/workflow-action-pins.cts";

const workflowDirectory = join(import.meta.dirname, "..", ".github", "workflows");
const violations = readdirSync(workflowDirectory)
  .filter((name) => /\.ya?ml$/i.test(name))
  .flatMap((name) => {
    const workflowPath = `.github/workflows/${name}`;
    return workflowActionPins.findUnpinnedWorkflowActions(
      readFileSync(join(workflowDirectory, name), "utf8"),
      workflowPath,
    );
  });

if (violations.length > 0) {
  console.error(`Unpinned GitHub Actions:\n${violations.join("\n")}`);
  process.exitCode = 1;
} else {
  console.log("GitHub Actions use immutable commit SHAs.");
}
