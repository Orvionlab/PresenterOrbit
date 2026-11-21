# GPUI target architecture

## Crates

```text
Cargo.toml            Workspace root
crates/orbit-host/    Everything in src-tauri/src today, without Tauri types
crates/orbit-ui/      Theme, tokens, primitives, icons, i18n lookup
crates/orbit-app/     GPUI binary: state, feature views, canvas
(src-tauri/ was the thin Tauri adapter; deleted in Phase 16, last commit 64121abe)
```

`orbit-host` has no GPUI dependency and `orbit-ui` has no host dependency. Only `orbit-app` sees both.

### Host boundary

The host keeps its `Value`-based dispatcher during the migration, because it already validates every payload:

```rust
impl Host {
    pub fn invoke(&self, method: &str, args: Vec<Value>) -> Result<Value, String>;
}

pub trait HostEvents: Send + Sync + 'static {
    fn emit(&self, event: &str, payload: Value) -> Result<(), String>;
    fn emit_to(&self, label: &str, event: &str, payload: Value) -> Result<(), String>;
}
```

`Host::invoke` and the raw frame renderers block, so a shell calls them off its UI thread. Windows, tray, autostart, and updates reach the host through the `WindowSystem`, `DesktopIntegration`, and `Updater` traits, and web fill capture through `WebCapture`. Tauri's adapter forwards `emit` and `emit_to` to the WebView. GPUI's adapter sends events over a channel to a foreground task. Typed Rust wrappers are added per feature as `orbit-app` needs them, using the types in `src/shared/` as the contract. Do not replace the JSON values inside the host during this migration.

### UI state

- `src/renderer/types/runtime-state-slices.ts` defines the slices. Each slice becomes one `Entity<...>`, so a change only re-renders views that observe that slice.
- Controllers in `src/renderer/features/` become plain Rust modules that operate on those entities. Pure helpers stay pure.
- Views render from state and call controllers. Business rules stay out of `Render` implementations, as with React today.
- Preserve the optimistic-then-canonical pattern from `docs/architecture-dataflow.md`, including revision checks and the single replay of stable-ID intents.

## Theme and colors

Source: `src/renderer/styles/tokens.css`, `app-theme-utils.ts`, `appearance-settings-utils.ts`.

```rust
pub struct Theme { pub colors: Colors, pub metrics: Metrics, pub appearance: Appearance }

impl Theme {
    pub fn resolve(
        settings: &AppearanceSettings, // theme, accent, text scale, density
        system: WindowAppearance,
    ) -> Self;
}
```

`Theme` is a GPUI `Global`, read with `cx.theme()` and rebuilt only when settings or the system appearance change.

**Colors are tables plus formulas, not 16 handwritten palettes.** The CSS has behavior that must be reproduced exactly:

- **Cascade:** `red` overrides only part of the dark palette. `light` overrides a different part. Build each theme from the dark base.
- **Aliases:** `live` is always `warning`, `selected` is always `accent_selected`, `canvas_bg` is `stage_bg`.
- **Accents are computed:** blue is the theme's own accent. Red, green, and purple derive `accent`, `hover`, `soft`, and `selected` with `color-mix`. Light and dark use different percentages. `red` as a theme uses the dark percentages.
- **Purple mixes in OKLCH,** everything else in sRGB. Add one small `mix_srgb` and one `mix_oklch` to `orbit-ui`.
- **System** resolves through `window.appearance()` and follows changes through `observe_window_appearance`.

To add a theme or accent later, add one table entry. Nothing else may change.

**Verification:** dump the computed values of every `--app-*` token from the Tauri app for all 12 combinations (dark, light, red × 4 accents) to a checked-in JSON fixture. A Rust test compares `Theme::resolve` against it, within ±1 per channel for rounding.

State colors stay exclusive as defined in `docs/design-system.md`. User-authored group colors are data and are not part of the theme.

## UI size

"UI size" is two independent settings. Do not merge them.

