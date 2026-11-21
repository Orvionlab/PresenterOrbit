# Phase 16, part 1: packaging, updater, carry-over, CI

Status: Windows packaging was built and checked on this machine (without running the installer). macOS and Linux packaging is written and **untested**. Part 2 (below) deleted Tauri on the maintainer's approval; the last commit that contains it is `64121abe`. The tables and notes of part 1 are kept as written; read `src-tauri/`, `src/renderer/` and `src/shared/` in them as history.

## Inputs that moved out of the Tauri trees

| Was | Now | Notes |
| --- | --- | --- |
| `src/renderer/features/i18n/locales/{en,nl}.arb` | `crates/orbit-ui/locales/` | `npm run i18n:build` reads them here and still writes the React `generated.ts`; the maintenance scripts and tests read the new location. |
| `src/shared/macro-icons.json` | `crates/orbit-host/assets/macro-icons.json` | The React catalogues import it from there until they are deleted. |
| `src-tauri/icons/128x128@2x.png` (test pattern) | `crates/orbit-host/assets/logo.png` | Copy. |
| `src-tauri/icons/*`, `Entitlements.plist`, `Info.plist` | `crates/orbit-app/packaging/` | Copies (`icons/`, `macos/`, `windows/installer.nsi`). |
| `src-tauri/resources/Sample Content` | `resources/Sample Content` | Moved; `tauri.conf.json` maps `../resources/` so the Tauri bundle is unchanged. |

Still read from outside the crates, and so needing a decision when Tauri is deleted: `tests/fixtures/` (test inputs; keep), `vendor/ProPresenter7-Proto/` (test inputs, downloaded by `npm install` / `npm run proto:prepare`, so the script must survive or be replaced), `LICENSE`, `src/shared/*.ts` (React only), and `scripts/package-ffmpeg.mts` plus `scripts/build-package-ffmpeg.sh` and `scripts/os-code-signing.mts` (reused by the GPUI packager, which also calls the Tauri CLI only for `signer sign`, see below).

## Packaging

`node scripts/package-gpui.mts --platform windows|macos|linux [--x64|--arm64] [--updater] [--os-signed] [--out <dir>] [--skip-build] [--makensis <path>]` (npm: `npm run package:gpui:windows`, `:macos`, `:linux`). It builds `orbit-app` in release mode, generates the Rust notices, stages the package, and writes `package-report-<platform>-<arch>.json` (hashes, signing state). Output defaults to `dist/gpui/`.

Windows (verified):

- `Presenter Orbit_<version>_<arch>-setup.exe`: NSIS installer, `crates/orbit-app/packaging/windows/installer.nsi`. Same identity as the Tauri installer so it upgrades it in place: product name `Presenter Orbit`, publisher `Orvion Lab`, per-machine (`RequestExecutionLevel admin`), default folder `%ProgramFiles%\Orvionlab\PresenterOrbit` (an existing install keeps its registered folder), registry `HKLM\Software\Orvion Lab\Presenter Orbit` and the `Uninstall\Presenter Orbit` key, Start menu shortcut, desktop shortcut on the finish page, executable `Presenter Orbit.exe`, uninstaller `uninstall.exe`, English and Dutch with a language selector, and the Tauri command line (`/P` passive, `/UPDATE`, `/R` relaunch, `/NS`, `/S`). There are no file associations (Tauri has none). It closes a running instance (the output process shares the executable name) and downloads the WebView2 bootstrapper only if the runtime is missing (web fills use it). Uninstall removes the listed files, shortcuts and keys, and asks, interactively only, whether to delete the application data.
- `Presenter Orbit_<version>_<arch>-portable.zip`: the same files plus `data/README.txt`.
- Contents: `Presenter Orbit.exe`, `ffmpeg.exe`, `ffprobe.exe`, `ffmpeg-source/` (as in `docs/bundled-ffmpeg.md`, built by `scripts/package-ffmpeg.mts`, never downloaded at run time), `resources/Sample Content/`, `LICENSE`, `THIRD-PARTY-NOTICES.txt`.
- NSIS: the script looks for `--makensis`, `PRESENTER_ORBIT_MAKENSIS`, `makensis.exe` on `PATH`, the NSIS install folders (the lookup of the copy that Tauri cached in `%LOCALAPPDATA%\tauri\NSIS` was removed in part 2). CI needs NSIS installed (`choco install nsis`; the Windows runner images normally have it); on a machine that only has the old Tauri copy, pass `--makensis` or set `PRESENTER_ORBIT_MAKENSIS`. No dependency was added.

