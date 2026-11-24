# App updates

Release builds check for updates at startup. General settings also offers a manual check and a download/install button, with status, release notes and download progress. Installation is explicit because restarting interrupts presentation outputs. The updater (`crates/orbit-app/src/updates/`) verifies each download against the public signing key embedded in the installed build. Windows starts the NSIS installer with `/P /UPDATE /R`, which shows its progress UI and relaunches the app. macOS and Linux restart after installation (untested).

The default development/build configuration has no endpoint or key. It reports that updates are unconfigured. No invented hosting URL or disposable signing key is shipped.

## Configure signed releases

The updater verifies minisign signatures. Use the signing key that matches the public key in the installed app. A new key pair can be generated with `cargo tauri signer generate -w <path>` or `minisign -G`. The packaging script expects the Tauri format, which is the password-protected secret key file encoded as base64.

Keep the private key and its password backed up securely, outside this repository. Keep using the same key for later releases. The `.pub` file is public. Before building, set:

```powershell
$env:PRESENTER_ORBIT_UPDATE_BASE_URL = 'https://updates.example.com'
$env:PRESENTER_ORBIT_UPDATE_PUBLIC_KEY = Get-Content "$env:USERPROFILE/.tauri/presenter-orbit.key.pub" -Raw
$env:TAURI_SIGNING_PRIVATE_KEY = Get-Content "$env:USERPROFILE/.tauri/presenter-orbit.key" -Raw
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = 'your-key-password'
$env:PRESENTER_ORBIT_UPDATE_NOTES = 'Release notes for this version'
npm run package:gpui:windows -- --x64 --updater
```

Replace the example URL with the actual public HTTPS hosting base. The URL and public key are embedded at build time; users cannot redirect update checks to another host. `package:gpui:macos` and `package:gpui:linux` accept the same flag; add `--arm64` for ARM builds. macOS and Linux packages must be built on their respective OS. Updater builds require the signing environment; ordinary unsigned builds remain available without `--updater`.

For every release, increase the version in `package.json`, and the workspace version in `Cargo.toml` together (`npm run version` syncs `Cargo.toml` and the lockfile from `package.json`). Build with `--updater`. The packaging script writes `dist/gpui/updates/<platform>-<architecture>/latest.json`, plus the installer/archive and `.sig` under a version subdirectory. Upload the version files first, then publish `latest.json` last. Serve manifests with `Cache-Control: no-cache`; keep version directories immutable. Use the installed NSIS app on Windows, the app bundle on macOS and AppImage on Linux for updates; standalone portable EXEs and Debian package installations are not a supported updater distribution.

Each platform/architecture has its own manifest so publishing a Windows build does not overwrite the macOS or Linux update feed. The client accepts only releases newer than its current semantic version. Network or signature errors are shown in settings and never trigger an installation. Test with two successive signed versions on an installed app before publishing broadly; downloading and installing cannot be validated using an unsigned development run.

Build public releases from the exact tagged source. Link that source on the download page and verify the notices in the packaged artifact for each target. The notices include the pinned crate source archives and Tabler's license. Windows and macOS packages also include FFmpeg's corresponding source, notices and build recipe. Keep those files available for every binary you offer, including binaries served by the update feed.

## GitHub releases and Cloudflare hosting

Push a `v` tag matching the workspace version to run the Presenter Orbit release workflow. It builds Windows x64, macOS Apple Silicon and Intel, and Linux x64. It publishes the installers and portable downloads to GitHub Releases, marks pre-release versions as pre-releases, and publishes signed update artifacts and per-platform manifests to R2. A failed publication can be retried by re-running the failed jobs so it reuses the same packages.

Configure the GitHub environment named `release` with a deployment tag rule for `v*`.

- Variables are `PRESENTER_ORBIT_UPDATE_BASE_URL` (currently `https://presenter-update.orvionlab.com`), `PRESENTER_ORBIT_UPDATE_PUBLIC_KEY` (the `.pub` file contents), `CLOUDFLARE_ACCOUNT_ID`, and `CLOUDFLARE_R2_BUCKET`.
- Secrets are `TAURI_SIGNING_PRIVATE_KEY` (the encrypted key file contents), `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, `CLOUDFLARE_R2_ACCESS_KEY_ID`, and `CLOUDFLARE_R2_SECRET_ACCESS_KEY`. Scope the R2 token to Object Read & Write on the release bucket.

The workflow verifies the signing keys match before building. It uploads versioned objects first, checks the public URLs, and publishes the manifests after the GitHub release assets. Existing public versioned objects are immutable. Public builds before `0.0.1-alpha.2` must be upgraded manually once to move to the root feed.

For a manual release, build with `--updater`, upload the versioned files from `dist/gpui/updates/` under the configured prefix, then publish each `latest.json` and attach the installers to the GitHub release. Install the first build with the configured updater manually. Windows Authenticode and macOS notarization are separate from updater signing.

The public R2 custom domain serves static JSON and installers over HTTPS. Manifests use `Cache-Control: no-cache`.

Cloudflare R2 has a free allowance, with charges above the allowance; do not assume unlimited free storage or requests. Public production access should use a custom domain rather than the development `r2.dev` endpoint. No Cloudflare account, bucket or deployment is created by these scripts.

References: [Cloudflare Pages limits](https://developers.cloudflare.com/pages/platform/limits/), [R2 pricing](https://developers.cloudflare.com/r2/pricing/), [R2 public buckets](https://developers.cloudflare.com/r2/buckets/public-buckets/).