| Setting | Values | Applies to |
| --- | --- | --- |
| `uiDensity` | compact 0.8, comfortable 1.0, spacious 1.2 | spacing, row and control heights, padding, header and tab heights |
| `textScale` | small 0.92, default 1.0, large 1.1, xlarge 1.2 | font sizes only |

Radii, border widths, and icon stroke do not scale.

```rust
pub struct Metrics { pub row_height: Pixels, pub control_height: Pixels, pub space_sm: Pixels, pub font_md: Pixels, /* ... */ }
```

`Metrics` is computed once in `Theme::resolve`. Views read `cx.theme().metrics.row_height` and never multiply by a scale themselves. Do not use `set_rem_size` as a shortcut, because it would couple density and text scale.

Every layout must work at all density and text-scale combinations, at window widths from 1120 to 1920 logical pixels.

## HiDPI

### What the app does today

The WebView handles logical layout, but bitmaps are limited:

- `getPreviewRequestSize` caps previews at 960×540 and `NativeOutputPreview` caps the pixel ratio at 2, so program previews are soft on 4K screens.
- `render_slide_still` rejects anything over 960 px per side.
- Thumbnails use no pixel ratio at all.

### Rules in GPUI

GPUI handles per-window scale, including `WM_DPICHANGED`, and snaps quads and borders to device pixels. Vector UI is therefore correct by default. Three rules cover the rest:

1. **Three coordinate spaces, never mixed.** Slide units (authored), `Pixels` (logical layout), device pixels (bitmaps). Convert only in named helpers.
2. **Every bitmap is requested at device size:** `bounds.size × window.scale_factor()`. Results are capped by the source resolution, because there is no value in upscaling a 1080p output. A size or scale change requests a new bitmap and keeps showing the old one until it arrives.
3. **Physical display geometry stays in the host.** Output windows use physical pixels from the existing display code. Do not derive them from GPUI display bounds, which are logical.

Remove the 960×540 preview cap and raise the still limit in `orbit-host`, with an upper bound that protects memory. Update its tests deliberately. This changes output previews only, not program output.

**Test matrix:** 100, 125, 150, 175, 200, and 300 %, plus dragging a window between monitors with different scales. Check for hairlines, soft previews, and misaligned handles.

## Frames in the UI

One `FrameView` element serves previews, thumbnails, and canvas planes.

```text
background thread: render BGRA -> Arc<RenderImage>
foreground: swap the current image, cx.notify()
paint: window.paint_image(bounds, image_bounds, corners, image, 0, false)
window.on_next_frame, from the paint that showed the replacement: cx.drop_image(old, Some(window))
```

- Frames are latest-wins, with at most one request in flight per view, as `NativeOutputPreview` does today.
- Frames with an unchanged `visual_key` do not create a new image.
- Hidden views stop requesting. Dynamic content refreshes at 30 fps or less, as today.
- The compositor emits straight alpha BGRA. GPUI's atlas and blend state expect exactly that (spike S2, DirectX), so there is no conversion.
- Pace 30 fps updates with a short poll against a deadline. A 33 ms sleep fires after about 50 ms on Windows.

## Output windows

Program output must not become GPUI windows. It needs its own wgpu surface for layered uploads and HDR (`native_hdr*`), it must keep working when the UI stalls, and GPUI's own swapchain occupies a GPUI window's surface. `native_output.rs` stays, and only its `tauri::Window` changes to a small handle type implementing `HasWindowHandle` and `HasDisplayHandle`.

GPUI owns the main thread's event loop, so windows are created outside of it:

| Platform | Phase 8 implementation; final platform acceptance deferred |
| --- | --- |
| Windows | A dedicated Win32 message pump, with cached HWNDs and physical geometry. This preserves S3's isolation without adding a second winit loop. |
| Linux | One dedicated winit X11 event loop (`with_any_thread`), with cached raw handles. Physical placement requires X11/XWayland; pure Wayland is explicitly unsupported. |
| macOS | `NSWindow` on the main thread through the `objc2` crates already in use. AppKit delivers its events through GPUI's run loop. |