macOS (untested): `Presenter Orbit.app` (Info.plist from `packaging/macos`, identifier `com.presenterorbit.desktop`, FFmpeg in `Contents/MacOS`, resources in `Contents/Resources`), ad-hoc signed with the entitlements file (`--os-signed`: `APPLE_SIGNING_IDENTITY`, hardened runtime, timestamp; notarization is not scripted here, add `xcrun notarytool` per `docs/package-signing.md` when needed), then `hdiutil` DMG. With `--updater`, a `.app.tar.gz` for the feed.

Linux (untested): AppImage via `appimagetool` (`PRESENTER_ORBIT_APPIMAGETOOL` or `PATH`; it does not bundle WebKitGTK, which Tauri's AppImage did; web fills need the system library) and a `.deb` with `dpkg-deb` (depends as the Tauri deb did: `libwebkit2gtk-4.1-0`, `libgtk-3-0`, `libasound2`, `ffmpeg`). Linux keeps the system FFmpeg.

Signing: the Windows (`PRESENTER_ORBIT_WINDOWS_CERTIFICATE_THUMBPRINT`, `PRESENTER_ORBIT_WINDOWS_TIMESTAMP_URL`) and macOS flows and the `--os-signed` / `PRESENTER_ORBIT_REQUIRE_OS_SIGNING` switches of `docs/package-signing.md` are reused unchanged (`scripts/os-code-signing.mts`). Without them the artifacts are unsigned and the script prints `UNSIGNED`. The updater signature is written by `scripts/updater-signing.mts` from `TAURI_SIGNING_PRIVATE_KEY` (and `_PASSWORD`); it replaced the Tauri CLI call in part 2 and writes the same prehashed minisign `.sig`.

Notices: `scripts/generate-rust-notices.mts` lists every crate compiled in (`cargo metadata`, normal dependencies, for the target) with its license and repository, then the license files from each crate's source grouped by identical text (about 590 crates, 1 MB). It is shipped as `THIRD-PARTY-NOTICES.txt` beside the executable (`Contents/Resources` on macOS) and the About window's Licenses page shows it after the GPL text. A build that was not packaged says the file is missing.

## Updater

Code: `crates/orbit-app/src/updates/`. It replaces the stub; dependencies added to `orbit-app`: `minisign-verify`, `semver`, `ureq`, `base64` (all already in `Cargo.lock`; `ring` as a dev dependency for the tests).

- Feed: `PRESENTER_ORBIT_UPDATE_BASE_URL` and `PRESENTER_ORBIT_UPDATE_PUBLIC_KEY` (the Tauri `.pub` contents) are compiled in only by an `--updater` package. Without them the status is `unconfigured`, as in Tauri. Endpoint: `<base>/<platform>-<arch>/latest.json` with `windows|macos|linux` and `x64|arm64`, the same folders `package-tauri.mts` writes.
- Manifest: Tauri static format (`version`, `notes`, `pub_date`, `platforms.<os>-<arch>[-<installer>]` with `url`, `signature`). The most specific key is used (`windows-x86_64-nsis` before `windows-x86_64`). Only a strictly newer semver (pre-releases ordered correctly) counts.
- Verification: the whole download is checked against the minisign signature (base64 of the `.sig` file, as `tauri signer sign` writes it) with the embedded key before anything runs. A mismatch leaves the status `error` and the update pending for retry.
- Status JSON is the Tauri one (`status`, `startupCheck`, `currentVersion`, `version`, `notes`, `downloaded`, `total`, `error`), so the Updates panel works unchanged; the panel no longer maps the stub's `unavailable`.
- Startup: in release builds, when `checkForUpdatesOnStartup` is on, the check runs in the background after the host starts; an available update opens the startup dialog (install now / later, progress, notes, error), as `StartupUpdateDialog.tsx`. Debug builds never check at startup.
- Install: Windows writes the installer to a temp folder and starts it with `ShellExecuteW` and `/P /UPDATE /R` (it asks for elevation, as the Tauri updater does), stops outputs, and quits; the installer waits for the process to end (force-closing it after 10 s) and relaunches the app. macOS swaps the bundle (extracted `.app.tar.gz` next to it) and runs `open -n`; Linux replaces the running AppImage. The macOS and Linux paths are untested.
- Tests (`updates/tests.rs`): a local fake feed on a loopback port, a key pair generated in the test (`ring`), tampered download, wrong key, bad base64, older/equal/newer and pre-release versions, platform key preference, feed errors, and the unconfigured state. The real key is never used.

### Publishing the transition release (Tauri to GPUI)

An installed Tauri app polls `<base>/<platform>-<arch>/latest.json` (compiled into it), accepts a higher version whose artifact verifies against its embedded public key, and runs the artifact. So the GPUI release is published exactly like a Tauri one:

1. Bump the version above the last Tauri release in `Cargo.toml` (workspace) and `package.json` (the packager refuses a mismatch).
2. On Windows: `PRESENTER_ORBIT_UPDATE_BASE_URL`, `PRESENTER_ORBIT_UPDATE_PUBLIC_KEY` (the same public key as the Tauri releases), `TAURI_SIGNING_PRIVATE_KEY` (+ `_PASSWORD`), optionally `PRESENTER_ORBIT_UPDATE_NOTES`, then `node scripts/package-gpui.mts --platform windows --x64 --updater` (add the `--os-signed` variables if a certificate exists).
3. Upload `updates/windows-x64/<version>/Presenter Orbit_<version>_x64-setup.exe` and its `.sig`, then `updates/windows-x64/latest.json` last (`Cache-Control: no-cache`), as in `docs/app-updates.md`. The manifest key is `windows-x86_64` and the artifact must be the NSIS installer `.exe` (the old app runs `<installer> /P /UPDATE /ARGS` and relaunches via `/R`). Same for macOS (`updates/macos-<arch>/…/Presenter Orbit.app.tar.gz`, key `darwin-<arch>`, archive top level `Presenter Orbit.app`) and Linux (`.AppImage`, key `linux-x86_64`); the last two are untested against an old Tauri app.
4. The package must be installed over a real Tauri install on a spare machine before publishing broadly.

The same key and feed make later GPUI-to-GPUI updates use the same steps.

## Identifier, data directory, and settings

Decision: a **release build uses `com.presenterorbit.desktop`**, the Tauri identifier, as its application data folder (`%APPDATA%\com.presenterorbit.desktop`, `~/Library/Application Support/…`, `~/.local/share/…`) and for the macOS bundle. An installation updated from Tauri then has its settings, workspace registry, screens, devices and downloaded Bible packages in place: no migration code, no import. A **debug build keeps `com.presenterorbit.gpui`** so development never writes to the settings an installed release reads (loading a workspace writes its path there). On its first start with no `settings.json`, a debug build copies the Tauri `settings.json` once (`carry_over.rs`; read-only on the source, never overwrites, tests use temp folders). `--data-dir <path>` overrides the folder for any build and skips the import. A consequence: running a *release* GPUI build by hand uses the real Tauri data; use `--data-dir`.

Layout positions that Tauri kept in WebView `localStorage` are not carried (the GPUI layout starts at its defaults). The old Electron `presenter-orbit` settings import in `orbit-host` is kept as is: it is shared with Tauri, runs only when no workspace path exists after the Tauri settings, and Electron installs are not an update source.

## CI

Workflows (the Tauri ones were deleted in part 2; `gpui-ci.yml` now runs `npm run verify`):

- `gpui-ci.yml`: Windows gate (part 1: fmt, clippy, tests, release build; part 2: `npm run verify`); macOS arm64 and Linux x64 build and test with `continue-on-error`.
- `gpui-package-acceptance.yml` (manual): Windows packages, unpacks the ZIP and runs `scripts/gpui-smoke.mts` (starts it on a private data folder, checks bundled files and FFmpeg, stays up N seconds), uploads artifacts and a signing-state summary; macOS and Linux jobs are opt-in and allowed to fail.
- `gpui-burn-in.yml` (weekly and manual): repeats the tests and the smoke run on the package-test runner.

None was run (no push). The Tauri smoke (`npm run tauri:smoke`) has no GPUI twin with native host probes; the GPUI app is covered by the Rust tests and the packaged smoke.

## Needs a human

- Signing secrets: the Tauri updater private key and password, the Windows certificate, Apple identity/notarization. Nothing was created, fetched or printed.
- A real install and upgrade test on a spare machine or VM: Tauri install, then the GPUI installer through the Tauri updater (`/P /UPDATE /R`), settings intact; silent install and uninstall; per-machine elevation.
- Mac and Linux: build, package, and update the packages; Gatekeeper; AppImage and `.deb` behavior.
- The service-length burn-in on real hardware and the operator checklist (`docs/propresenter-parity.md`).
- Deciding when the Tauri tree is deleted, including where the updater signing call and NSIS come from afterwards.

## Part 2: Tauri deleted

Done on the maintainer's approval of Phase 16 step 5. The last commit containing Tauri is `64121abe`.

- **Replaced before deleting:** updater signing (`scripts/updater-signing.mts`, the Tauri encoding of the key, prehashed minisign signature, tested with a throwaway key and verified by the `orbit-app` updater against a committed public fixture in `crates/orbit-app/src/updates/testdata/`; **not** exercised with the production key); NSIS lookup (flag, `PRESENTER_ORBIT_MAKENSIS`, `PATH`, NSIS folders).
- **Deleted:** `src-tauri/`, `src/renderer/`, `src/shared/`, `tests/renderer/`, the Vite, Vitest, ESLint and TypeScript configuration, the React and Tauri npm packages and scripts (`package.json` keeps Prettier), the renderer, Tauri and package-acceptance scripts for the Tauri packages, the Tauri-only workflows, the generator of the React `generated.ts`, the renderer-only fixtures, the `orbit-host` native smoke (`run_native_smoke`, `should_run_native_smoke` and `communication_deck_smoke.rs`, only called by the Tauri adapter), and `src-tauri` from the Cargo workspace.
- **Kept:** `package-gpui*.mts`, `package-ffmpeg.mts`, `os-code-signing.mts`, `generate-rust-notices.mts`, `gpui-smoke.mts`, `prepare-propresenter-proto.mts`, the workspace and version checks, `tests/fixtures/` that Rust tests read, the `integrations/` modules, and `build/icon.*` (unreferenced leftovers, not touched).
- **Gate:** `npm run check` (quick) and `npm run verify` (adds all tests); see `AGENTS.md`. `scripts/maintenance-baseline.json` keeps the locale metadata ratchet only.
- **Docs rewritten for GPUI:** `AGENTS.md`, `README.md`, `CONTRIBUTING.md`, `docs/design-system.md`, `docs/architecture-dataflow.md`, `docs/maintenance-guide.md`, `docs/package-signing.md`, `docs/app-updates.md`, the `.agents/skills`, and `docs/tauri-migration.md` (now a short historical note with the recovery commands).

### Needs a human (open)

- Signing secrets: the updater private key and password (and a check that `scripts/updater-signing.mts` output verifies in an installed Tauri app), the Windows certificate, the Apple identity and notarization.
- A real install and upgrade test on a spare machine: Tauri install, then the GPUI installer through the Tauri updater, settings intact; silent install, uninstall, per-machine elevation.
- macOS and Linux: build, package, update, Gatekeeper, AppImage and `.deb` behavior.
- The FFmpeg cache rebuild (`scripts/package-ffmpeg.mts`, `windows-media-build.yml`) on the runner that publishes releases.
- The service-length burn-in on real hardware and the operator checklist (`docs/propresenter-parity.md`).
- Running the CI workflows (`gpui-ci.yml`, `gpui-package-acceptance.yml`, `gpui-burn-in.yml`, `web-fill-acceptance.yml`) for the first time.
- A GPUI release workflow: the tag-triggered release and Cloudflare R2 publishing lived in the deleted `ci.yml` (see `docs/app-updates.md`).
