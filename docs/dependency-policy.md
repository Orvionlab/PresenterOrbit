# Dependency and native-binary policy

Runtime dependencies require a focused pull request that records the purpose, license, upstream source, security review, and removal/upgrade owner. Only `MIT`, `ISC`, `BSD-2-Clause`, `BSD-3-Clause`, `Apache-2.0`, `0BSD`, `Unlicense`, and `CC-BY-4.0` are allowed without a written exception in the pull request.

`npm ci` verifies the JavaScript lockfile in CI. The Rust notice generator reads the locked dependencies for each build target, lists their exact source versions, and includes the license texts found in the published crates. [The upstream notice catalog](licenses/upstream-crate-notices.json) supplies texts verified against source revisions for crates that omit them. When updating a dependency, check the new version's notices and source link. Entries with no bundled text are listed explicitly in the generated notices for release review.

## NDI runtime

The Rust host dynamically loads the user-installed NDI Runtime. Any update to the NDI ABI bindings or loader must include:

1. Review of the ABI and platform loader changes.
2. The supported NDI Runtime versions and upstream release notes.
3. Sender/receiver validation on every affected release platform.
4. An updated entry in the release evidence and, when the risk changes, an ADR.

Presenter Orbit never downloads the NDI Runtime at application runtime.

## Current ProPresenter Remote codec

