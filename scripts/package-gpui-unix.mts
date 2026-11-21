// The macOS and Linux halves of scripts/package-gpui.mts. Written but untested on those systems.
import { chmod, cp, mkdir, readFile, symlink, writeFile } from "node:fs/promises";
import { basename, join } from "node:path";
import { verifyMacAdHocBundle, verifyOsSignature } from "./os-code-signing.mts";

type Context = {
  projectRoot: string;
  packaging: string;
  version: string;
  architecture: "x64" | "arm64";
  updater: boolean;
  osSigning: { signer: string } | undefined;
  outputRoot: string;
  staging: string;
  productName: string;
  binary: string;
  stageApplication: (
    root: string,
    layout: { resources: string; ffmpegDirectory?: string; ffmpegSources?: string },
  ) => Promise<void>;
  record: (path: string) => Promise<void>;
  run: (command: string, args: string[], cwd?: string, env?: NodeJS.ProcessEnv) => Promise<void>;
  writeUpdateRelease: (artifact: string, fileName: string) => Promise<void>;
};

// ---------------------------------------------------------------- macOS (untested)

export async function packageMacos(context: Context) {
  const {
    packaging,
    version,
    architecture,
    osSigning,
    staging,
    productName,
    outputRoot,
    binary,
    stageApplication,
    record,
    run,
    updater,
    writeUpdateRelease,
  } = context;
  const app = join(staging, `${productName}.app`);
  const contents = join(app, "Contents");
  const resources = join(contents, "Resources");
  const macos = join(contents, "MacOS");
  await mkdir(macos, { recursive: true });
  await stageApplication(app, { resources, ffmpegDirectory: macos, ffmpegSources: join(resources, "ffmpeg-source") });
  await cp(binary, join(macos, productName));
  await chmod(join(macos, productName), 0o755);
  await cp(join(packaging, "icons", "icon.icns"), join(resources, "icon.icns"));
  const plist = (await readFile(join(packaging, "macos", "Info.plist"), "utf8"))
    .replaceAll("__VERSION__", version.replace(/-.*$/u, ""))
    .replaceAll("__FULL_VERSION__", version);
  await writeFile(join(contents, "Info.plist"), plist);

  const entitlements = join(packaging, "macos", "Entitlements.plist");
  const identity = osSigning ? process.env.APPLE_SIGNING_IDENTITY! : "-";
  await run("codesign", [
    "--force",
    "--deep",
    "--sign",
    identity,
    "--entitlements",
    entitlements,
    ...(osSigning ? ["--options", "runtime", "--timestamp"] : []),
    app,
  ]);
  if (osSigning) await verifyOsSignature("macos", app, osSigning.signer);
  else await verifyMacAdHocBundle(app);

  const dmgRoot = join(staging, "dmg");
  await mkdir(dmgRoot, { recursive: true });
  await cp(app, join(dmgRoot, `${productName}.app`), { recursive: true, verbatimSymlinks: true });
  await symlink("/Applications", join(dmgRoot, "Applications"));
  const dmg = join(outputRoot, `${productName}_${version}_${architecture}.dmg`);
  await run("hdiutil", ["create", "-volname", productName, "-srcfolder", dmgRoot, "-ov", "-format", "UDZO", dmg]);
  await record(dmg);
  if (updater) {
    const archive = join(outputRoot, `${productName}.app.tar.gz`);
    await run("tar", ["-czf", archive, "-C", staging, `${productName}.app`]);
    await writeUpdateRelease(archive, basename(archive));
  }
}

// ---------------------------------------------------------------- Linux (untested)

export async function packageLinux(context: Context) {
  const {
    projectRoot,
    packaging,
    version,
    architecture,
    staging,
    productName,
    outputRoot,
    binary,
    stageApplication,
    record,
    run,
    updater,
    writeUpdateRelease,
  } = context;
  const name = "presenter-orbit";
  const desktop = `[Desktop Entry]\nType=Application\nName=${productName}\nComment=Presentation control and live output\nExec=${name}\nIcon=${name}\nCategories=AudioVideo;Video;\nTerminal=false\n`;
  // AppImage: the executable and its data share usr/bin, where the application looks for them.
  const appDir = join(staging, `${productName}.AppDir`);
  const bin = join(appDir, "usr", "bin");
  await stageApplication(bin, { resources: bin });
  await cp(binary, join(bin, name));
  await chmod(join(bin, name), 0o755);
  await cp(join(packaging, "icons", "128x128.png"), join(appDir, `${name}.png`));
  await writeFile(join(appDir, `${name}.desktop`), desktop);
  await writeFile(join(appDir, "AppRun"), `#!/bin/sh\nexec "$(dirname "$(readlink -f "$0")")/usr/bin/${name}" "$@"\n`, {
    mode: 0o755,
  });
  const appimagetool = process.env.PRESENTER_ORBIT_APPIMAGETOOL ?? "appimagetool";
  const appImage = join(outputRoot, `${productName}_${version}_${architecture}.AppImage`);
  await run(appimagetool, [appDir, appImage], projectRoot, {
    ...process.env,
    ARCH: architecture === "arm64" ? "aarch64" : "x86_64",
  });
  await record(appImage);

  // .deb: the application in /usr/lib/presenter-orbit, started through a /usr/bin symlink.
  const root = join(staging, "deb");
  const library = join(root, "usr", "lib", name);
  await stageApplication(library, { resources: library });
  await cp(binary, join(library, name));
  await chmod(join(library, name), 0o755);
  await mkdir(join(root, "usr", "bin"), { recursive: true });
  await symlink(`../lib/${name}/${name}`, join(root, "usr", "bin", name));
  await mkdir(join(root, "usr", "share", "applications"), { recursive: true });
  await writeFile(join(root, "usr", "share", "applications", `${name}.desktop`), desktop);
  await mkdir(join(root, "usr", "share", "icons", "hicolor", "128x128", "apps"), { recursive: true });
  await cp(
    join(packaging, "icons", "128x128.png"),
    join(root, "usr", "share", "icons", "hicolor", "128x128", "apps", `${name}.png`),
  );
  await mkdir(join(root, "usr", "share", "doc", name), { recursive: true });
  await cp(join(projectRoot, "LICENSE"), join(root, "usr", "share", "doc", name, "copyright"));
  await mkdir(join(root, "DEBIAN"), { recursive: true });
  const debArch = architecture === "arm64" ? "arm64" : "amd64";
  await writeFile(
    join(root, "DEBIAN", "control"),
    `Package: ${name}\nVersion: ${version.replace("-", "~")}\nSection: video\nPriority: optional\nArchitecture: ${debArch}\nMaintainer: Orvion Lab\nDepends: libwebkit2gtk-4.1-0, libgtk-3-0, libasound2, ffmpeg\nDescription: Presentation control and live output\n Presenter Orbit is a desktop presentation controller with cue editing, media playback, screen routing, and optional NDI output.\n`,
  );
  const deb = join(outputRoot, `${name}_${version}_${debArch}.deb`);
  await run("dpkg-deb", ["--build", "--root-owner-group", root, deb]);
  await record(deb);
  if (updater) await writeUpdateRelease(appImage, basename(appImage));
}