Required window behavior is what `main.rs` does today: borderless, positioned in physical pixels, always on top, unfocusable, hidden from the taskbar, plus the windowed output mode with a title bar. The 38×38 control WebView on each display output becomes a small GPUI popup window or a native alternative.

### Deviations from the original plan

The original plan was two `winit` loops, one for output windows and one for web capture. Three decisions departed from the plan; the README decision log holds the evidence.

- **Windows output windows are direct Win32, not winit.** `winit` allows one event loop per process, and S3 needed cached raw handles because winit returns `HandleError::Unavailable` off its own thread. Web capture needs its own loop on Windows, so a second winit loop was impossible. `native_outputs/win32.rs` runs one message pump on its own thread, registers one window class, caches the HWND and physical geometry at creation, and sets `WS_EX_NOACTIVATE` and `WS_EX_TOPMOST` directly, which winit cannot do. Display changes arrive as `WM_DISPLAYCHANGE` and are coalesced by `native_outputs/logic.rs`. Linux keeps winit, because it is the only loop on that platform.
- **macOS and Linux web capture run in a helper process** (`--web-fill-helper`, `capture_process.rs`). WKWebView must live on the AppKit main thread, which GPUI owns, and WebKitGTK needs one GTK owner that is not the output window loop. The helper has its own main loop, and frames cross a length-prefixed pipe (`capture_protocol.rs`), which costs one process and a frame copy. Windows does not need it: S5 showed `wry` on its own thread works there.
- **Rich text uses `EditorState`.** `TextareaState` has no highlighter, which per-run bold, italic, underline, strikethrough, and color need. `EditorState` exposes the `InputHighlighter` seam, so `orbit_ui::controls::RichText` uses that input and IME engine with code features off, and keeps its own run model and undo stack. Font family and size are uniform in the panel because `HighlightStyle` has no per-run family or size.

Two Phase 8 changes are in the shared host, so they also affect the Tauri app on Windows:

- `native_output.rs` creates the fallback `wgpu::Instance` with `Backends::DX12` on Windows. `native_hdr_platform.rs` reads the DXGI swap chain through `surface.as_hal::<Dx12>()` to call `SetColorSpace1` for Rec. 2100 PQ. With the default instance wgpu may pick Vulkan for the surface, where `as_hal::<Dx12>()` is `None` and HDR fails with "HDR output requires a DirectX 12 swap chain". S3 measured 60 fps with the same p95 on Vulkan and DX12, so SDR output loses nothing. The change applies only when no shared GPU context already supplies an instance.
- `keep_display_window_on_top` and `keep_display_control_on_top` in `orbit-host/src/lib.rs` add `SWP_ASYNCWINDOWPOS`. They run on a host worker that holds GPU locks. `SetWindowPos` on a window owned by another thread is a synchronous cross-thread send, so it waits for that thread to answer, and the output window thread can be inside a resize callback that wants the same GPU lock, which is a deadlock. The async flag queues the request and returns. The cost is that topmost is applied a moment later and a failure to apply it is no longer reported.

Neither change was reverted, and no evidence was found that either is wrong. The hardware checks that remain are in the README follow-ups.

### Output process

Program output runs in `orbit-app --output-process --data-dir <ui data>/output-process`, so a crash of the UI process leaves the last accepted picture on the output windows and NDI keeps sending. The output process runs a second `Host` with the windows, NDI senders, audio sinks, `output_playback`, and the program compositor. The UI process runs a `Controller` host that keeps the model: authoring, workspace and settings (only it writes them), operator edits, automation, remote control, and the previews.

