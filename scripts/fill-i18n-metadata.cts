const { readFileSync, writeFileSync } = require("node:fs");
const { join } = require("node:path");

const arbPath = join("crates", "orbit-ui", "locales", "en.arb");
const arb = JSON.parse(readFileSync(arbPath, "utf8"));
const completed = { "@@locale": arb["@@locale"] };

for (const [key, value] of Object.entries(arb)) {
  if (key.startsWith("@")) {
    continue;
  }

  completed[key] = value;
  completed[`@${key}`] = arb[`@${key}`] ?? {
    description: createDescription(key),
  };
}

writeFileSync(arbPath, `${JSON.stringify(completed, null, 2)}\n`);

function createDescription(key) {
  const segments = key.split(".");
  const area = humanize(segments[0]);
  const purpose = humanize(segments.at(-1));
  return `User-facing ${purpose} text in the ${area} interface.`;
}

function humanize(value) {
  return String(value)
    .replace(/([a-z0-9])([A-Z])/g, "$1 $2")
    .replace(/[-_]/g, " ")
    .toLowerCase();
}
