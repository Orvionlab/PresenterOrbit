import { isAbsolute, join } from "node:path";

export function packageBuildDirectory(
  sourceRoot: string,
  target: string | undefined,
  cargoTargetRoot: string | undefined,
) {
  if (cargoTargetRoot !== undefined && (!cargoTargetRoot || !isAbsolute(cargoTargetRoot)))
    throw new Error("Packaged builds require an absolute CARGO_TARGET_DIR when it is configured.");
  const directory = cargoTargetRoot ?? join(sourceRoot, "target");
  return target ? join(directory, target, "release") : join(directory, "release");
}