- **Transport:** loopback TCP, length-framed JSON (`native_outputs/framing.rs`, 64 MiB limit). The output process binds port 0 and writes `endpoint.json` (port, token, pid) atomically; an exclusive lock on `output.lock` allows one output process per data directory (exit code 3 for the loser). The first message is `Hello` with the token and protocol version. One controller at a time: a second one is answered `Busy`.
- **Calls:** an allow-list of ten host methods (`updateOutputScene`, window and display calls, and three getters) and three forwarded events (`output:displays-changed`, `output:display-error`, `output-closed`). Everything else is refused or dropped. Events raised while nobody is attached are kept (64 at most) and delivered at the next attach.
- **Scene flow:** one UI sync sends to the output process first, then feeds the accepted (rebased) snapshot to the local host. A restarted UI has a new session, so its first full gets `cacheMiss` with what is on air, and the existing rebase adopts the native keys. A scene whose `screenConfig` differs from the last applied one makes the output process apply the routing itself. A change to a field the host refuses in a patch is sent as a full snapshot.
- **Lifecycle:** Waiting (30 s), Controlled, Orphaned (30 minutes, or at once when no window, display output, or NDI sender remains), Exiting. A normal quit sends `Shutdown`, waits 3 s, and kills a child it started. A crash of the output process is reported through `output:display-error`, restarted after 0.5, 1, 2, 5, then 10 s pauses, and the scene is resent.
- **Windows spawn:** `CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS | CREATE_BREAKAWAY_FROM_JOB`, falling back without breakaway when the job refuses it. The child survives a hard kill of the UI; a process tree kill (`taskkill /T`) also kills it. The window thread, window procedure, and tickers catch panics. The process sets per-monitor DPI awareness v2 (or accepts that the executable already has it).
- **Not done:** automation while the UI is down; the timeline soundtrack still plays from the UI process; no heartbeat or hot standby; the macOS main-thread AppKit loop of the output process and Linux were not compiled.

Test seams: `--inert` (no monitors, no windows) and `--grace-seconds` are hidden flags. The end-to-end test starts the real binary from a stand-in controller process.

### Test seams of the output adapters

`native_outputs` is replaced by an inert adapter under `cfg(test)`, so no test opens a window on the machine's displays. Decisions that need no operating-system call live in modules that also compile in tests: `logic.rs` (window placement and style choice, rectangle sizes, and the display-change coalescer) and `capture_protocol.rs` (packet framing, pixel conversion, and the header and frame checks of the helper pipe). Window creation, message handling, monitor enumeration, WebView creation, and the macOS and Linux files need a real display and browser.

## Web fill capture

The compositor keeps calling `web_capture::frame(url, w, h)`. The shell installs a `WebCapture` that is no longer Tauri-based. Spike S5 settled it on Windows:

```text
compositor threads -- frame(url, size) --> session map (latest frame per page) <-- decode thread (PNG to image)
                                                   ^
                          Windows web thread: Win32 pump, hidden windows, Wry WebViews, CapturePreview
```

- S5 measured 16 to 18 fps during a ten-second GPUI stall using an independent web thread. The Windows implementation uses a native Win32 pump rather than winit, preserving the separation between GPUI, output windows, WebViews, and PNG decoding. The implementation itself still needs the deferred acceptance run.
- The platform capture APIs and alpha conversions from `src-tauri/src/web_capture.rs` are retained. Wry owns hidden native parents, and platform extension traits expose WebView2, WKWebView, or WebKitGTK. Captured data is decoded away from the browser loop, with one pending capture per page.
- The earlier belief that WebView2 creation must stay off the event-loop thread was about Tauri's main thread. On a thread of our own it works and takes 130 to 630 ms (the first WebView also creates the environment).
- Phase 8 selects the S5 helper architecture for macOS/Linux: the current executable runs `--web-fill-helper` with its own AppKit or GTK main loop, and sends length-prefixed physical RGBA frames to a background reader. This keeps WKWebView independent of the GPUI main thread and GTK ownership isolated from output windows and file dialogs. A ten-second stall case is included in the new acceptance program; no new platform performance result is claimed. The helper adds a process and frame copies, with a 64 KiB request/header limit and a 1920x1080 RGBA frame limit.
- Source inspection found that winit permits only one event loop per process. Linux therefore reserves it for output windows; capture helpers use the platform browser loop directly. The parent reports helper failure and kills/reaps the process on shutdown. Automatic crash restart, heartbeat policy, endurance, and hardware behavior require the deferred acceptance/product decisions; no silent fallback is introduced.

