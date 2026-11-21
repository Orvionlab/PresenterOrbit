import { readFileSync } from "node:fs";

const read = (path: string) => readFileSync(path, "utf8");
const version = JSON.parse(read("package.json")).version as string;
const failures: string[] = [];
const expect = (location: string, actual: string | undefined) => {
  if (actual !== version) failures.push(`${location}: expected ${version}, found ${actual ?? "missing"}.`);
};

expect("Cargo.toml workspace version", read("Cargo.toml").match(/^version = "([^"]+)"/mu)?.[1]);
const lock = read("Cargo.lock");
for (const crate of ["orbit-app", "orbit-host", "orbit-ui"])
  expect(`Cargo.lock ${crate}`, lock.match(new RegExp(`name = "${crate}"\r?\nversion = "([^"]+)"`, "u"))?.[1]);
const npmLock = JSON.parse(read("package-lock.json"));
expect("package-lock.json version", npmLock.version);
expect("package-lock.json root version", npmLock.packages[""].version);

if (failures.length > 0) {
  console.error(`Version configuration failed:\n${failures.join("\n")}`);
  process.exitCode = 1;
} else {
  console.log(`Versions OK (${version}).`);
}
