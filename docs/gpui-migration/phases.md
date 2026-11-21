# GPUI migration phases

Each phase lists its scope and exit criteria. The README holds the agent prompt, the rules for every phase, and the status table. Port the matching TS tests in every feature phase.

Phases 5 through 15 also require a side-by-side comparison with the Tauri app, in dark and light theme, at compact and spacious density, and at 100 % and 200 % scale.

## Phase 0: Spikes and pin

Prove the risky assumptions in throwaway code under `spikes/`. Do not build product features. Delete `spikes/` once the decisions are recorded in the README decision log.

| Spike | Question | Pass criterion |
| --- | --- | --- |
| S1 | Does `gpui-pre =0.3.11` build and open a window on Windows, macOS, and Linux with the Rust version the project will use? Do the APIs in `architecture.md` exist in that version? | Window opens on all three, or the blocked platform is named. If the APIs are missing, evaluate a git rev. |
| S2 | Can `paint_image` carry previews? Render 1, 4, and 16 views at 30 fps, at 960×540 and 3840×2160, for 10 minutes, with `drop_image` on replacement. | Report UI frame time, CPU, and GPU memory. Memory must stay flat. Establish the correct moment for `drop_image` and the alpha format. |
| S3 | Can output windows with a wgpu surface coexist with GPUI, per `architecture.md`? Include HDR surface creation on Windows and display hot-unplug. | A 4K60 test pattern runs in a borderless always-on-top window while the GPUI window stays responsive, on all three OSes. If not, specify the separate-process alternative. |
| S4 | Does `gpui-component` fit? Check its input, multi-line editor, virtual list, table, and dock against pinned GPUI, including mapping our tokens onto its theme, dead keys, CJK IME, and rich-text runs. Check its license. | A decision per widget: use or build. A decision on the rich-text editor. |
| S5 | How does web-fill capture work without Tauri? Compare `wry` in-process against keeping the current capture code in a small helper process. | A captured frame from a real page on all three OSes, with the chosen approach recorded. |

**Exit:** pin chosen, five decisions logged, `architecture.md` corrected where the spikes disproved it.

## Phase 1: Host extraction

Create the Cargo workspace and move the host into `crates/orbit-host` without changing behavior.

**Read:** `src-tauri/src/main.rs`, `auxiliary_windows.rs`, `desktop_lifecycle.rs`, `web_capture.rs`, `remote.rs`, `app_updates.rs`.

Split this into sub-PRs. Each one keeps the Tauri app working:

1. Workspace root and a `Cargo.lock` move. `src-tauri` stays the only member with a binary.
2. Add `HostEvents`, and route all `emit` sites through it.
3. Replace `AppHandle`, `State`, and `tauri::Window` in service code with `Host` and a small window handle type. Break `AppState` out of `main.rs`.
4. Move `bridge_invoke`'s dispatch into `Host::invoke`, and move the frame commands likewise.
5. Move the modules into `crates/orbit-host`. Tauri-only code stays in `src-tauri`, behind traits where the host needs it (window creation, web capture, updater, tray, autostart).

**Exit:** `orbit-host` compiles with no `tauri` in its dependency tree (`cargo tree -p orbit-host`). All Rust tests pass. `npm run verify` and `npm run tauri:smoke:webview` pass. The Tauri bridge contract is unchanged.

## Phase 2: App skeleton

Create `crates/orbit-app` and `crates/orbit-ui`.

- Boot GPUI, construct `Host`, and deliver `HostEvents` to the foreground.
- Add a `host_call` helper that runs `Host::invoke` on the background executor.
- Load settings and the current workspace. Show the workspace title and library count in a plain window.
- Generate the Rust i18n table from `.arb` in `scripts/build-i18n.cts`, including placeholders and the language setting.
- Bundle fonts and Tabler icons, and add an `Icon` element.
- Add logging, and show startup errors as a failure surface as in `docs/architecture-dataflow.md`.

**Exit:** the app opens the same real workspace copy as Tauri on Windows, macOS, and Linux, and reading it changes nothing on disk. A failing workspace load shows an error rather than a blank window.

## Phase 3: Theme, UI size, HiDPI

Implement the Theme, UI size, and HiDPI sections of `architecture.md`.

- Add the token fixture dump and the Rust comparison test.
- Add `Theme`, `Metrics`, the color mixers, and the system appearance observer.
- Implement `FrameView` and its helper for device-pixel sizes. Use spike S2's lifecycle.
- Remove the host preview and still caps, with a memory bound.
- Add a small dev screen that shows every token.

**Exit:** the fixture test passes for all 12 combinations. Changing a setting updates all open windows without a restart. System appearance changes are followed. Moving a window across monitors of differing scale re-requests bitmaps at the new device size. Add a check, ported to the maintenance scripts, that rejects color literals in `orbit-app`.

## Phase 4: UI primitives and showcase

