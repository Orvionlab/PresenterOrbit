# Tauri runtime (historical)

Presenter Orbit was a Tauri app with a React/TypeScript renderer (Vite) until Phase 16 of the GPUI migration. The app is now Rust only: see [gpui-migration](gpui-migration/README.md) for the plan, decisions, and the Phase 16 packaging notes ([phase-16.md](gpui-migration/phase-16.md)), and [architecture-dataflow.md](architecture-dataflow.md) for the host rules.

## Recovering the Tauri app

- The last commit that contains `src-tauri/`, `src/renderer/`, `src/shared/`, and the Tauri npm scripts is **`64121abe`** (the commit before the deletion commit). Everything remains in git history:
  - `git worktree add ../orbit-tauri 64121abe` checks it out beside the repository (run `npm ci` and `npm start` there).
  - `git show 64121abe:src-tauri/tauri.conf.json` reads a single file; `git show 64121abe:docs/tauri-migration.md` is the previous version of this document.
- The Tauri adapter was a thin layer over `crates/orbit-host`; the host still has the `WindowSystem`, `DesktopIntegration`, `Updater`, and `WebCapture` traits that it implemented, and `orbit-app` implements them now.

## What stayed for compatibility

- The updater feed, manifest format, and minisign signature format are the Tauri ones, so an installed Tauri app can update to the GPUI build. `scripts/updater-signing.mts` writes the same signature as `tauri signer sign` did, from the same `TAURI_SIGNING_PRIVATE_KEY` variables.
- Release builds use the Tauri identifier `com.presenterorbit.desktop` for the data folder, so settings, workspaces registry, screens, devices and downloaded Bible packages are in place after an update. Debug builds use `com.presenterorbit.gpui`.
- The Windows installer keeps the Tauri product name, publisher, registry keys, default folder, and the `/P /UPDATE /R` command line.

## What did not carry over

- Layout positions that the React app kept in WebView `localStorage`.
- The Linux AppImage no longer bundles WebKitGTK and GStreamer (web fills need the system WebKitGTK).

## Provenance comments

Many Rust files say "ported from `src/renderer/...`" or "`src-tauri/src/...`". Those paths no longer exist; read them with `git show 64121abe:<path>`. The `integrations/companion-orbit/acceptance` helpers still drive the Tauri app (`src-tauri/tauri.conf.json`, `window.presenterOrbit`) and need a GPUI-side replacement before they can be used again.
