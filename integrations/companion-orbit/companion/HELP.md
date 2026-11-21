# Presenter Orbit

Requires Companion 5 with a Node 26 module runtime. Verified with Companion
5.0.7 and Presenter Orbit 0.0.1-alpha.3 on Windows.

1. Enable Remote in Orbit Settings and set its port and password.
2. Import the module's `.tgz` from Companion's Modules page, or load the built
   package through a local developer modules folder.
3. Add a Presenter Orbit connection. Enter the Orbit host, port and password.
   The password uses Companion's secret field. Green connection status means
   authenticated status polling is working.
4. Drag common buttons or Workspace targets from Presets to your button grid.
   Workspace targets include cues, macro execution, timers, props, Looks,
   messages, stage layouts, media triggers and recording controls.

The target selector preserves workspace, playlist entry and cue occurrence
identities. Re-select a target after changing its workspace or arrangement.
Unavailable targets fail explicitly. Clear and navigation controls operate on
the current live output. Timer buttons change color while the timer runs;
cue buttons change color when that exact occurrence is live.

Additional actions adjust timers, edit/show/hide stage messages and control the
current media/audio occurrence. Feedbacks include connectivity, presentation,
cue occurrence, prop, timer, message, recording and per-output layer visibility.
Use exact IDs from the status API for custom feedbacks, or use the generated
presets. Cue and timer feedbacks also require the exact workspace path so a
copied workspace cannot accidentally light an old target's button.
Variables include current workspace, presentation, cue, recording,
connection state and the last command result.

Queued macros are observed until completion; other queued actions report the
API's applied/dispatched state. A network failure never automatically repeats
a command submission. Check `last_result` and the connection log when an action
fails or its result is unconfirmed. Polling reconnects automatically and clears
live feedback on connection loss.

Orbit Remote uses HTTP Basic authentication on the configured local-network
endpoint. Module installation is local; this module is not yet distributed
through Bitfocus's official module store. Stored definition editing and
Companion's action recorder are outside this module's current control surface.
