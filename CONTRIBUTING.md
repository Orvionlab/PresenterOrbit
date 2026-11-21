# Contributing to Presenter Orbit

## Setup and first verification

1. Install Rust (stable, with the MSVC toolchain on Windows), Node 26.5.1, and npm 12.0.2 (`.node-version` and `package.json` are authoritative). Node is used only for repository scripts.
2. Run `npm ci` from a fresh clone (it also downloads the ProPresenter reference schemas that host tests read).
3. Run `npm run verify`. Run the app in an interactive desktop session when changing native windows, persistence, output, or NDI.
4. Start with `cargo run -p orbit-app -- --data-dir <scratch folder>`, choose a new test workspace folder, make one small UI change, and rerun `npm run verify`.

Do not commit `dist/`, `node_modules/`, or Rust target output. Edit `.arb` locale files in `crates/orbit-ui/locales`; the app reads them directly.

## Architecture and safe changes

UI is in `crates/orbit-app/src/<feature>/` on the primitives in `crates/orbit-ui`; reusable behavior belongs in controllers and pure helpers. The host (`crates/orbit-host`) owns filesystem, output, NDI, and remote capabilities. The UI reaches it only through `Host::invoke` wrappers; a new command needs the validated host method, a typed wrapper, and a controller call off the UI thread. See `AGENTS.md` and `docs/design-system.md`.

Run `npm run check` after any change. Workspace version 1 is strict: do not add migrations, aliases, or missing-file fallback behavior without an explicit product decision. See `docs/architecture-dataflow.md` and `docs/dependency-policy.md` before changing those boundaries.

## Small first change walkthrough

Add an English and Dutch ARB key (with the English `@key` description) for a non-critical settings label, use it in the view, add a focused assertion to the closest test, then run `npm run verify`. Include the command output and a rollback note in the pull request.
