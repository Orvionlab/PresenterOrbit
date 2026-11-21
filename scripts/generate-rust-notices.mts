// Writes the notices of the Rust crates compiled into orbit-app: every crate with its license and repository,
// followed by the license files found in each crate's source, grouped by identical text.
// Usage: node scripts/generate-rust-notices.mts --out <file> [--target <rust-triple>] [--json <file>]
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdirSync, readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { basename, dirname, join, resolve } from "node:path";

type Package = {
  id: string;
  name: string;
  version: string;
  license: string | null;
  license_file: string | null;
  repository: string | null;
  manifest_path: string;
  source: string | null;
};
type Node = { id: string; deps: { pkg: string; dep_kinds: { kind: string | null }[] }[] };

const args = process.argv.slice(2);
const option = (name: string) => (args.includes(name) ? args[args.indexOf(name) + 1] : undefined);
const out = option("--out");
if (!out) throw new Error("Usage: generate-rust-notices.mts --out <file> [--target <rust-triple>] [--json <file>]");

const upstream = JSON.parse(
  readFileSync(new URL("../docs/licenses/upstream-crate-notices.json", import.meta.url), "utf8"),
) as { packages: Record<string, { name: string; id: string; url: string }[]>; texts: Record<string, string> };

const run = (command: string, commandArgs: string[]) => {
  const result = spawnSync(command, commandArgs, { encoding: "utf8", maxBuffer: 1 << 28 });
  if (result.status !== 0) throw new Error(`${command} ${commandArgs.join(" ")} failed:\n${result.stderr}`);
  return result.stdout;
};
const target = option("--target") ?? /^host: (.+)$/mu.exec(run("rustc", ["-vV"]))?.[1];
if (!target) throw new Error("The host target could not be determined.");

const metadata = JSON.parse(
  run("cargo", ["metadata", "--format-version", "1", "--locked", "--filter-platform", target]),
) as { packages: Package[]; resolve: { nodes: Node[] } };
const packages = new Map(metadata.packages.map((item) => [item.id, item]));
const nodes = new Map(metadata.resolve.nodes.map((node) => [node.id, node]));
const root = metadata.packages.find((item) => item.name === "orbit-app");
if (!root) throw new Error("orbit-app is not in the workspace.");

// Normal dependencies only: build scripts and dev dependencies are not shipped.
const included = new Set<string>();
const visit = (id: string) => {
  if (included.has(id)) return;
  included.add(id);
  for (const dep of nodes.get(id)?.deps ?? []) if (dep.dep_kinds.some((kind) => kind.kind === null)) visit(dep.pkg);
};
visit(root.id);

const crates = [...included]
  .map((id) => packages.get(id)!)
  .filter((item) => item.source !== null)
  .sort((a, b) => a.name.localeCompare(b.name) || a.version.localeCompare(b.version));

const texts = new Map<string, { text: string; users: string[] }>();
const missing: string[] = [];
const entries: {
  name: string;
  version: string;
  license: string | null;
  repository: string | null;
  source: string | null;
  files: { name: string; textId: string }[];
}[] = [];
const upstreamSources: string[] = [];
const addText = (text: string, user: string) => {
  const key = createHash("sha256").update(text).digest("hex");
  const entry = texts.get(key) ?? { text, users: [] };
  entry.users.push(user);
  texts.set(key, entry);
  return key;
};
for (const item of crates) {
  const directory = dirname(item.manifest_path);
  const files = readdirSync(directory).filter(
    (name) => /^(licen[sc]e|copying|notice|unlicense)/iu.test(name) && statSync(join(directory, name)).isFile(),
  );
  if (item.license_file) {
    const declared = resolve(directory, item.license_file);
    if (!files.some((name) => resolve(directory, name) === declared)) files.push(item.license_file);
  }
  files.sort();
  const user = `${item.name} ${item.version}`;
  const entryFiles: { name: string; textId: string }[] = [];
  for (const name of files) {
    const text = readFileSync(resolve(directory, name), "utf8").replace(/\r\n/gu, "\n").trim();
    entryFiles.push({ name: basename(name), textId: addText(text, user) });
  }
  if (!files.length) {
    for (const record of upstream.packages[`${item.name}@${item.version}`] ?? []) {
      const text = upstream.texts[record.id];
      if (!text || addText(text, user) !== record.id) {
        throw new Error(`Invalid upstream license text for ${user}: ${record.name}`);
      }
      entryFiles.push({ name: record.name, textId: record.id });
      upstreamSources.push(`${user} | ${record.name} | ${record.url}`);
    }
  }
  if (!entryFiles.length) missing.push(`${user} | ${item.license ?? "No license expression declared"}`);
  entries.push({
    name: item.name,
    version: item.version,
    license: item.license,
    repository: item.repository,
    source:
      item.source === "registry+https://github.com/rust-lang/crates.io-index"
        ? `https://static.crates.io/crates/${item.name}/${item.name}-${item.version}.crate`
        : item.source,
    files: entryFiles,
  });
}

const tabler = "Tabler Icons";
const tablerText = readFileSync(new URL("../crates/orbit-ui/assets/icons/LICENSE", import.meta.url), "utf8").trim();
entries.push({
  name: tabler,
  version: "3.47.0",
  license: "MIT",
  repository: "https://github.com/tabler/tabler-icons",
  source: "https://github.com/tabler/tabler-icons/tree/v3.47.0",
  files: [{ name: "LICENSE", textId: addText(tablerText, tabler) }],
});

const lines = [
  "Presenter Orbit: Third-party notices",
  "",
  "Presenter Orbit is distributed under GPL-3.0-only (see LICENSE). This notice lists the Rust dependencies and",
  "Tabler Icons with their own license terms. The source links identify the versions included in this build.",
  "",
  `Components (${entries.length}, target ${target})`,
  "",
  ...entries.map(
    (item) =>
      `${item.name} ${item.version} | ${item.license ?? "No license expression declared"} | ${item.source ?? ""} | ${item.repository ?? ""}`,
  ),
  "",
  ...(missing.length
    ? ["Crates for which no standalone license text is bundled. See their source links above:", ...missing, ""]
    : []),
  ...(upstreamSources.length ? ["License files supplied from upstream repositories", ...upstreamSources, ""] : []),
  "License texts",
  "",
  ...[...texts.values()].flatMap(({ text, users }) => ["-".repeat(78), `Used by: ${users.join(", ")}`, "", text, ""]),
];
mkdirSync(dirname(out), { recursive: true });
writeFileSync(out, `${lines.join("\n")}\n`);
if (option("--json")) {
  writeFileSync(
    option("--json")!,
    JSON.stringify({ packages: entries, texts: Object.fromEntries([...texts].map(([id, entry]) => [id, entry.text])) }),
  );
}
console.log(`Third-party notices: ${entries.length} components, ${texts.size} distinct license texts -> ${out}`);
