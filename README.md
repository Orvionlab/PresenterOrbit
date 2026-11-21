# Presenter Orbit

Presenter Orbit is a desktop app for preparing and running live presentations. It manages libraries, playlists, cues, media, themes, stage displays, and audience outputs, with optional NDI support.

The application is written in Rust. The interface is built with GPUI on the Presenter Orbit design system (`crates/orbit-ui`), the application is `crates/orbit-app`, and the host services behind it (native storage, output playback, presentation-timer and automation clocks, output scene composition, media decoding, wgpu program-output surfaces, NDI, diagnostics, and remote control) are `crates/orbit-host`. Program output runs in its own process. The earlier Tauri and React app was removed in the GPUI migration; see [docs/gpui-migration](docs/gpui-migration/README.md) and [docs/tauri-migration.md](docs/tauri-migration.md).

## Current Scope

- Library and playlist management for service/order content.
- Cue editing with slide labels, text tools, and continuous playlist controls.
- Media and background bins for graphics, video, audio, gradients, colors, and placeholders.
- Theme editing with canvas and stage controls.
- Preview and program output, including screen routing and optional NDI sender support.
- Workspace storage and ProPresenter import into Presenter Orbit JSON.

## Project Structure

```text
crates/
  orbit-app/     GPUI application, output process, updater, packaging inputs
  orbit-ui/      Theme, primitives, icons, translations (locales/*.arb)
  orbit-host/    Host services and the Host::invoke dispatcher
resources/       Sample Content shipped with the app
scripts/         Packaging and maintenance scripts (Node)
tests/           Script tests and shared fixtures
integrations/    Companion and Stream Deck modules
docs/            Design system, architecture, migration plan
vendor/          Downloaded third-party reference schemas (ignored by Git)
```

## Presenter Orbit Workspace and ProPresenter Import

Presenter Orbit stores the active workspace in its own folder structure. The selected folder contains a small `workspace.json` manifest plus folders such as `Libraries/`, `Playlists/`, `Media/`, and `Themes/`. Libraries and playlist items are stored as separate JSON files in those folders. Themes live under `Themes/` in a dedicated subfolder per theme, with `theme.json` and an `Assets/` folder for media used by that theme. Media uses a central index in `Media/index.json`; the actual media assets live under `Media/Assets/`. `Graphics Files` shows all image/video assets from that folder and `Audio Files` shows all audio assets. Media playlists and folders only store references to asset files. Authored media identity and presentation properties are stored in the index; native loading derives filesystem-dependent fields such as the current asset URL. The runtime therefore does not depend on ProPresenter files and the workspace stays inspectable by hand.

The current development workspace schema is version 1 and is loaded strictly. Presenter Orbit does not migrate earlier experimental workspace structures or reconstruct missing indexes and legacy fields. During development, recreate or manually update an older workspace before opening it.

Workspace mutations are serialized and revision-checked by the Rust host. A stale editor or window is rejected before it can overwrite a newer workspace snapshot.

Global settings use the same native concurrency boundary. Settings reads return an in-process revision, writes must supply that revision, and audio or screen changes persist and apply as one ordered Rust operation. Stale settings windows are rejected before they can change either disk state or the active native output runtime.

Open settings from the `Presenter Orbit` button in the top-left corner. The flow is:

1. Use `Choose Orbit workspace folder` to select where Presenter Orbit may store its own workspace.
2. Use `Import ProPresenter workspace` once to select a local ProPresenter workspace folder.

During import, Presenter Orbit reads libraries from the `Libraries/` folder inside the ProPresenter workspace; every direct subfolder becomes a library. Candidate files such as `.pro`, `.propresenter`, and `.proplaylist` inside a library folder are converted into separate `.json` files under `Libraries/`. `Playlists/Library` and `.proplaylist` files in `Playlists/` are stored as `.json` files. Files in `Media/` are copied to `Media/Assets/`; their placement is tracked in `Media/index.json`. ProPresenter themes are stored as `Themes/<theme>/theme.json`; local media found in a theme is copied to `Themes/<theme>/Assets/`.

Import preserves playlist hierarchy, source order, empty folders/playlists and empty presentations. Presentation references resolve by exact local path or the document's workspace-relative `ROOT_SHOW` path, including escaped Unicode filenames. A missing reference remains an unlinked placeholder with an import warning, rather than selecting another library's file by name. Props, masks and stage layouts capture their media dependencies as well as presentation/theme assets. Source and destination folders must not overlap. Invalid sources do not initialize the destination; a failed import removes a newly created destination, and failed asset capture leaves an existing Orbit workspace unchanged.

The native importer implements a subset of the ProPresenter wire format based on GreyShirtGuy's `.proto` schemas. Rust tests check selected reference schemas under `vendor/ProPresenter7-Proto`. The importer also preserves empty cues as empty slides and uses ProPresenter cue groups as slide labels, such as `Chorus` or `Verse 1`.

Presenter Orbit is an independent project. ProPresenter is a product of Renewed Vision.

The path to the Presenter Orbit workspace and group color/key mappings are stored in the native host settings. On the next start, Presenter Orbit loads the selected JSON workspace file. ProPresenter is only read again when you intentionally import it again.

If no workspace has been configured yet, Presenter Orbit starts empty until you choose an Orbit workspace folder. A new Orbit workspace gets an empty Default library and no playlists.

## ProPresenter Structure Export

The ProPresenter workspace export requires a new destination folder. It writes libraries, a hierarchical `Playlists/Library`, themes and configuration records for timers, props, masks, stage layouts, groups, messages, macros and clear groups. Presentation text is exported with an explicit readable style and cue groups. Message template references share the exported theme slide identities. Playlist presentation paths include workspace-relative references, so moving the complete exported folder preserves those links.

