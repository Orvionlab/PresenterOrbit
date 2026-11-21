# AGENTS.md

## Project Overview

Presenter Orbit is a cross-platform desktop app for live presentations: libraries, playlists, cue editing, media, theme editing, stage displays, audience outputs, and optional NDI output.

The whole app is Rust. The UI is GPUI (`crates/orbit-app/`), built on the Presenter Orbit design system in `crates/orbit-ui/`. Host services (workspace storage, settings, output composition, audio, NDI, ProPresenter import, remote control) live in `crates/orbit-host/` and have no windowing or UI dependency. Program output runs in a separate process of the same executable (`crates/orbit-app/src/output_process/`). See `docs/tauri-migration.md` for background on the earlier Tauri and React app.

## Dependencies

- Runtime: Rust/Cargo (workspace `Cargo.toml`, `Cargo.lock` at the root). GPUI is the pinned `gpui-pre` crate.
- Node.js/npm are used only for repository scripts: packaging, maintenance checks, and the ProPresenter schema cache. `package.json` has one dev dependency (Prettier) and no production dependencies.
- Core crates: `gpui`, `wry` (web fills), `wgpu`, `cosmic-text`, `resvg`, `ffmpeg-sidecar`, `rodio`, and the ProPresenter parsing and NDI modules in `orbit-host`.

Run `npm install` once (it also prepares `vendor/ProPresenter7-Proto/`, which import tooling uses). Do not add new dependencies unless they are clearly necessary. Prefer existing crates and local utilities.

## Common Commands

```bash
cargo run -p orbit-app -- --workspace <copy>     # run the app (npm start does the same without arguments)
cargo run -p orbit-app -- --showcase             # the orbit-ui primitives
npm run check                                    # quick gate
npm run verify                                   # full gate: check plus end-to-end tests
npm run maintenance:check                        # script-side rules only
npm run test:e2e                                 # complete operator workflows
npm run package:gpui:windows                     # also :macos and :linux
npm run gpui:smoke -- --dir <unpacked package>      # start a packaged build on a private data folder
```

- `npm run check` runs, in order: the toolchain and workflow-pin checks, the orbit-app color-literal check (`scripts/check-architecture-boundaries.mts`), locale consistency (`scripts/check-i18n.mts`), version agreement, workspace compatibility, Prettier on scripts, syntax of the scripts, `cargo fmt --all --check`, and `scripts/check-clippy.mts` (clippy over the workspace: no findings in `orbit-app` and `orbit-ui`, `orbit-host` within its baseline in `scripts/maintenance-baseline.json`).
- `npm run verify` adds `npm run test:e2e`: complete operator workflows through GPUI's test platform, real workspace files, and real output child processes. The internal unit/component/timing and Node unit suites were removed by product request. Do not reintroduce them; extend the small E2E suite for critical user flows.
- A release-profile run of `orbit-app` without `--data-dir` uses the real `com.presenterorbit.desktop` data folder. Always pass `--data-dir <scratch>` for tests and smoke runs; debug builds use `com.presenterorbit.gpui` and import the release settings once.

## Architecture Map

```text
Cargo.toml                Workspace root; Cargo.lock lives here too
crates/orbit-app/         GPUI application: state, feature views, canvas, output process, updater, packaging inputs
  src/main.rs             Entry point and command-line options
  src/state/              State entities, revisions, the workspace/settings lanes
  src/<feature>/          One folder per feature (cues, collections, media, settings, transitions, ...)
  src/output_process/     The program output process and its link to the UI
  src/native_outputs/     Platform output windows and the web fill capture adapters
  src/updates/            Updater (Tauri feed and minisign format) and installers
  packaging/              Icons, NSIS script, macOS plist and entitlements
crates/orbit-ui/          Theme, metrics, primitives (controls/), icons, translations; locales/*.arb
crates/orbit-host/        Host services and the bridge dispatcher (Host::invoke), no UI dependency
  src/lib.rs              Host composition, settings, outputs, and tickers
  src/host_api.rs         Host::invoke dispatcher
  src/workspace.rs        Workspace read/write orchestration
  src/propresenter_*.rs   ProPresenter import parsing and conversion
  src/ndi.rs              Cross-platform NDI sender service
  src/native_compositor.rs Shared BGRA scene compositor for native output consumers
  src/native_render.rs    Native text, image, SVG, and video decoding primitives
resources/                Sample Content shipped with the app
scripts/                  Node scripts: packaging (package-gpui*.mts, package-ffmpeg.mts, os-code-signing.mts,
                          updater-signing.mts), notices, smoke, maintenance checks
tests/                    Reference assets for import and release acceptance checks
integrations/             Companion and Stream Deck modules (their own npm packages)
vendor/                   Downloaded, Git-ignored ProPresenter reference schemas
docs/                     Design system, architecture, GPUI migration plan and logs
```

## Data and Storage

Presenter Orbit writes its own workspace format, separate from ProPresenter files. A workspace folder contains `workspace.json` plus folders such as `Libraries/`, `Playlists/`, `Media/`, and `Themes/`.

- The workspace schema version lives in `crates/orbit-host/src/workspace.rs`.
- The current workspace path and app settings live in the application data folder.
- ProPresenter files are read during explicit import only; after import the app runs from Presenter Orbit JSON.
- Media assets are copied into workspace-controlled asset folders and referenced from JSON indexes.

The development workspace format is currently strict version 1. Do not add migrations, legacy field aliases, or missing-file fallbacks unless backward compatibility is explicitly reintroduced as a product requirement. Older experimental workspaces may be recreated manually.

