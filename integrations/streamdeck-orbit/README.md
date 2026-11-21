# Presenter Orbit Stream Deck plugin

Direct Orbit control through the authenticated Remote HTTP API, using the
official Elgato SDK. Companion is not required.

## Build and install

From the repository root, with its development dependencies installed:

```sh
npm ci --prefix integrations/streamdeck-orbit
npm --prefix integrations/streamdeck-orbit run check
npm run package:streamdeck
```

To test the packaged SDK with a specific Stream Deck Node runtime, set
`ORBIT_STREAMDECK_NODE` to its executable before building the package.
The Windows checks passed with Elgato's installed Node **20.19.0**.

Double-click
`integrations/streamdeck-orbit/dist/com.presenterorbit.control.streamDeckPlugin`.
The installer is self-contained: users do not need Node, npm or this repository.
Requires Stream Deck 6.9+ with its Node 20 runtime. Windows 10+ and macOS 12+ are
declared targets; native Orbit acceptance was run on Windows. Physical-device,
installed Stream Deck host and Mac acceptance remain unverified.

1. Enable Remote in Orbit and note its host, port and password (default port
   49321).
2. Drag **Orbit Control** from **Presenter Orbit** to a key.
3. Enter the connection and polling interval (250–10000 ms, for example 500),
   then click **Save connection**. This connection serves all Orbit keys.
4. Choose navigation, a layer to clear, or an exact workspace target. Optionally
   give the key a short label.

The password uses Stream Deck global settings, separate from exported action
settings. Orbit Remote uses HTTP Basic authentication; use a trusted network.
The plugin does not add TLS.

The target list includes live/library/playlist cues and arrangements, timers,
Looks, macros, props, messages, clear groups, stage layouts, media triggers and
recording controls. Targets retain exact workspace and occurrence identities.
Missing targets remain unavailable until explicitly selected again. The list
refreshes when Orbit's catalog changes.

Blue is ready; green indicates matching live content or a running timer;
amber means configuration is needed or execution is pending; red is offline.
Success produces Stream Deck's check mark; failures produce an alert and
inspector text. Macros wait for completion. Busy presses do not submit twice.
Polling reconnects automatically; uncertain commands are never resubmitted.
Custom Stream Deck images/titles can override generated feedback.

This release supplies keypad controls. Dials/touch strips, automatic profile
installation, arbitrary command authoring and definition editing are outside
its scope. Marketplace publication is separate and has not been requested.

## Verification

`npm run check --prefix integrations/streamdeck-orbit` runs Elgato validation.
Packaging includes GPL-3.0-only licensing and bundled dependency notices.
Eight tests cover targeting, feedback, lifecycle, in-flight cancellation, the
actual property-inspector HTML/JavaScript, and the bundled SDK in a separate
process against HTTP/WebSocket fixtures. That protocol harness is not the
installed Stream Deck host or physical hardware.

For a real Orbit smoke test, use the isolated app/fixture setup in the
[Companion acceptance instructions](../companion-orbit/README.md#installed-host-acceptance),
then run:

```sh
node integrations/streamdeck-orbit/acceptance/native.mjs --orbit=http://127.0.0.1:5194 --password-file=.tmp/ctl14/password.txt
```

The runner accepts only loopback and the synthetic `.tmp/ctl14/workspace`.
SDK key events operate real native cues/timers and await queued macro results.
Authentication failure/recovery is tested. It does not modify Stream Deck
profiles. Stop the disposable Orbit and renderer processes afterwards.
The native runner also accepts `--node=PATH` to select that runtime explicitly.

Source reuses the CTL-14 client/catalog helpers at build time; the installer
bundles its own copy. Changes to these helpers should run both
the standalone integration build/check commands.
