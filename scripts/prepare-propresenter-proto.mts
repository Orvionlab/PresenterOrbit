import { existsSync } from "node:fs";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const revision = "1b63dda196eb7e079721a8a4a7e7773520cb5ad2";
const files = [
  "LICENSE",
  "README.md",
  "proto/graphicsData.proto",
  "Proto7.16.2/graphicsData.proto",
  "Proto7.16.2/action.proto",
  "Proto7.16.2/playlist.proto",
  "Proto7.16.2/propresenter.proto",
  "Proto7.16.2/proscreen.proto",
  "Proto7.16.2/proworkspace.proto",
  "Proto7.16.2/screens.proto",
];

export async function prepareProPresenterProto(directory: string, download: typeof fetch = fetch) {
  const marker = resolve(directory, ".revision");
  if (
    existsSync(marker) &&
    (await readFile(marker, "utf8")) === revision &&
    files.every((file) => existsSync(resolve(directory, file)))
  ) {
    return;
  }

  const contents = await Promise.all(
    files.map(async (file) => {
      const response = await download(
        `https://raw.githubusercontent.com/greyshirtguy/ProPresenter7-Proto/${revision}/${file}`,
        { signal: AbortSignal.timeout(60_000) },
      );
      if (!response.ok) throw new Error(`Unable to download ProPresenter schema ${file}: HTTP ${response.status}`);
      return Buffer.from(await response.arrayBuffer());
    }),
  );
  for (const [index, file] of files.entries()) {
    const target = resolve(directory, file);
    await mkdir(dirname(target), { recursive: true });
    await writeFile(target, contents[index]);
  }
  await writeFile(marker, revision);
  console.log(`ProPresenter reference schemas prepared at ${revision}.`);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  await prepareProPresenterProto(fileURLToPath(new URL("../vendor/ProPresenter7-Proto", import.meta.url)));
}
