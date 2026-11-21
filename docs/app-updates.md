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

## Cloudflare hosting

Publish releases manually. Build with `--updater`, upload `dist/gpui/updates/` to the configured host with the versioned files first and `latest.json` last, then attach the packages to a GitHub release.

The settings the old workflow used are still the right ones: repository **Variables** `PRESENTER_ORBIT_UPDATE_BASE_URL` (public HTTPS custom domain of the bucket, no path, query or fragment), `PRESENTER_ORBIT_UPDATE_PUBLIC_KEY` (contents of the `.pub` file), `CLOUDFLARE_ACCOUNT_ID`, `CLOUDFLARE_R2_BUCKET`; **Secrets** `TAURI_SIGNING_PRIVATE_KEY` (full contents of the private key, not a path), `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, and the R2 access key pair scoped to the bucket. Avoid publishing two versions concurrently: the last publication sets the feed. The first signed installer must be installed once by hand; builds without an endpoint cannot discover the feed. Updater signing does not provide Windows Authenticode signing or macOS notarization.

No dynamic update backend is required: static JSON and public HTTPS files are sufficient. Cloudflare Pages can serve the manifests, but its individual file limit is 25 MiB, which can be too small for desktop installers. R2 is suitable for larger files: upload the contents of `dist/updates/` to an R2 bucket and expose them through a public custom domain. Use that domain as the update base URL. Set the manifest's cache metadata to `no-cache`.

Cloudflare R2 has a free allowance, with charges above the allowance; do not assume unlimited free storage or requests. Public production access should use a custom domain rather than the development `r2.dev` endpoint. No Cloudflare account, bucket or deployment is created by these scripts.

References: [Cloudflare Pages limits](https://developers.cloudflare.com/pages/platform/limits/), [R2 pricing](https://developers.cloudflare.com/r2/pricing/), [R2 public buckets](https://developers.cloudflare.com/r2/buckets/public-buckets/).
