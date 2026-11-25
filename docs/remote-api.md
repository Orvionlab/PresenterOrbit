# Orbit HTTP API v1

Enable Network and ProPresenter Remote in app settings, and set the network port and Remote password. The default network port is 49321 and is shared by the browser/HTTP API, current ProPresenter Remote and Stage App. Every Remote HTTP status and command request requires HTTP Basic authentication with username `presenter-orbit` and the Remote password. Disabling Network closes LAN access and mobile/TCP sessions and suspends Network Link delivery without erasing its configuration. Remote and Stage App can be switched independently. The services use the local network; use a trusted network. This HTTP API is an Orbit API; the separate [ProPresenter Remote and Stage Display WebSocket](#propresenter-remote-and-stage-display-websocket) paths support the current mobile clients.

The native server handles one request per TCP connection and explicitly returns
`Connection: close`. Clients must open another connection for subsequent
requests. The [Companion module](../integrations/companion-orbit/README.md)
provides a locally installable control surface for this API, including exact
target selection, live feedback and queued-command completion.

The separate [Stream Deck plugin](../integrations/streamdeck-orbit/README.md)
offers direct keypad controls and live key feedback without Companion.

## Local MCP

In Presenter Orbit, open **Settings → Local MCP**, enable the service, and set
its password. The MCP port defaults to 49324 and works independently of Network
and ProPresenter Remote. Keep Presenter Orbit running while using MCP.

In your local MCP client, use the full path to the Presenter Orbit executable
as the command and give it the single argument `--mcp`. Set
`PRESENTER_ORBIT_MCP_PASSWORD` to the MCP password in the server process
environment. Set `PRESENTER_ORBIT_MCP_PORT` if Orbit uses a port other than
49324. The client communicates with the process over stdio. The process reads
Orbit's password-protected MCP endpoints on `127.0.0.1`.

The Local MCP permission switches apply to all clients using the password.
Read permissions are enabled by default. **Create presentations** and
**Create and delete playlists** are disabled by default. The playlist permission
also allows the source presentation edits described below.

### Codex

Under **Settings → Local MCP**, copy the Codex install command and run it once
in PowerShell 7. The command includes the current Orbit executable path and
MCP port. For example:

```powershell
codex mcp add presenter_orbit --env ("PRESENTER_ORBIT_MCP_PASSWORD=" + (Read-Host -MaskInput 'Orbit MCP password')) --env 'PRESENTER_ORBIT_MCP_PORT=49324' -- 'C:\Path\To\orbit-app.exe' --mcp
```

PowerShell masks the password as you enter it. Codex stores the value in its
local configuration. Keep Orbit open, restart Codex, and open a local session
on that computer. Use `/mcp` to check for `presenter_orbit`, then ask for your
Orbit libraries. A hosted ChatGPT web session does not read this local
configuration.

A development build also works. Use its `target/debug/orbit-app` executable
(`orbit-app.exe` on Windows) and enable Local MCP in that running build. Turn
on **Create presentations** if you want Codex to save a presentation.

### Claude Code

Under **Settings → Local MCP**, copy the Claude Code install command and run
it once in PowerShell 7. For example:

```powershell
claude mcp add --transport stdio --scope user presenter_orbit --env ("PRESENTER_ORBIT_MCP_PASSWORD=" + (Read-Host -MaskInput 'Orbit MCP password')) 'PRESENTER_ORBIT_MCP_PORT=49324' -- 'C:\Path\To\orbit-app.exe' --mcp
```

Claude Code saves the password in your user configuration and makes the
server available across projects. Keep Orbit open, start a new Claude Code
session, and use `/mcp` to check the connection.

### Available tools

| MCP tool | Result |
| --- | --- |
| `get_live_status` | Live presentation, current slide, outputs and layers |
| `list_libraries` | Library and presentation titles and IDs, plus available arrangements |
| `list_playlists` | Playlist titles and IDs, folder relationships, and ordered entries |
| `list_themes` | Themes and layout IDs |
| `create_presentation` | The ID and title of a saved presentation |
| `create_playlist` | The saved playlist ID, title, and ordered entries |
| `delete_playlist` | The deleted playlist ID |

Use `create_playlist` with `workspaceId`, `title`, and optional `entries`. For
each entry, specify one of these types.

- `header` takes a `title` and optional `color` (hex RGB/RGBA or `transparent`).
- `placeholder` takes a `title`. Set `linkedItemId` to reference an existing
  presentation and optionally choose its `arrangementId`.
- `presentation` takes an `itemId` and an optional `arrangementId`. An optional
  `themeLayoutId` applies a layout from `list_themes` to every slide in the
  source presentation. Optional `size` takes an integer `width` and `height` in
  pixels and changes the source resolution. These two settings apply wherever
  that presentation is used.

Use `list_libraries` for presentation and arrangement IDs. Use
`delete_playlist` with `workspaceId` and `playlistId` from `list_playlists`.
Deleting a playlist removes its entries and preserves its source presentations.


The read tools take no arguments. To create a presentation, use the
`workspaceId` from the library or theme catalog, choose `libraryId` and
`themeLayoutId`, and supply a title, width, height, and a `slides` array with
one `text` value per slide. Dimensions are whole pixels from 1 to 16384.

Optional fields include `notes` on each slide, `operatorNotes` for the whole
presentation, and `layer` with `slides` (default) or `announcements`.
`playlistId` appends the presentation to the end of an existing playlist.
To save a named arrangement, supply `arrangement` with a `name` and a
one-based `slideOrder` such as `[2, 1, 2]`. The arrangement becomes the
selected order and is assigned to the new playlist entry when requested.
Orbit validates the options together before saving and keeps the current
output selection.

## HTTP status and commands

`GET /api/v1/status` returns the accepted native output state. Existing presentation, slide, output, layer, background and audio fields remain available. `timers` and `props` contain available definitions, `timerRuns` contains timer run state, and `livePropIds` identifies active props. Timer run values use milliseconds: `end` is the epoch anchor, `paused` identifies a stopped run, `remaining` is its anchor value, and elapsed runs include `elapsed: true`. Running values evolve from the anchor; status does not poll a new run value every second.

The status response also includes the current `workspacePath` and persisted `audienceLooks` and `macros` target catalogs (`id`, `title`, `enabled`). A macro is available when at least one action is enabled. Empty macros and macros with only disabled actions cannot be dispatched. `controlsError` explicitly reports workspace catalog read failures. Look and macro commands must include the exact `workspacePath` returned by status; this binds a request to the workspace the operator viewed.

`POST /api/v1/command` accepts JSON:

`currentSlide`, `previousSlide`, and `nextSlide` include `cueInstanceId` as well as `id`, `title`, and `text`. The browser presenter view shows the next cue and triggers that exact arrangement occurrence with the current presentation/playlist identity. At the end of the arrangement it shows an end marker instead of an advance target.

The browser timer panel shows running/paused values calculated from the run anchors and the status response's `serverTimeMs` (current server epoch milliseconds), including elapsed and negative overrun values. `updatedAt` continues to identify snapshot acceptance time; `serverTimeMs` refreshes on every status response. It accepts a signed numeric seconds adjustment through `increment`. Polling preserves the adjustment field and keyboard focus; fields disable during command submission or connection loss. An absent run is shown as not started. These live controls do not change persisted timer definitions; the Timer definitions section does (see `timerDefinition`).

| Command | Required fields | Behavior |
| --- | --- | --- |
| `timer` | `id`, `action` | `start`, `stop`, `reset`, or `increment` |
| `timer` with `increment` | `id`, `action`, `seconds` | Finite signed numeric adjustment in seconds; uses existing timer runtime semantics |
| `prop` | `id`, `action` | `show` or `clear` an exact prop id |
| `cue` | `workspacePath`, `presentationId`, `playlistEntryId`, `slideId`, `cueInstanceId` | Select an enabled exact cue occurrence in the current live arrangement; playlist/cue occurrence fields must be explicitly null for unscoped cues |
| `libraryCue` | `workspacePath`, `libraryId`, `presentationId`, `playlistEntryId: null`, `slideId`, `cueInstanceId`; optional `arrangementId` | Trigger an enabled exact cue from the selected persisted library presentation and named arrangement; clear live playlist context. Without `arrangementId`, use the persisted selection |
| `playlistCue` | `workspacePath`, `playlistId`, `playlistEntryId`, `presentationId`, `slideId`, `cueInstanceId` | Trigger an enabled exact cue from a persisted presentation occurrence and its entry arrangement; retain that exact playlist context |
| `stageMessage` | `workspacePath`, `action`; `text` for edit | `edit`, `show`, `hide`, or `clear` the accepted live stage message; editing preserves visibility, clearing removes text and visibility |
| `nextSlide`, `previousSlide`, `clearAll` | `command` | Existing synchronous output controls |
| `clearLayer` | `layer` | Existing layer control |
| `nextItem`, `previousItem` | `command` | Existing renderer item navigation; queued asynchronously |
| `audienceLook` | `id`, `workspacePath` | Activate a persisted Audience Look through the local validated screen mutation controller |
| `capture` | `action`, `workspacePath`; `id` for start | `start` runs a supported persisted file recording preset; `stop` waits for encoder finalization |
| `macro` | `id`, `workspacePath` | Validate and run a persisted macro through the local macro controller and automation runtime; the request ends in `completed` or `error` once every enabled action has finished (see Macro completion) |
| `message` | `action`, `workspacePath`; `id` for show | `show` displays a saved audience message with its saved token values through the local Messages controller; `clear` clears the audience message and keeps stage text |
| `stageLayout` | `screenId`, `id`, `workspacePath` | Assign a persisted stage layout to a stage screen through the local Stage panel's screen output path |
| `clearGroup` | `id`, `workspacePath` | Run a persisted clear group through the local Clear Groups controller |
| `timerDefinition` | `id`, `workspacePath`, `patch` | Edit a persisted timer definition. `patch` supports `name`, `durationSeconds`, `allowsOverrun`, `type` (`countdown`, `countdown-to-time`, `elapsed`), `targetTimeSeconds`, `elapsedStartSeconds`, `elapsedEndSeconds` and `format`. Optional time fields and format accept `null` to clear them; duration remains required numeric data when supplied. Unknown keys are rejected |
| `messageDefinition` | `id`, `workspacePath`, `text` and/or `tokenValues` | Edit a persisted message. `text` is nonempty (at most 2000 characters) and may reference only defined tokens; `tokenValues` maps the names of the message's `text` tokens to strings (at most 2000 characters each, at most 64 entries) |

Examples: `{"command":"timer","id":"timer-id","action":"start"}`, `{"command":"timer","id":"timer-id","action":"increment","seconds":-10}`, `{"command":"prop","id":"prop-id","action":"show"}`.

Look/macro example: `{"command":"audienceLook","id":"look-id","workspacePath":"C:\\Shows\\Sunday"}`. Substitute the actual catalog identity and path. Native code validates the persisted target while holding the workspace operation lock. Renderer execution rechecks the workspace and current target, then uses the same controller as local controls. An ID from a different workspace, a removed target, or a macro without enabled actions is rejected.

Targets must exist in both the active workspace and accepted output snapshot. Timer definitions must still match their persisted runtime definition. Prop show enables the props look layer; clear removes only the selected prop and keeps other props. Repeated show does not duplicate a prop. Start resumes a stopped timer, stop pauses it, reset stops and restores the runtime's initial value (elapsed timers reset to zero), and increment follows existing runtime behavior, including starting countdown timers. These commands change live state, not persisted definitions.

The authenticated browser Library section lets the operator choose a library, presentation and named arrangement without changing output. Selecting an individual cue then commits that presentation and exact arrangement occurrence to native output. Status includes `libraries` with presentation and cue summaries plus `arrangementId` and named `arrangements` (`id`, `title`, `slides`). Removed library membership, disabled/missing cues, removed arrangements, stale arrangement occurrences, and commands from another workspace are rejected. Selecting a library cue clears playlist context. Arrangement choice only affects live output; it does not change the persisted presentation selection or arrangement definitions.

The Playlists section opens a playlist and its individual presentation occurrences before cue triggering. Status includes `playlists` (`id`, `title`, `entries`); each entry includes its own `id`, `presentationId`, `title`, `arrangementId`, and `slides`. Cue occurrence identities are scoped to the entry, so repeated presentations and repeated sections remain distinct. Execution rechecks the persisted playlist, entry membership, presentation and arrangement while holding the existing workspace operation lock. Removed entries or changed entry arrangements invalidate their old cues. Headers, placeholders and standalone media are outside this presentation-cue catalog.

This browse-first, cue-second workflow follows Renewed Vision's [ProPresenter Remote Library and Playlist views](https://support.renewedvision.com/hc/en-us/articles/43051104879379-ProPresenter-Remote-App-Interface): opening a presentation exposes its slides and tapping a slide triggers output. The bounded Orbit controls provide presentation browsing and named arrangement selection; they do not edit collections or arrangements.

The Stage message section separates its editable draft from the authoritative `stageMessage` status. Save text changes live text without changing visibility; Show and Hide affect live visibility; Clear empties and clears the live message. Polling preserves the draft and focus, while switching workspace resets the draft. These controls change accepted live output state, not the stored stage message definition. Each response carries the native revision and accepted state patch. Feedback confirms state acceptance, not physical screen delivery.

Responses:

- `200`: synchronous native command accepted; body includes `ok: true`, `status: "applied"` or `"unchanged"`, `changed`, and `revision`. Timer/prop/cue/libraryCue/playlistCue/stageMessage responses also include the accepted state `patch`. This confirms native state acceptance, not delivery to a physical display.
- `202`: renderer command event queued; `status: "queued"` does not confirm execution. Look/macro responses contain `requestId`, `workspacePath`, `command`, `id`, and `acceptedAtMs`; poll `GET /api/v1/commands/{requestId}` with the same authentication for execution feedback.
- `400`: malformed/incomplete request, invalid JSON, unsupported command/action, missing field, invalid adjustment, unavailable target or stale timer definition.
- `401`: absent or incorrect credentials; includes a Basic authentication challenge.
- `413`: request exceeds 64 KiB.
- `503`: output has no accepted snapshot or renderer event could not be queued.
- `500`: native workspace/output operation failed; `ok: false` and `error` explain the failure.
- `404`: unsupported path/method.

Messages, stage layouts and clear groups follow the queued look/macro path: native code validates the exact persisted target in the active workspace, and the renderer rechecks it and runs the local controller. Status adds the `messages`, `stageLayouts` and `clearGroups` catalogs (`id`, `title`, `enabled`), `activeMessage` (`id`, `instanceId`, or null), and a `stageLayoutId` for each stage output, which is its configured layout or its stage screen assignment. A stage layout assignment reports `applied`. Message and clear-group commands report `dispatched`, because timed message clearing and clear-group effects continue in the runtime. These controls follow the [ProPresenter Remote](https://support.renewedvision.com/hc/en-us/articles/43051104879379-ProPresenter-Remote-App-Interface) Messages, Stage Screen Layouts and Clear Groups views. Timer and message definition edits are described in Stored definition editing.

`GET /api/v1/commands/{requestId}` returns `queued`, `applied`, `dispatched`, `completed`, `error`, or `unconfirmed`. `applied` confirms that the Audience Look controller finished its native screen mutations. It does not confirm delivery to physical displays, and activation of several screens is sequential rather than atomic. A later screen failure can leave earlier screens applied; its result is `error`.

## Macro completion

A `macro` request moves `queued` to `completed` or `error`. While any enabled action is still pending the request reports `dispatched`, which is the only intermediate state and never the final result of a macro. A macro whose actions all finish synchronously goes straight from `queued` to its terminal state. The macro runs through the same controller as the local Macros panel and hotkeys, and the same runtime executes it; only the observation of the result differs.

`completed` means that every enabled action finished. What finished means depends on the action:

| Action | Finished when |
| --- | --- |
| Look, slide, clear, mask, prop show/clear, stage, message, media selection, audio input, transport | The renderer state mutation was applied. This is not delivery to a physical display. |
| `macro`, playlist cue entry with actions | Every nested action finished. Nesting is limited to 8 levels; a deeper level is a failure of its first enabled action. |
| Any action with `delaySeconds` | The native scheduler fired and the action, with everything it started, finished. A native rejection, a stale scheduler session, a failed scheduler bridge call, stopping the automation runtime or a workspace change before it fired is a failure of that action. |
| `timer` | The native timer control was acknowledged and its run applied. With a `configuration`, the persisted definition mutation is acknowledged first; if it fails the control does not run. An unavailable timer bridge is a failure. |
| `capture` | The capture controller finished starting or stopping. |
| `communication` | The device send was acknowledged as ok. |
| External presentation | The operating system accepted the URL open. |
| Clear group with the stage-message layer | The native stage message clear was acknowledged. |

Deliberately not part of an action's completion: media marker actions (they fire at media times), a prop's automatic clear timer, a message's timed clear, the end of playback and delivery to physical outputs. Disabled actions are ignored.

A synchronous failure stops the macro at that action, as it does locally; asynchronous work already started is still awaited. Failures never cancel other started actions. The terminal `error` carries `error`, a summary, and `failures`, a list of `{actionId, type, message}` with one entry per failed action in completion order (the innermost failing action for nested macros). A workspace change before completion ends the request in `error` even when the remaining actions would have succeeded, and delayed actions that have not fired are reported as failures; they still fire locally as before. Problems found before any action starts (unavailable macro, no enabled actions, invalid authored action) return an immediate `error` without `failures`.

While a macro is `dispatched`, the request is polled until it ends. Native code reports a dispatched macro as `error` as soon as the active workspace differs. The renderer is the only source of the terminal result: if it is reloaded or stopped while a macro is `dispatched` in the same workspace, the request stays `dispatched` until the workspace changes or the request is evicted. Macro requests have no timeout, because delays are authored by the operator.

## Stored definition editing

`timerDefinition` and `messageDefinition` edit persisted definitions and, unlike the live timer commands, are bound to the `workspacePath` from status. Native code parses the request strictly (unknown fields and invalid values are rejected with `400`), then reads the persisted workspace while holding the workspace operation lock and rejects unknown ids, invalid timer type/time/format fields, token values for tokens that are not `text` tokens, and text that references an undefined token. The renderer rechecks the target and persists through the local editors: timer edits use the native countdown timer mutation with the shared definition queue, and message edits use the Messages panel save with its validation and stale check. The request ends in `applied` only after the saved definition is adopted and equals the edit (a name is stored trimmed). Native validation errors, a stale message, a failed write or a workspace change during the write end the request in `error` and change nothing the remote can observe as saved.

Status adds `timerDefinitions` (`id`, `name`, `type`, `durationSeconds`, `allowsOverrun`, `durationEditable`) and `messageDefinitions` (`id`, `title`, `text`, `tokenValues`, `textTokens`) with the persisted values. The browser panel shows them in the Timer definitions and Message definitions sections. Each row separates its editable draft from the authoritative saved line; polling keeps the draft and focus, and switching workspace discards it. Only changed fields are sent. An edit does not change a live timer run's elapsed state or the text of a message that is already showing. The remote cannot create or delete timers or messages, change timer types, formats or token definitions, or edit message titles, themes or clear rules.

A workspace change before acknowledgement produces `error`. If the renderer has not acknowledged execution within 30 seconds, polling returns `unconfirmed` and does not retry or cancel the command. Late acknowledgement may still replace that unconfirmed view with a result. The server retains the latest 100 request results in memory; unknown, evicted or restarted-server IDs return `404`. Terminal acknowledged results retain their original workspace identity.

Open `/` on the configured remote port for the authenticated browser operator panel. It shows the live presentation, slide/item navigation, clear-all, available Audience Looks and macros, disables macros with no enabled actions, and displays acceptance and execution feedback separately. Macro runs are tracked in a list below the macro buttons (running, completed, or the aggregated failure), without blocking other controls. The browser also lists exact live arrangement cues, timer start/stop/reset, prop show/clear, and clear-layer buttons. Cue requests carry the live presentation, playlist-entry, and arrangement-occurrence identities from status; a changed presentation or removed/disabled occurrence is rejected. Synchronous feedback includes the committed native revision. The live cue section follows the current presentation; the Library section independently opens other library presentations for cue triggering. Capture controls start supported presets and stop the native encoder. The status response includes `recordingPresets` (`id`, `title`, `enabled`) and authoritative `captureStatus` with active/finalizing flags, output path, frame/byte counters and asynchronous encoder/source errors. Start confirms recording began; it does not promise later success. Poll status through finalization. Capture responses include `captureStatus`; failures are reported explicitly. Capture is video-only file recording; streaming is unsupported.


The Media Bin and Audio Bin sections browse persisted media libraries (including playlist item-file membership and graphics ordering) and trigger exact managed media IDs. Status includes mediaBins with id/title/items; each item exposes only id/title/type, never local source paths. Image/video triggers select the media layer; audio triggers select the independent music layer. Each trigger creates a new selection occurrence and resumes its layer while retaining cue, look and opposite-layer state. The current saved asset settings are adopted through the established native selection helpers.

| Command | Required fields | Behavior |
| --- | --- | --- |
| media | workspacePath, id, action: trigger | Trigger a persisted image, video or audio asset accepted by the output snapshot |
| media | workspacePath, id, action: play or pause, selectionId | Control the exact currently active video/audio occurrence through the coordinated native transport |
| media | workspacePath, id, action: seek, selectionId, position | Seek the exact native video/audio occurrence; position is finite and normalized from 0 to 1 within its trim range |
| media | workspacePath, id, action: inPoint or outPoint, selectionId | Jump video to its authored trim boundary; existing loop/end rules still apply |
| media | workspacePath, id, action: restart, selectionId | Seek the exact video/audio occurrence to its in boundary and resume through coordinated native transport |
| media | workspacePath, id, action: previous or next, selectionId, playlistId, playlistOrder | Trigger a neighbor within an explicitly authored audio playlist; playlistOrder must match the current canonical track ids |

Background and audio status expose selectionId; background additionally exposes paused, matching the audio paused field. Requests from a different workspace, deleted assets, assets absent from accepted output, still-image transport, and stale playback occurrences are rejected. The existing authenticated command endpoint applies to every media command. This implements manual audio track triggering and transport; automatic remote audio playlist progression remains outside this increment.

Seeking requires a known positive native duration. It updates the actual playback registry, resets decoder/audio queues through the shared propagation path and acknowledges the native output revision. Clock commands filter native sources to the selected identity and main destination. Restart also commits the canonical running transport state. Validation failures report invalid-target before applying a clock mutation; a propagation failure reports an explicit error without asserting that no partial mutation occurred.

Audio playlist navigation uses the workspace's canonical media-array order filtered by playlist membership, matching the local audio selection contract. It does not step through arbitrary bins or wrap past playlist boundaries. Changed playlist membership/order or a replaced active selection requires refreshing the remote. The status catalog identifies authored audio playlists separately from generic media folders.


## Tickers

Status includes persisted tickers (id, title, enabled) and ticker, the active native ticker snapshot or null. The active snapshot includes generation, elapsedSeconds, paused, and an explicit sourceError when a live source cannot provide content. The Tickers browser section follows the existing saved-definition/live-control organization. It offers exact saved ticker Play, generation-bound Pause/Resume/Clear, and Seek. Polling preserves the seek field.

| Command | Required fields | Behavior |
| --- | --- | --- |
| ticker | workspacePath, type: play, id | Start the exact saved ticker; allocate a new native generation |
| ticker | workspacePath, type: pause, resume, or clear, generation | Control only that active native generation |
| ticker | workspacePath, type: seek, generation, seconds | Seek that generation to a finite 0–86,400 second position |

These controls execute synchronously through the native ticker runtime, independently of the renderer cue selection. Responses carry ok, status, changed, and ticker on success; invalid/stale targets return invalid-target with an explicit error. The active workspace and generation are checked at the execution boundary. Seek preserves paused/running state. A successful play confirms the native clock started; a live RSS cache miss can still appear as sourceError while its fetch is pending. This ticker panel is an Orbit extension; ProPresenter's documented Remote interface does not establish ticker seek compatibility.

## Using Bitfocus Companion

Orbit has no dedicated Companion module and does not emulate the ProPresenter 7 HTTP API (its ProPresenter Remote WebSocket listener is described below). The `companion-module-renewedvision-propresenter` module cannot be pointed at Orbit: Orbit does not serve ProPresenter paths such as `/v1/trigger/next` or `/v1/status/updates`, and this API has no WebSocket channel; push feedback is the Orbit-specific event stream described below. What Companion can use today is a generic HTTP client (for example the Generic HTTP module, or any module that can send an HTTP request with a custom header). The contract below is taken from `crates/orbit-host/src/remote.rs` and its tests. It has not been exercised against a running Companion instance, and the Generic HTTP module's option names depend on its version, so check them in your installed module.

Connection:

- Base URL `http://<orbit-host>:<port>`, default port 49321. Orbit listens on all interfaces and uses plain HTTP; there is no TLS.
- Every request needs the header `Authorization: Basic <base64("presenter-orbit:<password>")>`. For the password `secret` the value is `Basic cHJlc2VudGVyLW9yYml0OnNlY3JldA==`. A missing or wrong header returns `401`.
- Send action requests as `POST /api/v1/command` with a JSON body and `Content-Type: application/json` (the server parses the body as JSON and does not check the content type). The request must carry a `Content-Length`, and the whole request is limited to 64 KiB.
- Orbit handles one connection at a time and closes it after each response. Do not poll faster than about once per second, and turn off connection reuse if the module offers that option.

Button actions that need no Orbit identities:

| Action | Body |
| --- | --- |
| Next slide | `{"command":"nextSlide"}` |
| Previous slide | `{"command":"previousSlide"}` |
| Next item | `{"command":"nextItem"}` (queued, `202`) |
| Previous item | `{"command":"previousItem"}` (queued, `202`) |
| Clear all | `{"command":"clearAll"}` |
| Clear one layer | `{"command":"clearLayer","layer":"slide"}`; layer ids are `music`, `messages`, `props`, `announcements`, `slide`, `media` and `videoInput` |

Button actions bound to a target. Read the ids from `GET /api/v1/status` (`timers`, `props`, `audienceLooks`, `macros`, `messages`, `clearGroups`, `recordingPresets`) and copy them into the button once:

| Action | Body |
| --- | --- |
| Timer start, stop, reset | `{"command":"timer","id":"<timer id>","action":"start"}` (`stop`, `reset`) |
| Timer adjust | `{"command":"timer","id":"<timer id>","action":"increment","seconds":-10}` |
| Prop show or clear | `{"command":"prop","id":"<prop id>","action":"show"}` (`clear`) |
| Audience Look | `{"command":"audienceLook","id":"<look id>","workspacePath":"<workspacePath from status>"}` |
| Macro | `{"command":"macro","id":"<macro id>","workspacePath":"<workspacePath from status>"}` |
| Clear group | `{"command":"clearGroup","id":"<group id>","workspacePath":"<workspacePath from status>"}` |
| Message show or clear | `{"command":"message","action":"show","id":"<message id>","workspacePath":"<workspacePath from status>"}`; clear has no `id` |
| Stop recording | `{"command":"capture","action":"stop","workspacePath":"<workspacePath from status>"}` |

`workspacePath` is a Windows path in JSON, so each backslash is doubled (`C:\\Shows\\Sunday`). A button written for one workspace is rejected, not misapplied, after the operator opens a different workspace; re-copy the ids. Cue, library, playlist, media and ticker commands need exact occurrence identities from status and are impractical as fixed buttons; use the browser panel for them.

Feedback. `GET /api/v1/events` is a server-sent-event stream with the same authentication. It sends `event: status` with the `GET /api/v1/status` JSON on one `data:` line when it opens and again whenever that JSON changes (`serverTimeMs` alone does not count); workspace catalog changes are picked up within about a second. Idle streams get a `: keepalive` comment every 15 seconds. At most four streams are open at once; a fifth gets `503`. Streams close when Remote is stopped or its settings change, so clients must reconnect. `GET /api/v1/commands/{requestId}` reports the result of a queued Look, macro, message, stage layout or clear group request (`requestId` comes from its `202` response). A `200` response means native state was accepted; `202` means queued and not yet executed; `400` is an invalid or unavailable target; `401` is bad credentials.

Not provided on this HTTP API: ProPresenter path compatibility and any assessment of Companion behavior against a live instance.

## ProPresenter Remote and Stage Display WebSocket

The opt-in compatibility listener now includes the **current ProPresenter Remote**
transport at `/v1/ws`, alongside the Stage Display endpoint. Renewed
Vision distinguishes the clients in its [connection guide](https://support.renewedvision.com/hc/en-us/articles/42892551789843-How-to-Connect-the-ProPresenter-Remote-App).
The operator confirmed a direct current-client connection and library browsing.
On 2026-10-29, phone testing also confirmed automatic discovery, presentation
thumbnails and active-cue feedback. It exposed a cue-media activation defect:
remote slide selection did not enable the associated media as a local cue click
does. The shared native selector now activates that media, publishes its playback
state and enables the media layer; the renderer retains that layer change.
The operator confirmed the image fix on the phone. Associated media selection now
also resumes the media transport instead of retaining an earlier pause. In the
desktop rehearsal the video clock ran and moving video appeared when only the
slide layer was cleared; the selected slide/theme content obscured the picture.
The operator also confirmed working video and faster cue changes on the phone.
The operator subsequently confirmed faster active-cue feedback, nearly immediate
presentation thumbnails and successful reconnect. Saved-connection selection
while ProPresenter is also running still needs acceptance: the phone shows one
saved entry and can reconnect it to ProPresenter after Orbit was selected.
The operator confirmed that a manual IP/49321 connection remains connected to
Orbit while ProPresenter is also running. The address displayed by Orbit used a
multicast routing probe, which selected the WSL adapter on this Windows machine.
Using a reserved unicast destination for the socket route lookup selects the
LAN address instead; no datagram is sent and mDNS interface advertising is unchanged.
Support remains experimental until that acceptance is complete.

The same-day discovery capture showed distinct Orbit and ProPresenter instance
names, SRV hostnames and ports (49321 and 55435 respectively), on the same PC's
addresses. This does not establish why the phone reuses its saved entry. A manual
Orbit connection with its explicit port remains a useful phone diagnostic.

Modern Remote timer readings now use signed `HH:MM:SS`, retaining native
millisecond precision internally. Identical readings within a second no longer
produce different subscription payloads. Boundary, negative-overrun and elapsed
timer regressions pass, as do all 79 modern Remote tests and `npm run check`.

The eight imported props were compared with the original ProPresenter file:
seven contain no drawable elements there either. The remaining prop (`Prop`)
had an old workspace opacity of 1%, although the original stores full opacity
and the current importer converts it correctly. Restoring that object's opacity
to 100% through the editor made its blue rectangle visible in the output preview
when triggered through the encrypted Remote route. Phone confirmation remains
pending: the operator subsequently reported persistent loading spinners in the
phone's prop panel, which requires separate thumbnail-request diagnosis. No
workspace migration or remote-trigger workaround was added.

The phone trace subsequently showed `v1/prop_collections` requests without any
prop thumbnail requests. With operator-authorized, read-only access to the local
ProPresenter reference server, its encrypted WebSocket response was verified to
contain `data.prop_collections.collections`; Orbit had incorrectly returned
`data.collections`. The collection-list response now matches that envelope,
includes `single_prop_enabled: false` for the native additive trigger behavior,
and supplies the reference's zero `auto_clear_duration` for disabled auto-clear.
Individual prop and collection responses remain unwrapped, as verified against
the reference. A subscription regression checks the exact envelope and active
prop updates. The operator confirmed that props now load after selecting the
nonempty collection; the initially selected collection was empty. A subsequent
phone test exposed missing auto-clear execution for Remote triggers. Native
prop deadlines now use the existing output ticker, restart on retrigger, retire
on accepted manual clear and reset on workspace changes. Expiry commits native
output and publishes active-prop feedback without relying on the Props panel.
The deterministic deadline/retrigger/manual-clear regression and all 253 selected
prop-related Rust tests pass (two hardware-dependent tests ignored), together
with all 80 modern Remote tests, maintenance checks and `npm run check`. In a live
encrypted subscription test, retriggering after two seconds cleared the active
feedback at 7.327 seconds from the first trigger. Phone auto-clear confirmation
and saved-Look activation acceptance are still pending.
A subsequent reference capture during a five-second prop activation showed a
constant `auto_clear_duration: 5` in prop details and collection subscriptions,
with `is_active` changing at activation and expiry; no per-prop decreasing time
field was emitted. The operator accepted leaving the countdown indicator as-is
after this reference comparison; no additional countdown field was introduced.

The live Remote catalog also returns three saved audience Looks (`Default`,
`Default 1`, `Default 2`). Orbit edits them under Settings → Video → Audience
Looks; the current mobile app exposes Looks under More, as described in the
[Remote interface guide](https://support.renewedvision.com/hc/en-us/articles/43051104879379-ProPresenter-Remote-App-Interface).
Their selection from the actual phone still needs acceptance.
The operator could see the three Looks but activation failed locally with an
unavailable `audience-1` reference: that output had been deleted while the saved
Looks retained its mappings. Native screen deletion now removes matching Look
mappings in the same workspace transaction as screen configuration and stage
assignments, with renderer response/event updates. The Look editor also allows
explicit removal of an unavailable mapping from a draft to repair existing data;
activation validation still rejects unavailable screens. With operator approval,
the three existing Looks were repaired through the editor to retain only
`audience-2` and `audience-3`; the authored layers, themes and masks were preserved.

Remote commands reuse the existing workspace-revision cache under the workspace
I/O lock, preserving active-workspace and revision checks. On 2026-10-29, six
local HTTP cue requests alternating text and image slides took 300–357 ms before
this change and 54–100 ms afterwards. Six local encrypted WebSocket triggers with
an active slide-index subscription took 74–120 ms afterwards. These small samples
measure request/response time on this development machine, not phone-network or
physical-output latency. The cache-hit/revision-invalidation and paused-media
selection regressions pass.

For the separate active-cue feedback change, three local encrypted WebSocket
trigger-to-subscription measurements were 503–605 ms before and 199–220 ms after
the status-change refresh was connected. Request acknowledgements and feedback
were measured separately. The real-socket regression checks prompt notification
and the retained periodic refresh; all 76 modern Remote tests passed. These are
local samples, not a qualification of phone or physical-output latency.

The network listener is off by default and shares the configured network port (default 49321, 1024–65535) with the Orbit HTTP API. Remote and Stage App have independent enable switches and passwords. A disabled app or an empty password rejects connections to its endpoint.

While enabled, the listener advertises `_proapiv1ws._tcp.local.` as
the configured Network Name for the current app's server list. The SRV record
uses the shared network port and host addresses update automatically. Its TXT
record contains `path=/v1/ws`, matching the observed current ProPresenter
advertisement. Orbit uses a `presenter-orbit-` hostname prefix so its independently
published address records do not share ProPresenter's machine hostname. Stopping
the listener withdraws the advertisement. Discovery carries no password and does
not remove the password requirement. Discovery startup errors are reported
explicitly. Direct IP/port entry remains available.

### Current Remote

Enter the computer's IP address, network port and Remote password in the
current mobile app. Messages are password-derived AES-256 CTR64BE encrypted JSON
inside Base64 WebSocket text frames. The server sends a `version` envelope after
upgrade. Requests use relative API URLs without a leading slash, for example
`{"url":"v1/libraries","method":"GET"}`. Responses echo the complete `url` and
contain either `data` or `error`. Optional JSON `body` carries timer/stage updates.

Implemented routes cover libraries, nested playlists and presentation details,
exact zero-based cue triggering, active cue/playlist feedback, timer configuration
and runtime control, macros and ordered macro collections, clear groups/layers,
stage screens/layout assignments/messages, saved audience looks and live layer
presence. Additional routes expose props and prop collections, text/clock/timer message tokens,
audio/media playlists and audio transport controls. Required media detail metadata
comes from the source catalog or native media probe. An absent artist is an empty
API string; an unavailable duration produces an explicit error. Public API IDs use stable UUIDs even when an Orbit source ID is not a
UUID. Commands resolve back to native workspace IDs and exact cue occurrences.

Supported read endpoints accept `chunked=true` subscriptions. Accepted status
changes refresh subscriptions after the current request or socket read timeout
(100 ms), retaining the exact request URL. A 500 ms periodic refresh still checks
clocks and data without a status-change signal. Only changed responses are sent. Each
connection allows at most 64 subscriptions, and mutation endpoints cannot be
subscribed. Valid unsupported subscriptions receive a correlated API error without
closing the session and cannot dispatch a mutation. Messages and frames remain
bounded at 1 MiB. Wrong-password or malformed requests cannot execute commands,
and an idle unauthenticated connection
expires after 10 seconds.

`GET v1/timer/system_time` returns Unix seconds. `GET v1/timer/video_countdown`
returns the active video's remaining content time as `HH:MM:SS`, consistent with
Orbit's media transport. The reading respects paused playback, capture time,
rate, trim and loops. A known native out point can bound the reading before total
source duration is available. Missing clock bounds, an inactive video or a stale
selection return explicit errors. Both read endpoints support subscriptions;
special timer names cannot be used to edit or control an unrelated named timer.

Message details include native clock tokens (24-hour local time with seconds)
and timer tokens referencing the accepted timer definitions. A trigger may send
the returned full token list: only text values are editable, and clock/timer
configuration and token identities must remain unchanged. Invalid or retired timer
targets are rejected before any definition edit or message trigger.

Presentation thumbnails use `GET /v1/presentation/{uuid}/thumbnail/{index}` on
the same listener. They return JPEG by default or PNG with `thumbnail_type=png`.
`quality` is the longest image edge in pixels (default 400, supported 1–1920).
The renderer composes the selected native cue and theme near the requested size
(capped at 960 pixels), then resizes to the exact requested dimensions. Requests
preserve arrangement occurrence selection and reject stale workspace/cue targets.
Revision-checked rendered previews are reused before cloning the workspace
catalog. Final JPEG/PNG responses are cached by source image content, requested
size and format, with at most 128 entries and 16 MiB of encoded image data. A full
cache is cleared; oversized responses are served without being retained. These
caches preserve the session-bound access check and do not enable client caching.
Enabling Remote prepares the static-preview font catalog and decoder discovery
once. Each still renderer keeps its own mutable font and drawing state while
reusing that catalog; newly installed fonts or decoder tools require an app
restart for these previews. Frozen video previews remain pending until a frame
is decoded, then finish without waiting out the one-second decode deadline.

In a 2026-10-29 Windows development measurement, the first local 400-pixel
requests for one text, image and video slide after restart took 129, 169 and
229 ms respectively; repeated requests took 49–56 ms. Before sharing preview
resources and finishing decoded video stills promptly, those first requests took
336, 302 and 1347 ms. Before the thumbnail size/cache changes, even repeated
requests took 509–595 ms. These are three-slide local samples; phone-network
latency is separate from these measurements. The operator subsequently confirmed
that thumbnails appear almost immediately on the phone when opening a previously
unopened presentation and reopening it.

The mobile app sends these image requests separately, without an Authorization
header. As explicitly approved by the operator, image access requires a currently
authenticated Remote WebSocket from the same peer IP address. Access ends when
that peer's last authenticated session closes. Another device sharing that IP
address can therefore request images while the session is active. Responses use
`Cache-Control: no-store`; inactive peers receive `403`.

Native routes also provide source-probed audio/video durations, focused presentation and playlist selection, presentation timeline play/pause/rewind, current audience Look identity and theme/mask assignments. Timeline commands validate the selected occurrence, workspace and transport generation, and read the monotonic native playhead. If the current screen configuration differs from its saved Look, the API exposes a distinct read-only Current Look.

Authenticated image access also covers playlist cues, props, media, masks, stage layouts, theme slides, macro icons and clear-group icons. These use native scene composition, source media and the shared Orbit icon catalogue. Macro and Clear Group icon routes return PNG by default to preserve alpha; presentation and other thumbnail routes retain JPEG/PNG negotiation. Unassigned Stage screens are absent from the layout map but remain in the screen catalog.

Unsupported HTTP API extensions outside the selected mobile UI baseline include message definition editing, prop automatic-clear controls, timer creation/deletion, macro editing and announcement timeline transport. Those
requests return explicit errors; they do not report successful no-op behavior.
The implemented CTR transport has no authentication tag and is not TLS; use a
trusted network.

### Stage Display

`/stagedisplay`: authenticate with `{"pwd":"...","ptl":610,"acn":"ath"}`; the reply is `{"acn":"ath","ath":true,"err":""}`.

| Message | Reply |
| --- | --- |
| `asl` | All stage layouts. Orbit layouts are free-form slides, so each is described with the same six standard ProPresenter frames (current and next slide, their notes, clock, message) |
| `psl` | `{"acn":"psl","uid":<layout id>}` of the first stage screen with a layout; an error when none has one |
| `fv` | `cs`, `ns` (uid and text), `csn`, `nsn` (notes), `msg` (visible stage message text) and `sys` (local time) |

Stage displays are pushed a new `fv` when the current or next slide, its notes or the stage message change, and `{"acn":"sys","txt":...}` when the minute changes. Timers (`tmr`), live slide images and static slide image URLs are not provided. Status `currentSlide`, `previousSlide` and `nextSlide` now also carry `notes`.


## Direct TCP/IP

Enable TCP/IP under Network settings and choose its separate port (default 49323). It must differ from the shared network port. The master Network switch controls this listener too. This is an opt-in, unauthenticated local-network socket; the mobile app passwords apply to Remote and Stage App, not TCP/IP.

Requests and responses are single-line JSON terminated by CRLF, following [Renewed Vision’s TCP/IP envelope](https://support.renewedvision.com/hc/en-us/articles/31606866768147-TCP-IP-Connections-with-ProPresenter-API):

```json
{"url":"v1/libraries"}
{"url":"v1/stage/message","method":"PUT","body":"Ready"}
{"url":"v1/timers/current","chunked":true}
```

`url` selects an existing current-Remote compatibility route described above. `method` defaults to GET without a body and POST with a body. `chunked` must be a boolean; only supported read-only routes can subscribe. Replies preserve `url` and contain `data` or `error`. Successful commands without response data do not emit a reply. Subscription updates use the same envelope and publish changed values. Unsupported routes return an explicit error; this does not add the full ProPresenter authoring API.

Connections are limited to eight, messages to 64 KiB, partial frames to five seconds and subscriptions to 64 per connection. Disabling TCP/IP or Network closes existing TCP connections. Route execution uses the same native validation and command paths as the current Remote app.

### Remaining phone acceptance

Resume CTL-12 with saved-Look activation, five-second prop expiry and inactive
feedback, timer start/pause/resume/reset, macros, audience messages, media/audio
transport and clearing, and stage layouts/messages where used. Discovery, manual
connection, browsing, images/video, thumbnails and cue feedback were confirmed.
Saved-connection coexistence with ProPresenter remains unresolved; explicit
IP/49321 connection works. Record the client versions with the next phone run.