The modern Remote transport uses RustCrypto `aes` 0.8 and `ctr` 0.9
([block ciphers](https://github.com/RustCrypto/block-ciphers),
[stream ciphers](https://github.com/RustCrypto/stream-ciphers)), with `sha2` 0.10
([hashes](https://github.com/RustCrypto/hashes)), `base64` 0.22
([upstream](https://github.com/marshallpierce/rust-base64)) and `getrandom` 0.3
([upstream](https://github.com/rust-random/getrandom)). These crates use
MIT OR Apache-2.0 licenses. SHA-512/256, Base64 and system randomness already
exist in the resolved dependency graph; AES, CTR and their `cipher`/`inout`
support crates are added to avoid implementing cryptographic primitives locally.
UUID v5 support supplies deterministic public IDs for Orbit's non-UUID domain IDs.

Interoperability requires the observed SHA-512/256 password derivation and
AES-256 CTR64BE envelope with a fresh system-generated 16-byte IV per message.
CTR provides no message authentication tag. The server therefore still validates
every decrypted request and target, bounds messages and subscriptions, and rejects
wrong-password or malformed JSON before executing commands. It does not present
this transport as TLS or authenticated encryption. The native Remote protocol
maintainer owns dependency upgrades, security review and removal when this
compatibility transport is replaced. The focused change review must include the
independent codec fixture and native listener/command rejection tests.

## Current Remote discovery

The native Remote advertises `_proapiv1ws._tcp.local.` with the configured
Remote listener port using `mdns-sd` 0.21.6 (MIT OR Apache-2.0), from
https://github.com/keepsimple1/mdns-sd. The library owns DNS-SD record encoding,
multicast socket lifecycle and service withdrawal; Orbit disables its optional
async and logging features. The operator-supplied DiscoverProPresenter reference
confirms this service type and resolves its host and port. No TXT keys were
verified, so the advertisement has an empty TXT record and carries no password
or other credentials. The native Remote maintainer owns upgrades, security
review and removal if this discovery contract is replaced.

## Native audio backend

The Rust output path uses `rodio` 0.22.x (MIT OR Apache-2.0) with its `cpal`
playback backend (MIT OR Apache-2.0) and Symphonia format/codec support. This
is required for cross-platform device playback and local or cached remote
AAC/M4A/video-audio decoding. Production output has no WebView audio fallback;
unsupported paths, devices, or routing modes are explicit native errors. Symphonia's
MPL-2.0 license is the focused written exception for this backend. The owner
for upgrades and license/security review is the native playback maintainer;
revisit the exception when the audio backend is replaced or before its next
major release.

## Timecode input and timeline soundtrack

No new dependency is added. LTC capture uses the `cpal` input backend that
`rodio` already pulls in (the same crate and licenses as the playback path
above), MTC uses the `midir` 0.11.1 dependency from the MIDI section with its
own connection, and the LTC biphase-mark decoder and MTC parser are first-party
code. The soundtrack plays through the existing `rodio` and Symphonia stack. The
native audio maintainer owns upgrades and review of these paths.

## Desktop automatic startup

`orbit-app` uses `auto-launch` 0.5.x (MIT), from
https://github.com/zzzgydi/auto-launch. (The earlier Tauri app reached it through
`tauri-plugin-autostart`.) This provides native login startup registration on
Windows, macOS, and Linux. Registration changes only when the user changes the
saved startup preference, and failed settings writes restore the previous
registration. The native host maintainer owns upgrades, security review, and
removal if startup support is replaced.

## MIDI trigger input

The native host reads MIDI trigger input with `midir` 0.11.1 (MIT), from
https://github.com/Boddlnagg/midir. It uses WinMM on Windows (through the
`windows` crate already in the tree), CoreMIDI through `coremidi` (MIT) on
macOS, and ALSA through the `alsa` crate already used by the audio backend
on Linux. WKWebView and WebKitGTK provide no Web MIDI, so a native adapter
is required for MIDI bindings on every release platform. Input ports open
only while a trigger binding has a MIDI note; only note-on messages reach
the renderer. No JavaScript permissions are granted. The native host
maintainer owns upgrades and security review.

## Serial communication output

The native host uses pinned `serialport` 4.10.1 (MPL-2.0), from
https://github.com/serialport/serialport-rs, for Windows COM ports and macOS/Linux
TTY ports. This is a focused written license exception for cross-platform UART
configuration and bounded writes; maintaining a second unsafe platform adapter
inside Orbit is not justified. Default features are disabled, avoiding a new
Linux libudev requirement. The MPL-covered upstream files remain unmodified.
The native communication maintainer owns upgrades, security review, and removal
if the transport is replaced; revisit this exception before a major upgrade.

The focused review is limited to explicit operator-selected port enumeration,
opening, line configuration, and writing. Ports are opened only on an explicit
test or communication action and released afterwards. The WebView receives no
generic file/process capability. No device acknowledgement, persistent vendor
session, or automatic retry is inferred from a successful write. Windows
compilation and a Linux pseudo-terminal byte exchange verify the adapter; a
physical UART and macOS runtime review remain release acceptance tasks. No
crate advisory scan is claimed by this source review. Include this record in
the focused pull request before merging the new runtime dependency.

## External presentation control

The native host drives Microsoft PowerPoint on Windows through COM
automation with the `windows` crate already in the tree. It enables two
additional crate features, `Win32_System_Ole` (`GetActiveObject`) and
`Win32_System_Variant` (`VARIANT` and `IDispatch::Invoke`). No new crate is
added. On macOS the host spawns the system `osascript` with generated
AppleScript; the WebView never receives process access. The installed
PowerPoint or Keynote application is an optional user-provided runtime.

## Portable PDF rasterization

Stage chord-chart PDFs are rasterized by `hayro` 0.8.0 (MIT OR Apache-2.0), from https://github.com/LaurenzV/hayro, with its pure-Rust dependencies (`hayro-syntax`, `hayro-interpret`, `vello_cpu` and image/font decoders; all MIT, Apache-2.0 or BSD-3-Clause alternatives, except `zlib-rs` 0.6.8, which `hayro-syntax` selects as its `flate2` backend). `zlib-rs` is licensed Zlib, a permissive license outside the list above; this paragraph is the written exception, owned by the native host maintainer, to be revisited if `hayro` changes its inflate backend. `hayro` forbids unsafe code in its own crate and has no native binary. It replaces the Windows-only `Windows.Data.Pdf` path (the `Data_Pdf`, `Foundation`, `Storage`, `Storage_Streams` and `Win32_System_WinRT` `windows` features were removed), because macOS and Linux had no PDF decoder. Every platform runs the same decoder on a dedicated worker thread; encrypted documents are rejected and a decoder panic becomes an explicit import error. The native host maintainer owns upgrades and security review, and should re-run the chord-chart tests on each `hayro` release.

## ProPresenter archive import

The native host reads ProPresenter `.proBundle` and `.proPlaylist` archives
with `zip` 4.6 (MIT), from https://github.com/zip-rs/zip2, built with only its
`deflate-flate2` feature. Its `flate2` (MIT OR Apache-2.0) Rust backend is
already in the dependency tree, so no further crate enters the lockfile. Archives
are read only after explicit operator selection; entries that escape the
archive and oversized documents are rejected. The native import maintainer
owns upgrades and security review.

## PowerPoint import

The native host reads `.pptx` packages with `zip` (see above) and parses their
OOXML parts with `roxmltree` 0.21.1 (MIT OR Apache-2.0), from
https://github.com/RazrFalcon/roxmltree, a read-only DOM parser without unsafe
code. The version is the one already compiled for the SVG renderer, so no new
crate enters the lockfile. DTDs are rejected by its defaults, and parts larger
than 64 MiB are refused before parsing. The native import maintainer owns
upgrades and security review.

## ProPresenter WebSocket compatibility

The native host uses `tungstenite` 0.28.0 (MIT OR Apache-2.0), from
https://github.com/snapview/tungstenite-rs, with default features disabled and
only `handshake` enabled: a synchronous RFC 6455 implementation over the
standard `TcpStream`, with no async runtime, TLS or compression. It is required
for the ProPresenter Remote and Stage Display listener (see `docs/remote-api.md`);
hand-written framing, masking, fragmentation and UTF-8 validation would be
security-sensitive code on a network trust boundary. It adds four crates
(`tungstenite`, `sha1` 0.10.7 for the handshake accept key, `data-encoding` and
`utf-8`, all MIT or Apache-2.0); `http`, `httparse`, `bytes`, `rand`, `log` and
`thiserror` were already in the lockfile. Messages and frames are limited to
1 MiB and at most 8 connections are served. The native host maintainer owns
upgrades and security review.

## GPUI application

`crates/orbit-app` and `crates/orbit-ui` use `gpui-pre` and `gpui-pre-platform` at the exact version 0.3.11 under Apache-2.0. These packages are snapshots of [Zed](https://github.com/zed-industries/zed) commit `f816e1f`. Review and test the three GPUI dependencies together when upgrading.

`gpui-base` =0.9.0 (Apache-2.0, https://github.com/longbridge/gpui-kit) supplies unstyled behavior for controls: text editing with IME, focus, keyboard, and accessibility. It is the same project as `gpui-component` without that crate's visual layer, and only `orbit-ui` may use it.

`futures` 0.3 and `dirs` 6 (MIT OR Apache-2.0) and `sys-locale` 0.3 (MIT OR Apache-2.0) were already in the resolved graph. `futures` carries host events to the foreground, `dirs` locates application data, and `sys-locale` resolves the `system` language.

`crates/orbit-ui/assets/icons` holds the outline SVGs of the Tabler icons the application uses, copied from `@tabler/icons` 3.47.0 under the MIT license kept beside them.

### Native output windows in `orbit-app`

Phase 8 additionally uses the already-resolved `windows` 0.61.3 (MIT OR Apache-2.0,
https://github.com/microsoft/windows-rs) directly for Windows output windows, physical
monitor geometry, message pumps, and COM. It does not add a second window toolkit.

### GPUI web fill capture on Windows

`orbit-app` uses `wry` =0.55.1 (MIT OR Apache-2.0,
https://github.com/tauri-apps/wry) to retain HTML web fills after the Tauri
shell was removed. `webview2-com` 0.38.2 (MIT, https://github.com/wravery/webview2-rs) exposes
WebView2 PNG capture, and `url` 2.5.8 (MIT OR Apache-2.0,
https://github.com/servo/rust-url) names the existing host capture trait's URL type.
These packages already exist in Cargo.lock; this change adds direct dependency
edges without new package versions. System WebView2 remains an external runtime.

The adapter loads only URLs accepted by the host's HTTP(S) validator, has no IPC
handler or native browser bridge, isolates browser data under the GPUI data
directory, and owns WebView2 on a separate COM thread. PNG decoding stays on a
separate worker with a single pending frame per page. This is a source-level
boundary review; browser crash, long-running capture, and platform acceptance are
deferred to the combined migration verification. The native output maintainer
owns upgrades, security review, and removal when web fills or their browser engine
are replaced.

### GPUI macOS and Linux native adapters

macOS uses the already-resolved `objc2` 0.6, `objc2-app-kit`,
`objc2-foundation`, and `objc2-web-kit` 0.3.2, and `block2` 0.6
(MIT, https://github.com/madsmtm/objc2). Default framework features are disabled;
only the window, screen, notification, button, image, and snapshot APIs used by
the adapters are enabled. CoreGraphics/CoreFoundation provide native display and
cursor queries. AppKit window owners remain on the GPUI main thread, while a
browser helper owns WKWebView on its own main thread.

Linux output uses `winit` =0.30.13 (Apache-2.0,
https://github.com/rust-windowing/winit), with X11 and raw-window-handle 0.6 support
only, and `x11-dl` 2.21 (MIT, https://github.com/AltSysrq/x11-rs) for native cursor
queries and close-overlay painting. Winit is new to Cargo.lock; its platform
dependencies are target-gated and do not join the Windows runtime graph. Pure
Wayland physical placement is explicitly unsupported; XWayland is supported.

The Linux browser helper uses already-resolved `gtk` 0.18.2 (MIT,
https://github.com/gtk-rs/gtk3-rs), `webkit2gtk` =2.0.2 (MIT,
https://github.com/tauri-apps/webkit2gtk-rs), and `cairo-rs` 0.18.5 (MIT,
https://github.com/gtk-rs/gtk-rs-core), matching Wry. GTK/WebKitGTK remain system
libraries; no additional system browser binary is bundled. TIFF decoding uses
the existing `image` crate, already enabled by the shared host.

The helper accepts only validated HTTP(S) URLs and physical raster dimensions
within the host capture limit. The parent uses an isolated application-data
directory and launches only its own executable, without a shell. Length-prefixed
headers/requests are limited to 64 KiB and raw frames to 1920x1080 RGBA; no native
browser IPC bridge is exposed to pages. Latest-frame queues are bounded per
page, unused pages retire, helper errors reach the host, and shutdown kills and
reaps the owned process. The native output maintainer owns dependency upgrades,
security review, and removal. This is a source-boundary review; platform builds,
browser crash/endurance checks, and hardware acceptance remain deferred.

### Tray, launch at login, and GPUI window handles

`tray-icon` 0.24.2 (MIT OR Apache-2.0, https://github.com/tauri-apps/tray-icon, built with default features off, Windows and macOS only) draws the tray icon and its two-item menu. `auto-launch` 0.5.0 (MIT, https://github.com/zzzgydi/auto-launch) registers the executable as a login item. `raw-window-handle` 0.6.2 (MIT OR Apache-2.0 OR Zlib) reads the native handle of a GPUI window, which GPUI cannot hide or show itself, and `image` (already used by `orbit-host`, PNG decoding only) decodes the tray icon. All four, and `serde`, were already in `Cargo.lock` at these versions through Tauri, `tauri-plugin-autostart`, and `orbit-host`, so the lockfile gains no package. The Tauri app uses the same two crates through its own plugins. `orbit-app` registers its login item under its own name, so it does not replace the Tauri app's while both are installed. The Linux build has no tray yet and reports that explicitly. The GPUI foreground window's show/hide control remains in `native_window.rs`; output HWNDs are owned by the Phase 8 Win32 adapter. The native host maintainer owns upgrades and security review, and removal if the GPUI app moves to another tray implementation.
