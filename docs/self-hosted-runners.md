# Local Windows and Linux build runners

The main CI and package builds run on two repository-scoped self-hosted GitHub Actions runners on one Windows PC:

| Environment | Build label | Working directory |
| --- | --- | --- |
| Native Windows x64 | `orbit-windows-x64` | `%USERPROFILE%\actions-runner-presenter-orbit\_work` |
| Ubuntu 24.04 x64 in WSL2 | `orbit-linux-x64` | `~/actions-runner-presenter-orbit-linux/_work` |

Public CI and pull requests use GitHub-hosted runners. Register local runners only with a private repository.

These are native builds on each operating system. Linux compilation, dependencies and build outputs live inside Ubuntu's Linux filesystem. This does not cross-compile Linux packages with Windows tools. The existing GTK 3, WebKitGTK 4.1 and ALSA development packages are present in Ubuntu. Node 26.5.1, npm 12.0.2 and Rust are available to both runners.

Each repository checkout has persistent npm and Cargo caches in a sibling `.orbit-build-cache` directory inside its runner workspace. `configure-local-runner-caches.mts` validates the runner-owned workspace before setting `NPM_CONFIG_CACHE`, `CARGO_HOME` and `CARGO_TARGET_DIR` for subsequent steps. These local caches are not uploaded to GitHub. They preserve compilation output across checkout cleanup and keep the operator's general npm/Cargo caches separate. The packager respects the configured absolute Cargo target directory when collecting installers and other artifacts.

## Start and availability

Both listeners start hidden after the runner's user signs in to Windows. A per-user Startup shortcut named `Presenter Orbit Actions Runners` launches a copy of the PowerShell launcher in `%USERPROFILE%\actions-runner-presenter-orbit`. This works independently of the current repository branch or checkout. No `.bat` conversion, password, Windows service or administrator account is needed. The Linux listener has a persistent hidden WSL process so it remains available after a shell command finishes. The PC must remain awake and connected. Signing out stops the listeners; signing in starts them again. A WSL shutdown requires starting the Linux listener again.

To start the runners manually without signing out, run this from the repository in PowerShell:

```powershell
./scripts/start-local-actions-runners.ps1
```

Use `-Platform Windows` or `-Platform Linux` to start one runner. The launcher checks for an existing listener and starts new processes hidden. Its logs are `listener.stdout.log`, `listener.stderr.log`, `linux.stdout.log` and `linux.stderr.log` under the Windows runner directory. Runner registration credentials remain in the runner directories, outside the repository, and are not committed.

To install or update automatic sign-in startup on this PC, run `./scripts/install-local-runner-startup.ps1`. This validates the registered repository, copies the launcher and creates only its own shortcut in the current user's Startup folder. To disable it, disable `Presenter Orbit Actions Runners` in Windows Startup apps or remove that shortcut through `shell:startup`. Registration and running listeners are unaffected.

## Build and check jobs

In a private repository, Windows pushes and manual CI runs can use the local Windows x64 runner. The self-hosted package acceptance, burn-in, and bundled FFmpeg jobs are private-repository jobs. macOS and Linux PR checks use GitHub-hosted runners. The workflows call `npm run check`, `npm run verify`, and `npm run package:gpui:windows`.

The Windows installer needs NSIS on the runner (`choco install nsis`) or `PRESENTER_ORBIT_MAKENSIS`. Private-fork workflow execution was disabled when these local runners were registered; keep local runners restricted to this repository.

## Separate runtime acceptance

These build runners do not have the `orbit-package-tests` label. The packaged smoke and the burn-in workflows that start the application require a separate disposable runner with that label. `scripts/gpui-smoke.mts` starts the package on a private temporary data folder, so it never reads or writes real settings.

### Disposable Hyper-V Windows guest

Run `scripts/setup-windows-package-test-runner.ps1` **inside the disposable Windows 11 x64 VM**, while signed in to its test profile. It rejects a physical Windows host and verifies the guest's Hyper-V model and VM identity before writing anything. The VM needs Internet access to GitHub, nodejs.org and registry.npmjs.org. Do not run it in a normal host profile.

The bootstrap creates only `%TEMP%\orbit-package-test-runner-v1` inside the guest. That path must not contain whitespace because the silent NSIS installation test requires it. An ownership marker binds the directory to the repository, VM UUID and guest user SID. Rerunning the bootstrap reuses its verified archives and existing ephemeral registration and avoids duplicate listeners. It installs no Windows service, startup item or machine-wide developer tools.

The script downloads portable Node 26.5.1, PowerShell 7.4.21, MinGit 2.56.0.2 and Actions runner 2.337.0 from their official publishers and checks pinned SHA-256 hashes before extraction. Node's bundled npm is 11.17.0, so npm 12.0.2 is separately downloaded from the official npm registry, verified against its pinned SHA-512 integrity value, and installed offline into the runner-owned prefix. Tool versions are checked before registration. The actual NSIS installer provisions WebView2 silently if the guest does not already have the Evergreen Runtime; runtime failures remain test failures.

For an interactive registration, run:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\setup-windows-package-test-runner.ps1
```

Paste a short-lived **repository runner registration token**, not a personal access token, at the hidden prompt. Alternatively, a prepared private VM kit can include `orbit-registration-token.json` with `token` and `expires_at` fields:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\setup-windows-package-test-runner.ps1 -RegistrationTokenFile .\orbit-registration-token.json
```

The file route validates the expiration and consumes only that explicitly supplied file after successful registration. Tokens are never printed or committed. The runner stores its own credentials inside its guest-owned directory. Delete any unused kit token copy after registration or expiration. Do not place kit credentials in shared repository history, workflow artifacts or logs.

If guest HTTPS downloads are unavailable, prepare the same five publisher archives in the private kit's `downloads` directory and explicitly select that input:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\setup-windows-package-test-runner.ps1 -RegistrationTokenFile .\orbit-registration-token.json -ArchiveDirectory .\downloads
```

This mode performs no bootstrap archive downloads. It requires all five exact archive names, verifies the same pinned hashes before copying each file, and can replace an interrupted guest cache download only after the supplied source passes verification. Missing or invalid supplied archives fail explicitly; there is no alternate network source. The runner registration and subsequent GitHub Actions job still need Internet access. Online download failures report their official URL and guest connectivity error rather than silently switching download methods.

The runner registers for this repository with `orbit-package-tests`, `Windows`, and `X64` labels and `--ephemeral`. Its `.env` explicitly marks only this disposable VM profile and sets a local npm prefix. It accepts one job and GitHub deregisters it afterwards. Keep the VM signed in and the foreground listener window open until the installed/portable job has uploaded its evidence. A later job requires rerunning the script with a fresh token. Host build runners remain unchanged.

Publisher checksum sources: [Node release checksums](https://nodejs.org/dist/v26.5.1/SHASUMS256.txt), [PowerShell release](https://github.com/PowerShell/PowerShell/releases/tag/v7.4.21), [Git for Windows release](https://github.com/git-for-windows/git/releases/tag/v2.56.0.windows.2), [Actions runner release](https://github.com/actions/runner/releases/tag/v2.337.0), and [npm package metadata](https://registry.npmjs.org/npm/12.0.2). These are pinned bootstrap inputs; updates require verifying new official digests.
