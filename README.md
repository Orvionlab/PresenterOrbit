# Presenter Orbit

Presenter Orbit is a desktop app for preparing and running live presentations. It manages libraries, playlists, cues, media, themes, stage displays, and audience outputs, with optional NDI support.

Presenter Orbit is built in Rust with GPUI. Program output runs in a separate process from the editor and live controls.

## Features

- Library and playlist management for service/order content.
- Cue editing with slide labels, text tools, and continuous playlist controls.
- Media and background bins for graphics, video, audio, gradients, colors, and placeholders.
- Theme editing with canvas and stage controls.
- Preview and program output, including screen routing and optional NDI sender support.
- Workspace storage and ProPresenter import into Presenter Orbit JSON.

## Getting started

Open Settings from the **Presenter Orbit** button in the top-left corner and choose an Orbit workspace folder. A new workspace starts with a Default library and editable sample themes and stage layouts.

An Orbit workspace has a `workspace.json` manifest and folders for libraries, playlists, media, and themes. Orbit copies imported assets into the workspace so saved presentations can use them there. The current workspace format is version 1.

## ProPresenter compatibility

Use **Import ProPresenter workspace** in Settings to copy libraries, playlists, themes, and media into an Orbit workspace. Review the import report for missing files and unsupported settings. Orbit reads the source folder during import and uses its own workspace files afterward.

The ProPresenter workspace export writes presentation text and workspace structure to a new folder. Review its loss overview before exporting. The export omits layered slides, dependent media, arrangements, and some actions and settings. To open the result in ProPresenter, use **Manage Workspaces → Import Legacy ProPresenter Folder**.

Presenter Orbit is an independent project. ProPresenter is a product of Renewed Vision. The importer uses [GreyShirtGuy's reference schemas](https://github.com/greyshirtguy/ProPresenter7-Proto).

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

Windows packages are unsigned by default. macOS packages use ad-hoc signing. See [package signing](docs/package-signing.md) for publisher certificates and [app updates](docs/app-updates.md) for signing and publishing update builds.

## Releases

Releases and the update feed are published manually. See [app updates](docs/app-updates.md) for the release procedure.

The app reads the `.arb` files in `crates/orbit-ui/locales` directly; there is no generation step. Run `npm run i18n:metadata` to fill missing English `@key` descriptions, and `npm run maintenance:check` to validate the locales.

## Contributing

Start with [CONTRIBUTING.md](CONTRIBUTING.md). The [architecture guide](docs/architecture-dataflow.md) describes `orbit-app`, `orbit-ui`, `orbit-host`, and how they work together. See the [design system](docs/design-system.md) for UI changes and [end-to-end testing](docs/e2e-testing.md) for operator workflows. Companion and Stream Deck modules are under [integrations](integrations).

## License

Presenter Orbit's source and original sample content are licensed under the GNU General Public License v3.0 only. See [LICENSE](LICENSE) for the full text.
Bundled components retain their licenses. The app and release packages include third-party notices for Rust dependencies and Tabler Icons. The Companion module has [its own MIT license](integrations/companion-orbit/LICENSE). Windows and macOS packages include separate notices and corresponding source for FFmpeg and ffprobe, described in [Bundled FFmpeg](docs/bundled-ffmpeg.md).
