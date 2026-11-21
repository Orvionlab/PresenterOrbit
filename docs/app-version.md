# App version

`package.json` owns the application version. The workspace version in `Cargo.toml` is kept equal to it, and the app reads its version from Cargo (`CARGO_PKG_VERSION`). About, updates, and native diagnostics use that version, and `scripts/package-gpui.mts` refuses a package when the two differ.

Use `npm version <version> --no-git-tag-version` to prepare a release. The npm version lifecycle runs `scripts/sync-app-version.mts`, synchronizing the Cargo manifest and regenerating its lockfile without updating dependencies. npm updates its own manifest and lockfile. The maintenance check rejects mismatches between these versions.

Validate with `npm run verify`, commit and push, and build Windows installers and portable packages with `npm run build` (`npm run package:gpui:windows`). `scripts/check-versions.mts` (part of `npm run maintenance:check`) rejects mismatches between `package.json`, `Cargo.toml`, and both lockfiles.