## Canvas editor

This is the hardest screen. The React version draws layers with a second, CSS-based renderer (`theme-shape-renderer`, `theme-text`) that only approximates the Rust compositor. Effects it cannot draw already go through `useNativeStill`.

### Decision: the compositor draws the content, GPUI draws the tools

```text
Top     Text-edit caret and selection     GPUI, later phase
        Selection frame, handles, guides  GPUI quads and paths, constant logical size
        Pen preview, crop and feather UI  GPUI paths
        Rulers                            GPUI
Bottom  Slide content                     Compositor bitmaps through FrameView
```

Rejected alternatives:

- **Rebuild layers as GPUI elements.** It repeats the mismatch of today's CSS renderer. GPUI also lacks feathering, background blur and invert, cut-out text, and centered strokes, and would use a different text engine than output.
- **A custom wgpu or shader canvas inside GPUI.** Not possible across platforms, see the facts in the README.

The result is true WYSIWYG: the canvas shows the pixels that output will show, and the CSS renderer is not ported at all.

### Keeping drags smooth

Re-rasterizing a full scene on every mouse move is too slow, and per-object sprites would break background-dependent effects and z-order. Use three planes during a gesture:

```text
Idle:           [ composed ]                   one bitmap of the whole slide
Gesture start:  [ below ] [ active ] [ above ] slide split at the active layer
Move:           paint [ active ] at an offset, no rasterization
Resize:         stretch [ active ], rasterize at the new bounds latest-wins
Pointer up:     send one mutation, keep showing the planes until the new composed bitmap is ready
```

Phase 10 extracts the planes from the compositor's existing native layer cache, partitioned at the active layer, instead of rendering filtered slide copies. This preserves its ordering and backdrop classification without a second rendering path. Rendering starts on pointer-down. Until the planes exist, the gesture shows the old bitmap and moves only the selection frame. If the active layer or a layer above it depends on the backdrop, the complete scene is re-rasterized in place through the same latest-wins worker.

The canvas owns one `NativeCompositor::new_static_preview` on a background thread and a generation counter that discards stale results.

**Raster size:** `slide size × zoom × scale_factor`, capped at twice the slide size per side. A 1920×1080 slide at 200 % on a 200 % display would otherwise need 132 MB per plane. Add viewport cropping only if the Phase 10 measurements call for it.

### Structure

```rust
struct CanvasView { zoom: f32, pan: Point<Pixels>, scale_factor: f32 } // all conversions, pure, tested
enum Gesture { Idle, PendingMove { .. }, Moving { .. }, Resizing { handle, .. }, Panning { .. }, Pen { .. }, Crop { .. }, Feather { .. } }
struct SlideCanvas { document: Rc<dyn CanvasDocument>, view: CanvasView, gesture: Gesture, planes: Planes }
```

`SlideCanvas` owns one custom `Surface` element with its own hit testing, using `capture_pointer` so a gesture continues outside the bounds. `CanvasDocument` is what differs between the theme, slide, prop, and mask editors: read layers, commit bounds, commit a new layer. `CanvasView`, hit testing, and gesture transitions run without a window, so they get ordinary unit tests.

### Behavior to reproduce

From `theme-object-drag-controller.ts`, `theme-geometry.ts`, `ThemeCanvasObjects.tsx`, `theme-tool-controller.ts`, `theme-viewport-*.ts`, `theme-keyboard-controller.ts`:

- The topmost visible layer under the pointer wins, tested against its unrotated bounds. Handles are tested first, in logical pixels.
- Eight resize handles. Minimum size is 8 slide units. Coordinates round to integers after applying the delta from the start point.
- A move starts after 3 logical pixels. A press without movement only selects.
- Locked objects select, show the translated status, and do not move.
- No store write happens during a gesture. Commit once on release through the existing mutation.
- Path layers scale their points and control points on resize and convert to local coordinates.
- Pen tool, including clicks on top of existing objects, Escape, and its node preview.
- Zoom from 0.1 to 2 with the existing menu steps, custom zoom, fit, per-document viewport memory, pan padding, and scroll restoration after an edit.
- Delete, Escape, undo/redo, shape shortcuts, and the rule that shortcuts do not fire while typing.

