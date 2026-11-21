import { readFileSync } from "node:fs";
import { join } from "node:path";
import { findLocaleProblems } from "./maintenance/i18n-rules.mts";

const projectRoot = join(import.meta.dirname, "..");
const baseline = JSON.parse(readFileSync(join(projectRoot, "scripts", "maintenance-baseline.json"), "utf8"));
const read = (locale: string) =>
  JSON.parse(readFileSync(join(projectRoot, "crates", "orbit-ui", "locales", `${locale}.arb`), "utf8"));
const problems = findLocaleProblems(read("en"), read("nl"), baseline.i18nMissingMetadataCount);

if (problems.length > 0) {
  console.error(problems.join("\n"));
  process.exitCode = 1;
} else {
  console.log("Locale files OK (keys, placeholders, Dutch terminology, metadata).");
}
