# Optional operating-system code signing

`scripts/package-gpui.mts` signs only when asked. Without signing secrets in the environment the artifacts are unsigned (Windows) or ad-hoc signed (macOS) and the script prints `UNSIGNED`. The Windows flow is verified; the macOS flow is written and untested. The reports are `package-report-<platform>-<arch>.json` in the output folder (default `dist/gpui/`).

## macOS without an Apple account

`npm run package:gpui:macos` on a Mac builds `Presenter Orbit.app` and a DMG. The default build uses Apple's ad-hoc signing identity (`-`) with the entitlements in `crates/orbit-app/packaging/macos`. It requires no Developer ID, Apple account, certificate export or notarization credentials, and the build subprocess excludes Apple certificate and notarization variables even if they are present in the parent environment. The updater key (`TAURI_SIGNING_PRIVATE_KEY`) is independent and only used by `--updater` builds.

Packaging checks the application seal with `codesign --verify --deep --strict`, verifies the application identifier and ad-hoc signature, then mounts the generated DMG read-only and checks its application payload. An ad-hoc signature verifies bundle integrity; it does not establish a trusted publisher or notarization. Gatekeeper can warn about or block applications downloaded from the Internet. No system security settings are changed.

## Optional Developer ID distribution

Use `npm run package:gpui:macos -- --os-signed` when Developer ID distribution is wanted. Set `APPLE_SIGNING_IDENTITY` to the full `Developer ID Application: ... (TEAMID)` identity and install the associated private key in the build host's keychain. Configure credentials through the build environment, never source files or command arguments containing secrets.

The script signs with the hardened runtime and a timestamp and verifies the expected authority and identifier. Notarization is not scripted: run `xcrun notarytool submit ... --wait` and `xcrun stapler staple` on the app and the DMG (Apple: [Notarizing macOS software before distribution](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution)), then repeat the Gatekeeper check. This mode never substitutes an ad-hoc build when the requirements fail.

## Optional Windows publisher signing

Ordinary Windows packaging is unsigned. Use `npm run package:gpui:windows -- --os-signed` to require publisher signing. The certificate and its private key must already be in the build host's Windows certificate store. Set:

- `PRESENTER_ORBIT_WINDOWS_CERTIFICATE_THUMBPRINT`: the certificate's 40-character hexadecimal SHA-1 thumbprint.
- `PRESENTER_ORBIT_WINDOWS_TIMESTAMP_URL`: the provider's HTTP(S) RFC 3161 timestamp endpoint.

The script signs the application executable before it is staged and the NSIS installer after it is built, with SHA-256 and RFC 3161 timestamping, using `TAURI_WINDOWS_SIGNTOOL_PATH` when set (the variable name is kept from the Tauri packaging) and otherwise the newest x64 SignTool of the installed Windows 10/11 SDK. Windows must then report a Valid Authenticode signature from exactly that certificate, with a timestamp, for the executable and the installer. The ZIP contains the verified executable. Hosted CI certificate provisioning depends on the certificate provider; the repository does not provision a signing identity or change certificate trust. This route does not claim SmartScreen reputation or a publisher-signed upgrade rehearsal.

`PRESENTER_ORBIT_REQUIRE_OS_SIGNING=true` is the environment equivalent of `--os-signed` for Windows and macOS. Its default is false. An explicit `--os-signed` flag on Linux is rejected.

## Updater signatures

`--updater` builds sign the installer, `.app.tar.gz` or AppImage with `scripts/updater-signing.mts` (Tauri-format minisign; see [app updates](app-updates.md)). Updater signatures do not prove OS publisher trust.

## Runtime prerequisites

The Windows installer downloads the WebView2 bootstrapper only if the runtime is missing (web fills use it). Windows and macOS packaging builds and bundles FFmpeg and ffprobe beside the application executable with their exact corresponding source archives and notices; the media tools are separate GPL-2.0-or-later subprocesses with static x264 and zlib, and no paid signing certificate is required for this route. See [bundled media tools](bundled-ffmpeg.md). Linux keeps its system FFmpeg dependency. NDI still requires its separate runtime for NDI workflows.

The gates exercise the stated operating-system baselines. They do not certify every older OS release, driver, audio interface, monitor arrangement, codec, NDI device or older experimental workspace upgrade. In-place replacement preserves the current strict-v1 workspace; no workspace migration is introduced. See [local runner boundaries](self-hosted-runners.md) for the packaged smoke and installed-package runners.