Port `src/renderer/react/ui` to `orbit-ui`, following `docs/design-system.md` and the decisions from S4.

`ActionButton`, `IconButton`, `Field` with `TextInput`, `Select`, `Textarea`, `NumberInput` (with hold-to-repeat stepper), `RangeInput`, `CheckboxField`, `ColorInput`; `ToggleSwitch`, `SegmentedControl`, `TabList`, `Disclosure`, `PanelHeader`, `EmptyState`, `Dialog`, `ContextMenu` with submenus, `ColorPicker`, `PickerSelect`, tooltip.

Add a virtualized list and tree row, and a showcase window as the counterpart of `?ui-showcase=1`.

**Exit:** every primitive has normal, hover, focus-visible, active, and disabled states in the showcase, with keyboard paths and accessibility roles and labels. Long Dutch labels fit. All themes, densities, and text scales are verified in the showcase.

## Phase 5: Shell and auxiliary windows

**Read:** `react/shell`, `features/shell`, `features/layout`, `react/windows`, `react/about`, `docs/auxiliary-windows.md`, `auxiliary_windows.rs`.

Main window layout, resizable panels with persisted sizes, module registry, status bar, menus, keyboard shortcut dispatch from `key-mapping-utils`, and custom title bar.

Settings, Screens, and About windows follow the documented behavior: created at startup and hidden, closing hides them, position retained, closing the main window destroys them. Their content comes in Phase 14.

Implement minimize to tray, start minimized, and launch at login.

**Exit:** the layout matches the Tauri app at 1120 through 1920 logical pixels. Window behavior matches `docs/auxiliary-windows.md`. Shortcuts run registered commands and do not fire while typing.

## Phase 6: State runtime and collections

**Read:** `src/renderer/core`, `types/runtime-state-slices.ts`, `ui/presenter-orbit-runtime.ts`, `features/app`, `features/collections`, `features/arrangements`, `react/collections`, `docs/architecture-dataflow.md`.

- Slice entities, the composition root, and the persistence lanes with revision checks and single replay.
- Library and playlist trees, search through the native index, presentation metadata, rename, reorder, copy, delete, theme assignment, arrangements, and drag and drop with insertion lines.
- Keep lists fast at 10,000 items.

**Exit:** every library and playlist mutation produces a workspace that Tauri reads back identically, and the reverse. A stale revision behaves as in TS. Ported tests pass.

## Phase 7: Frame views and program preview

**Read:** `react/output`, `features/output`, `native-output-preview-utils.ts`.

Program and stage previews, cue thumbnails with caching, and first-frame handling. Replace the `render_*_raw` IPC path with direct calls. Send `updateOutputScene` from the state runtime through the existing protocol.

**Exit:** previews are pixel-sharp at 200 %. A preview shows the same pixels as the output. Thumbnails stay smooth in a large presentation. Memory stays flat during an hour of video preview.

## Phase 8: Native output windows

**Read:** `native_output.rs`, `native_hdr*.rs`, the window functions around `create_display_native_output_window` in `main.rs`, `output_arrangement.rs`, `docs/operations-failover.md`.

Implement the output window host from S3 behind the Phase 1 trait, including windowed outputs, physical-pixel placement, always-on-top, HDR, and the per-display control. Also implement the web fill capture service from S5 (`wry` on its own `winit` thread behind the `WebCapture` trait), after proving it on macOS and Linux; see the README follow-ups.

**Exit:** an output on a second display matches Tauri's. It keeps running while the UI thread is deliberately stalled for 10 seconds. Display unplug and replug behave as today. NDI, key/fill, and HDR are tested on hardware or named as untested.

## Phase 9: Cue editor and live control

**Read:** `react/cue-editor`, `features/cue-editor`, `features/shell` as needed.

Slide grid and list modes, cue activation and clearing, build steps, groups, slide selection and reorder, group menus, live state colors, look layers, clear groups, and the arrangement view.

**Exit:** a complete service can be run from this screen alongside Phase 8: take slides, clear layers, run builds. The time from click to output frame is no worse than in Tauri. Live state matches the design system's color rules.

## Phase 10: Canvas core

Implement `SlideCanvas` as specified in `architecture.md`, with a read-only theme document at first and then with commits for bounds. No inspectors.

- `CanvasView`, hit testing, and the gesture state machine, all with unit tests ported from `theme-geometry` and the drag controller.
- Composed bitmap, three-plane gesture path, stale-result handling, selection frame, handles, rulers, zoom menu, fit, pan, viewport memory, status bar readout.
- Measure gesture performance with a heavy slide (video background, 20 objects, feathered shapes, 4K) at 100 % and 200 %.

**Exit:** dragging holds the display refresh rate with the heavy slide. A release commits exactly one mutation with the same integer result as the TS code for the same input. Handles keep a constant on-screen size at all zooms and scales. The canvas matches program output pixel for pixel when compared with the same slide at the same size. Decide on viewport cropping using the measurements.