Review the loss overview before choosing the destination. This is a portable text and workspace-structure export: layered slides, dependent media, arrangements, macro actions and several application settings are omitted. Windows-invalid or colliding names receive distinct filenames; ProPresenter displays presentation filenames as titles. `export-report.json` records counts, presentation paths, original presentation names and the loss findings. An existing destination is never replaced, and a failed export removes only its newly reserved incomplete folder.

In ProPresenter's General settings, open the active workspace menu, choose Manage Workspaces, then use the plus menu's Import Legacy ProPresenter Folder option to open the exported folder as a separate workspace.


## Development

Install Rust (the MSVC toolchain on Windows) and Node.js 26.5.1 with npm 12.0.2 (`.node-version` and `.nvmrc`; npm enforces the engine range). Then:

```bash
npm install
```

Installation downloads the ProPresenter reference schemas and upstream license from the fixed commit in `scripts/prepare-propresenter-proto.mts` into the Git-ignored `vendor/ProPresenter7-Proto` directory; no Git submodule is needed. The first download requires internet access. Run `npm run proto:prepare` to restore the cache before invoking Cargo tests directly. Updates are manual: change the pinned commit and run the importer tests.

Run the app against a copy of a workspace:

```bash
cargo run -p orbit-app -- --workspace <folder with workspace.json>
cargo run -p orbit-app -- --showcase
```

`npm start` runs the same app without arguments. Debug builds keep their data in `com.presenterorbit.gpui` and import the release settings once; release builds use `com.presenterorbit.desktop`; `--data-dir <folder>` overrides both and should be used for any test run.

Dev builds retain optimization level 2 for `orbit-host` (native output) and reduced debug information to shorten compilation and linking. Inspecting Rust variables in a debugger requires `CARGO_PROFILE_DEV_DEBUG=2`.

`npm run dev:hot` enables GPUI hot reload with a separate build cache and private development data. UI render changes can be applied while the app stays open. See [Hot reload](docs/hot-reload.md) for setup, workspace copies, and changes that require a restart.

The gate (see [AGENTS.md](AGENTS.md) and [docs/maintenance-guide.md](docs/maintenance-guide.md)):

```bash
npm run check     # script rules, locale consistency, fmt, clippy (none in orbit-app and orbit-ui)
npm run verify    # check plus the focused end-to-end suite
npm run test:e2e  # operator workflows only; no unit/component/timing suites
```

Linux needs the system libraries listed in `.github/workflows/gpui-ci.yml` (GTK 3, WebKitGTK 4.1, ALSA, X11/XCB, Vulkan, fontconfig). Physical output placement on Linux requires X11 or XWayland.

## Packaging

```bash
npm run package:gpui:windows    # NSIS installer and portable ZIP in dist/gpui/
npm run package:gpui:macos      # .app and .dmg (untested)
npm run package:gpui:linux      # AppImage and .deb (untested)
```

Each platform builds on its own operating system. Options: `--x64`, `--arm64`, `--updater`, `--os-signed`, `--out <dir>`, `--skip-build`, `--makensis <path>`. The Windows installer needs NSIS (`makensis.exe` on `PATH`, `PRESENTER_ORBIT_MAKENSIS`, or `--makensis`); install it with `choco install nsis` or `winget install NSIS.NSIS`. The Windows portable ZIP contains the application, FFmpeg and ffprobe with their corresponding sources and licenses (`docs/bundled-ffmpeg.md`), the Sample Content, and the notices for the Rust dependencies. Run `node scripts/gpui-smoke.mts --dir <unpacked package>` to start a package on a private data folder.

The standalone ZIP does not isolate application settings: they use the Windows user profile. Use `--data-dir` to place them elsewhere.

Windows packages are unsigned by default. macOS packages use ad-hoc signing without an Apple account or Developer ID. Publisher certificates are optional on both platforms through `--os-signed`; see [package signing](docs/package-signing.md). Updater builds (`--updater`) sign with the Tauri-format minisign key from `TAURI_SIGNING_PRIVATE_KEY`, so installs of the earlier Tauri app can update to this one; see [app updates](docs/app-updates.md) and [Phase 16](docs/gpui-migration/phase-16.md).

Windows x64 and Linux x64 can build through the configured local GitHub Actions runners, with Linux running natively inside WSL2. See [local runner startup and build jobs](docs/self-hosted-runners.md).

## Releases

There is no automatic release workflow yet. `gpui-package-acceptance.yml` builds and smoke-tests the packages; publishing the artifacts and the update feed is manual (`docs/app-updates.md`).

The app reads the `.arb` files in `crates/orbit-ui/locales` directly; there is no generation step. Run `npm run i18n:metadata` to fill missing English `@key` descriptions, and `npm run maintenance:check` to validate the locales.

## Extending

New features are a folder under `crates/orbit-app/src/<feature>/` with a view, state, and controller, using primitives from `orbit-ui` and host methods through typed wrappers. Keep non-visual behavior, persistence, normalization, sorting, and selection logic in controllers and pure helpers rather than in `Render` implementations. Add Rust tests for the layer that owns the behavior and an end-to-end test for storage and output flows.

## License


Presenter Orbit's source and original sample content are licensed under the GNU General Public License v3.0 only. See [LICENSE](LICENSE) for the full text.
Bundled components retain their licenses. The app and release packages include third-party notices for Rust dependencies and Tabler Icons. The Companion module has [its own MIT license](integrations/companion-orbit/LICENSE). Windows and macOS packages include separate notices and corresponding source for FFmpeg and ffprobe, described in [Bundled FFmpeg](docs/bundled-ffmpeg.md).
