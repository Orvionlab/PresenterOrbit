# Presenter Orbit Companion module

This standalone Bitfocus Companion module controls Presenter Orbit through its HTTP API. It requires Presenter Orbit's Remote service to be enabled and reachable. Configure the Orbit host, port, and Remote password in the Companion connection. The API uses HTTP Basic authentication with the fixed username `presenter-orbit`.

The module reads live state and exact target IDs from `GET /api/v1/status`. It submits commands to `POST /api/v1/command`; queued commands are observed at `GET /api/v1/commands/{requestId}`. It does not retry a command submission after an uncertain network failure.

Actions cover navigation and clears, exact live/library/playlist cues, Audience Looks, macros, timers, props, stage messages/layouts, audience messages, clear groups, media transport, and recording presets. Dynamic target choices refresh with the active workspace. Cue choices retain playlist entry and arrangement occurrence identity.

Feedbacks report connection and live presentation state, current cue, props, timers, audience message, capture state, and output layer visibility. Variables expose the latest workspace, presentation, cue, recording, and connection values. Button presets provide common navigation, clear, and live-state feedback controls.

This independently authored standalone module uses the MIT license required by Bitfocus's module tooling. It communicates with Orbit through HTTP and contains no copied Orbit application implementation. Presenter Orbit itself retains its project license. Run `npm run check` and `npm run package` to validate and build a Companion module package. The package is compatible with Companion's Node 26 runtime.

## Build and install

From the Orbit repository root:

```sh
npm ci --prefix integrations/companion-orbit
npm --prefix integrations/companion-orbit run check
npm run package:companion
```

Import `integrations/companion-orbit/presenter-orbit-0.1.0.tgz` in Companion's
Modules page, then add a Presenter Orbit connection. The built directory
`integrations/companion-orbit/pkg` can also be used as a developer modules path.
The first release is verified with Companion **5.0.7**, module-base **2.1.3**
and the Node 26 runtime on Windows. See [operator help](companion/HELP.md).

## Installed-host acceptance

The optional acceptance runner operates only on loopback endpoints, a connection
named `Orbit_Test` and a workspace under `.tmp/ctl14/workspace`. It imports four
virtual buttons into the disposable Companion configuration. It checks actual
cue/navigation, native timer state, the rendered timer button's feedback color,
queued macro completion, incorrect credentials and automatic network recovery.
It uses a local forwarding proxy to inject a network outage; it never retries
command submissions. The runner restores the connection to its direct Orbit
endpoint when it finishes.

Run `node integrations/companion-orbit/acceptance/create-fixture.cjs` from the
repository root to generate the synthetic workspace and password file. Use a
separate Orbit test identity (`com.presenterorbit.companion-smoke`) and seed its
own settings with this workspace before launching, so legacy preferences cannot
select an unrelated workspace. Build the test identity by setting `TAURI_CONFIG`
to `{"identifier":"com.presenterorbit.companion-smoke"}` for the Cargo build.
Launch its WebView against the local renderer server on port 5173. Optional
bridge setup uses `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=5192`
and `node integrations/companion-orbit/acceptance/evaluate-webview.cjs .tmp/ctl14/setup-webview.js`.
Restart the test application after initial fixture setup so renderer and native
catalogs start from the same persisted workspace. Remote uses port 5194.

Start the installed Companion core using its bundled Node 26, a separate
`--config-dir .tmp/ctl14/companion-config`, `--extra-module-path integrations/companion-orbit/pkg`,
`--admin-address 127.0.0.1` and `--admin-port 5193`. Disable discovered surfaces
in this test configuration and add `Orbit_Test` using the synthetic password.
Paths supplied to the installed executable should be absolute.

```sh
node integrations/companion-orbit/acceptance/installed-host.cjs --companion=http://127.0.0.1:5193 --orbit=http://127.0.0.1:5194 --password-file=.tmp/ctl14/password.txt --canvas-module="C:/Program Files/Companion/resources/node_modules/@napi-rs/canvas"
```

The canvas argument selects the installed Companion renderer library used to
check the actual button pixel. Stop the two disposable applications after the
test. The test does not require a physical Stream Deck or enable Orbit outputs.