## Phase 11: Theme editor

**Read:** `features/theme-editor` (86 files), `react/theme-editor`.

Split this into PRs per inspector:

1. Theme sidebar, slide list, tools, object actions, creation, ordering, lock, and hide.
2. Pen tool and path editing.
3. Text inspector: fonts, paragraph, tab stops, lists, linked text, timers, scrolling, effects.
4. Shape inspector: fills, gradients, strokes, feathering, effects.
5. Media inspector, crop, picker dialogs, thumbnails.
6. Builds and transitions.

Do not port the CSS rendering helpers. Do port the logic that also determines behavior. Inspector edits update the canvas through the normal path, without per-keystroke disk writes, matching the debounce in TS.

**Exit:** every inspector field in the React editor exists and has the same effect on the saved JSON, tested with fixtures from the TS implementation. Undo matches.

## Phase 12: Slide, prop, mask, and stage editors

**Read:** `slide-editor-controller.ts`, `react/cue-editor/SlideEditorControls.tsx`, `react/mask-editor`, `features/output/screen-canvas.ts`, `react/settings/ScreenCanvasFields.tsx`, `slide_text_edit.rs`.

Add `CanvasDocument` implementations, and the rich-text editor from S4 with runs, IME, plain-text paste, and commit on blur. Include the mask editor stage, prop transitions, stage layout editor, and screen canvas.

**Exit:** text edits with mixed runs round-trip identically with the Tauri app. IME input works on Windows and macOS. All documents share `SlideCanvas`, with no second canvas implementation.

## Phase 13: Media bin and audio

**Read:** `react/media-bin`, `features/media-bin`, `native_audio.rs`, `native_waveform.rs`.

Media libraries, smart libraries, grid and list, thumbnails, drag-in of files, marker and thumbnail editors, transport, effect chains, audio bin, waveforms, and the preview audio and meter path that replaces Web Audio.

**Exit:** the media workflows match, including drops from the file manager. Scrolling a 2,000-item bin stays smooth. Preview audio does not disturb program audio.

## Phase 14: Settings, screens, and devices

**Read:** `react/settings`, `features/settings`, `react/output`, `features/ndi`, `features/capture`, `features/network-link`, `react/network-link`.

All settings panels, audio routing, screen configuration, looks, NDI, capture, cameras, video inputs, timecode, network link, group and shortcut editors. Mutations stay semantic.

**Exit:** every panel works and sends the same semantic intents. Theme, accent, text scale, density, and language changes apply live across windows. Hardware not available is listed as untested.

## Phase 15: Automation, macros, and remaining features

**Read:** `react/automation`, `features/app` (action runner), `react/bibles`, `features/bibles`, `react/presentation-export`, `features/external-presentation`, `features/remote`, `react/portable-bundle`.

Action lists, macros, messages, timers, calendar, tickers, Bible browser, Planning Center, imports and exports, handouts, spellcheck, portable bundles, external presentations, startup actions, due-event handling.

**Exit:** a repository search finds no user-reachable feature of the React app without a GPUI counterpart. Produce the list by walking `src/shared/presenter-orbit.ts` and marking each bridge method as used. Unused methods require an explanation.

## Phase 16: Packaging and cutover

**Status:** part 1 done (see `phase-16.md`): items 1 (Windows built, macOS and Linux written and untested), 2 and 3 (workflows written, not run). Item 5 (deletion of Tauri and the React/Vite/npm build, docs rewritten, one gate) is done; the last commit that contains Tauri is `64121abe`. Item 4 and the human acceptance listed in `phase-16.md` are open.

**Read:** `docs/tauri-migration.md` (now a short historical note), `docs/package-signing.md`, `docs/app-updates.md`, `docs/bundled-ffmpeg.md`, `scripts/package-tauri.mts`, `.github/workflows/`.

1. Package NSIS, portable ZIP, `.app`, `.dmg`, AppImage, and `.deb`, including FFmpeg, signing, and notices.
2. Updater: existing installs must be able to update to the GPUI build. Keep the update signature format.
3. Replace the smoke tests and CI jobs, including the burn-in and package acceptance workflows.
4. Run a service-length burn-in on real hardware. Go through `docs/propresenter-parity.md` and the operator release checklist.
5. Only after explicit approval from the maintainer: delete `src-tauri`, `src/renderer`, the Vite and npm build setup that no longer has consumers, and Tauri dependencies. Rewrite `AGENTS.md`, `docs/design-system.md`, `docs/architecture-dataflow.md`, and `docs/tauri-migration.md` for GPUI.

**Exit:** a clean install and an update from the last Tauri release both work on all three OSes, with existing workspaces and settings intact. The burn-in passes without leaks. Everything remains recoverable from git history.
