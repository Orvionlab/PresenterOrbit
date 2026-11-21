import { build } from "esbuild";
import { cp, mkdir, readFile, readdir, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import path from "node:path";
const root = fileURLToPath(new URL(".", import.meta.url));
const output = path.join(root, "com.presenterorbit.control.sdPlugin");
await mkdir(path.join(output, "bin"), { recursive: true });
await cp(path.join(root, "assets"), output, { recursive: true });
const result = await build({
  entryPoints: [path.join(root, "src/plugin.mjs")],
  outfile: path.join(output, "bin/plugin.js"),
  bundle: true,
  platform: "node",
  target: "node20",
  format: "esm",
  banner: { js: 'import { createRequire } from "node:module"; const require = createRequire(import.meta.url);' },
  external: ["bufferutil", "utf-8-validate"],
  legalComments: "linked",
  metafile: true,
});
await writeFile(path.join(output, "package.json"), JSON.stringify({ type: "module" }));
await cp(path.join(root, "../../LICENSE"), path.join(output, "LICENSE"));
const packages = new Set();
for (const input of Object.keys(result.metafile.inputs)) {
  let dir = path.dirname(path.resolve(input));
  while (dir.includes("node_modules")) {
    try {
      await readFile(path.join(dir, "package.json"));
      packages.add(dir);
      break;
    } catch {
      dir = path.dirname(dir);
    }
  }
}
const notices = [
  "Shared Orbit HTTP client and target helpers:\n" +
    (await readFile(path.join(root, "../companion-orbit/LICENSE"), "utf8")),
];
for (const dir of [...packages].sort()) {
  const metadata = JSON.parse(await readFile(path.join(dir, "package.json"), "utf8"));
  const licenses = (await readdir(dir)).filter((name) => /^(license|copying)(\.|$)/i.test(name));
  if (!licenses.length) throw new Error(`Missing bundled dependency license: ${metadata.name}`);
  notices.push(
    `${metadata.name}@${metadata.version}\n` +
      (await Promise.all(licenses.map((name) => readFile(path.join(dir, name), "utf8")))).join("\n"),
  );
}
await writeFile(path.join(output, "THIRD-PARTY-NOTICES.txt"), notices.join("\n\n"));