## Host Boundary and Security

The UI reaches native capabilities only through the host: `Host::invoke(method, args)` in `crates/orbit-host/src/host_api.rs` or a focused module in `crates/orbit-host/`. Add a new capability in three places:

1. A validated method in the host dispatcher or a focused host module, with malformed-payload coverage in a relevant end-to-end workflow.
2. A typed wrapper in `orbit-app` next to the feature that uses it.
3. Controller code that calls it from a background task, never on the UI thread.

Only the UI process writes workspace or settings files. The output process has an invoke allow-list that must not grow writers; record any addition in `docs/gpui-migration/README.md`.

## UI Patterns

- Views are GPUI `Render` implementations in `crates/orbit-app/src/<feature>/`. State lives in entities under `src/state/` and per-feature state types. Views observe the entities they need.
- Keep business logic, normalization, persistence glue, selection, sorting, and formatting in controllers and small pure helpers outside `Render`. Prefer an existing helper over a new one.
- Never block the UI thread: use `cx.background_spawn` for host calls and drop stale results.
- Keep user-facing strings in `crates/orbit-ui/locales/{en,nl}.arb` with an English `@key` description; Dutch uses "dia" for slide. Read them through the `Translator`.
- Colors, sizes, and fonts come from `orbit_ui::theme` (`cx.theme()`, `theme.colors`, `theme.metrics`). Literal colors are allowed only in the files listed in `scripts/check-architecture-boundaries.mts` (slide data, native output pixels, showcase).
- Do not `unwrap` or `expect` data that comes from outside the process, and do not hide invalid data behind a default.

## UI Design System

Presenter Orbit is live broadcast software: information density and unambiguous state beat decoration. `docs/design-system.md` is the source of truth for tokens, primitives, and layout rules.

- Use `Theme` colors for color and `Metrics` for spacing, type, radius, and heights. Literal values are allowed only on specialist surfaces (slide/output rendering, canvas, rulers, meters, color pickers, masks).
- Reuse primitives from `orbit_ui::controls` (`ActionButton`, `IconButton`, `Field` controls, `Dialog`, `TabList`, `ContextMenu`, `PanelHeader`, `SegmentedControl`, `Disclosure`, `EmptyState`). A new reusable control goes in `orbit-ui` with a showcase entry.
- No cards inside panels, decorative shadows, gradients, glows, or pill shapes (switches and slider thumbs excepted). No uppercase labels outside tab and column headers. No explanatory subtitles under headings.
- State colors are exclusive: `colors.live` means on air, `colors.selected` means selection, `colors.warning` means a real warning, `colors.danger` means clear or destructive. Never reuse one state's color for another.
- Skills in `.agents/skills/`: `ui-design` for new UI, `ui-review` for reviewing UI changes, `ui-refactor` for migrating an existing screen.

## Coding Style

- Keep changes minimal and focused on the requested behavior.
- Avoid unnecessary comments. Add comments only when they explain non-obvious constraints, platform quirks, or persistence compatibility.
- Follow the style of the surrounding Rust and `cargo fmt`. `orbit-app` and `orbit-ui` must stay at zero compiler warnings and zero clippy findings; do not raise `#[allow]` lists to pass.
- Do not modify unrelated files.

## Change Guardrails

- Implement only the requested behavior. Do not add fallback values, silent error recovery, compatibility aliases, or alternate execution paths unless the user explicitly asks for them or an existing documented contract requires them.
- Treat missing or invalid required data as a condition to handle explicitly at the owning boundary; do not conceal it with a guessed default.
- Do not weaken tests, coverage thresholds, lint rules, maintenance baselines, or build budgets merely to make checks pass. Fix the production code or add/repair meaningful tests instead.
- When splitting or moving code, preserve behavior and update focused tests for the affected paths. Do not rewrite tests to match an unintended implementation.

## Language Policy

Project documentation, code comments, hardcoded fallback/status text, and new source-code identifiers should be English.

## Testing and Verification

Before finishing code changes, run at least:

```bash
npm run check
npm run test:e2e
```

`npm run verify` runs the whole gate. Prefer internal E2E automation through GPUI's test platform; leave routine visual verification to the user. Use computer-use tools only when needed and requested. See `docs/e2e-testing.md` for suite boundaries. Output-window, media, audio, and NDI changes often need hands-on verification because they depend on local hardware/runtime capabilities. Tests must seed settings and use temp folders so they never reach the real data.

Do not leave known failing checks unexplained. If a check cannot be run because of local tooling, missing hardware, or environment limits, mention that explicitly.

## Generated Files

- Do not edit generated files by hand.
- Do not commit build output such as `dist/` or `target/`, or dependency folders such as `node_modules/`.

## Gotchas

- `vendor/ProPresenter7-Proto/` is downloaded external reference material, prepared by `npm install` or `npm run proto:prepare`. Update its pinned commit through `scripts/prepare-propresenter-proto.mts`, rather than editing cached schemas.
- `dist/` and `node_modules/` are generated and should normally be ignored.
- The app may have user data outside the repo. Do not assume a clean first-run state during manual testing, and never point a test at `com.presenterorbit.desktop` or `com.presenterorbit.gpui`.
- Some NDI behavior depends on whether the NDI runtime is installed on the host machine.
- The packaged Windows build needs NSIS (`makensis.exe` on `PATH`, `PRESENTER_ORBIT_MAKENSIS`, or `--makensis`).
- This repo may have a dirty working tree; do not revert unrelated user changes.
