# Maintenance guide

## Direction

The desktop app is Rust: GPUI views in `crates/orbit-app`, primitives in `crates/orbit-ui`, host services in `crates/orbit-host`. Node is used only for repository scripts (packaging and maintenance checks). Do not reintroduce a web renderer or a second UI runtime.

## Module boundaries

- Keep window lifecycle, output delivery, file dialogs, audio, and native host behavior in focused host modules (`crates/orbit-host/`) or the platform adapters in `crates/orbit-app/src/native_outputs/`.
- Normalize persisted and native settings in the host before returning them to the UI. Keep UI-only defaults and interaction state in the feature's state types.
- Keep network transport and batch failure isolation in the host. The UI may cache and apply returned data when that depends on live UI state.
- Keep views focused on rendering and user interaction. Persistence, selection rules, normalization, and host calls stay in controllers and pure helpers next to the feature.
- Extract a module when it has a separate lifecycle, can be tested without the UI, or becomes hard to review. Prefer a feature-local helper over a generic catch-all.
- Native scene normalization, framebuffer composition, and media decoding live in `native_compositor.rs` and `native_render.rs` of the host. New output consumers must use that shared compositor instead of introducing another renderer. External display and window outputs use `native_output.rs` to upload the compositor's BGRA frame to a wgpu swapchain; `output_playback.rs` owns the playback state.
- ProPresenter parsing, theme conversion, and workspace import live in the focused `propresenter_*` host modules.
- Keep cross-domain coordination in composition modules rather than letting individual panels mutate unrelated state.
- Only the UI process writes workspace and settings; the output process must not gain a writer.
- When adding a UI-to-host payload, validate its shape in the owning host method, cover rejected input in the relevant end-to-end workflow, and wrap it with a typed function in `orbit-app`.

## Workspace compatibility

`workspace.json` currently uses a strict development version 1 schema. There are no manifest migrations, legacy aliases, or missing-index fallbacks. Until a public release requires compatibility, incompatible experimental workspaces are recreated.

`npm run maintenance:check` enforces that release boundary (`scripts/check-workspace-compatibility.mts`). Raising `WORKSPACE_VERSION` above 1 is rejected unless the migration module, a fixture for every earlier version under `tests/fixtures/workspaces/`, and named backup/full-fixture/idempotency tests exist.

## User-facing strings

All visible labels, prompts, confirmations, empty states, and status messages belong in `crates/orbit-ui/locales/*.arb`. Every English message requires an `@key` metadata description; run `npm run i18n:metadata` to list or fill missing ones. Read strings through `orbit_ui::Translator`. `scripts/check-i18n.mts` checks key parity, placeholders, Dutch product terminology (Dutch says "dia", never "slide"), and that metadata debt (`i18nMissingMetadataCount` in `scripts/maintenance-baseline.json`) does not grow.

Persisted workspace names created as domain data, including default group, screen, and audio-channel names, remain stable English values. Translate surrounding UI labels, but never translate an existing stored name.

## Theme, text scale, and density

Colors and sizes come from `orbit_ui::theme` (`cx.theme().colors` and `.metrics`); see `docs/design-system.md`. Literal colors are reserved for the specialist files listed in `scripts/check-architecture-boundaries.mts`. When adding a panel, verify it under the light, red, and dark themes, with compact, comfortable, and spacious density, and at every text scale.

## Required verification

```bash
npm run check     # quick gate
npm run verify    # check plus the focused end-to-end suite
```

`npm run check` runs the toolchain, workflow-pin, color-literal, locale, version, and workspace-compatibility checks, Prettier on scripts, `cargo fmt --all --check`, and `scripts/check-clippy.mts`. The last runs `cargo clippy --workspace --all-targets`; `orbit-app` and `orbit-ui` must have no findings, and `orbit-host` has a known backlog that may not grow (`orbitHostClippyWarnings`: 362 library and 364 library-test warnings after retiring the internal suites; lower the baseline when it shrinks). `-D warnings` is not used because cargo applies it to every workspace crate, `orbit-host` included.

The focused end-to-end suite exercises complete operator workflows through GPUI's test platform with real host services, temporary workspace files, and real output child processes. Internal unit/component/timing and Node unit suites were removed by product request. See [E2E testing](e2e-testing.md) for coverage, isolation, and hardware limitations. Prefer internal E2E automation; the user handles routine visual checks.

The supported toolchain is Node.js 26.5.1 with npm 12.0.2. Keep `.node-version`, `.nvmrc`, `package.json`, and CI aligned when upgrading it. `engine-strict` makes installation fail on an unsupported version.

When an intentional change moves a baseline, update `scripts/maintenance-baseline.json` in the same focused commit and explain why in the commit message. Never raise a baseline to pass a check.

## Verification boundary

`npm start` runs the app. `npm run package:gpui:<platform>` builds the release path, and `scripts/gpui-smoke.mts` starts the unpacked package on a private data folder. The service-length burn-in and the operator checklist (`docs/propresenter-parity.md`) need real hardware and are not part of the gate.
