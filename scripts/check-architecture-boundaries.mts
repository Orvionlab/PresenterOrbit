import { readdirSync, readFileSync } from "node:fs";
import { join, relative } from "node:path";
import { findGpuiColorLiterals } from "./maintenance/gpui-color-rules.mts";

const projectRoot = join(import.meta.dirname, "..");
const violations: string[] = [];

// Colors in orbit-app come from `orbit_ui::theme`. These files only hold slide document data or native output pixels.
const gpuiColorLiteralFiles = new Set([
  "crates/orbit-app/src/canvas/catalog.rs",
  "crates/orbit-app/src/canvas/editing.rs",
  "crates/orbit-app/src/media/markers.rs", // the stored marker color default React shows in its picker
  "crates/orbit-app/src/output/meters.rs", // the program level bars mirror the React meter colors
  "crates/orbit-app/src/settings/patterns.rs", // the stored default colors of the test patterns
  "crates/orbit-app/src/settings/groups.rs", // the stored default color of a new group, `DEFAULT_COLOR`
  "crates/orbit-app/src/native_outputs/acceptance.rs",
  "crates/orbit-app/src/native_outputs/linux.rs",
  "crates/orbit-app/src/native_outputs/win32.rs",
  "crates/orbit-app/src/showcase.rs",
]);
const gpuiAppFiles = collectFiles(join(projectRoot, "crates", "orbit-app", "src"), /\.rs$/i);
for (const filePath of gpuiAppFiles) {
  const relativePath = relative(projectRoot, filePath).replaceAll("\\", "/");
  if (gpuiColorLiteralFiles.has(relativePath)) continue;
  for (const line of findGpuiColorLiterals(readFileSync(filePath, "utf8"))) {
    violations.push(`${relativePath}:${line} builds a color from a literal; read it from the theme instead.`);
  }
}

if (violations.length > 0) {
  console.error(violations.join("\n"));
  process.exitCode = 1;
} else {
  console.log(`Architecture boundaries OK (${gpuiAppFiles.length} orbit-app files).`);
}

function collectFiles(directory: string, pattern: RegExp): string[] {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const filePath = join(directory, entry.name);
    return entry.isDirectory() ? collectFiles(filePath, pattern) : pattern.test(entry.name) ? [filePath] : [];
  });
}