Single selection, no rotation handle, and no snapping are the current behavior. Keep them for parity and do not add multi-select "for later".

### Live content on the canvas

- **Video and animated images:** first frame with `freeze_video_playback`. This differs from the `<video>` elements in today's editor. If playback proves essential, add a refresh loop on the `FrameView` path.
- **Clocks and timers:** re-render on a one-second tick, only when the slide contains them (`hasThemeSystemClock`, `hasThemeCountdownTimer`).
- **Web fills:** a still from the web capture service instead of an interactive iframe.

### Text editing

Keep the current model first: rich text in `SlideEditorControls` (a `contentEditable` panel below the canvas) with runs for bold, italic, underline, strikethrough, font, size, and color; plain-text paste; commit on blur.

Phase 12 adds `orbit_ui::controls::RichText`. The pinned `gpui-base` exposes its highlighter on `EditorState`, so the wrapper uses that shared input/IME engine with all code-editing features disabled. It paints per-run bold, italic, underline, strikethrough, and color. Font family and size are uniform in the editing panel; their per-run values remain in the document and the native compositor draws the actual slide typography. A byte-aligned run model preserves formatting during edits, while the wrapper owns styled undo/redo snapshots, selection restoration, and commit on blur. Plain-text paste stays in Base's existing clipboard path. Real Windows/macOS IME proof is deferred to the combined acceptance pass.

On-canvas editing is a post-parity feature. When it is built, caret and selection geometry must come from `native_text_layout.rs` (cosmic-text), not from GPUI's text system, which on Windows uses DirectWrite and has different metrics. The compositor keeps drawing the glyphs.

## Replacing web-only features

| Today | GPUI app |
| --- | --- |
| `<video>` in 13 components | First frame through `FrameView`. Playback through the existing FFmpeg decoders where the workflow needs it. |
| `<audio>`, Web Audio analyser | Host decoders, native meter data |
| `mediaDevices` camera preview | Existing `render_camera_input_frame` |
| `contentEditable` | Rich-text editor above |
| Hidden WebViews in `web_capture.rs` | Wry on a dedicated Win32 thread on Windows; platform-loop capture helpers on macOS/Linux, see "Web fill capture" |
| CSS grid, `@media`, `@container` | Taffy layout and explicit width breakpoints |
| Tabler icons from `@tabler/icons-react` | The SVG files, bundled in `orbit-ui` |
| `scripts/build-i18n.cts` output | Also generates a Rust table from the same `.arb` files |
| Tauri tray, autostart, updater | Focused crates, chosen in Phases 2 and 16, under the dependency policy |
| `rfd`, `arboard`, `open` | Unchanged |

`integrations/` (Companion, Stream Deck) use the Remote API and are unaffected.

## Known risks

| Risk | Mitigation |
| --- | --- |
| GPUI is pre-1.0, and `gpui-pre` and `gpui-base` have one publisher | Exact pins, `orbit-ui` wraps the widgets, git-rev fallback |
| Atlas churn from 30 fps previews | Spike S2 measures it before anything depends on it |
| Output windows coexisting with GPUI's event loop | Proven on Windows by S3. Program output now runs in its own process (see Output process). macOS and Linux remain unproven. |
| Web fill WebViews next to GPUI | Proven on Windows by S5 (`wry` on its own `winit` thread). macOS (main-thread WKWebView) and Linux (GTK thread) remain open, with a helper process as the fallback. |
| A renderer of this size takes long to rewrite | Tauri keeps shipping until parity, and phases can run in parallel |
| Accessibility regresses | GPUI has AccessKit support. `orbit-ui` primitives set roles and labels from Phase 4. |
| Rendering differences are hard to test automatically | `Window::render_to_image` exists on Windows. Use it for primitive snapshots if it proves stable. |
