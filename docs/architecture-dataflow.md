# Architecture and data flow

This guide describes the boundaries between the GPUI app, shared UI controls,
native host services, and program output. See the [design system](design-system.md)
for UI conventions and the [maintenance guide](maintenance-guide.md) for
contributor checks.

## Runtime ownership

```text
GPUI views (crates/orbit-app/src/<feature>/)
    │ state entities, intents
    ▼
Controllers and pure helpers (crates/orbit-app/src/state/, <feature>/)
    │ Host::invoke(method, args) off the UI thread, canonical result back
    ▼
Host (crates/orbit-host/)
    ├── workspace/settings storage and revisions
    ├── persisted settings validation and normalization
    ├── presentation timer and automation scheduling clocks
    ├── native output RSS fetching, caching, and scene resolution
    ├── output windows and screen routing (WindowSystem trait)
    ├── audio and native NDI output
    └── native dialogs and shell integration (DesktopIntegration trait)
```

Program output runs in a separate process (`crates/orbit-app/src/output_process/`); the UI process
is the host's `OutputRole::Controller` and is the only writer of workspace and settings files.

- GPUI views own rendering, focus, and direct user interaction. They never call storage or
  output services directly and keep domain rules out of `Render` implementations.
- Controllers own selection, state normalization, persistence decisions, and cross-state
  behavior. Rust owns normalization at native persistence and network boundaries.
- UI state is a set of GPUI entities (`orbit-app/src/state/`); a change re-renders only the
  views that observe that entity.
- Host calls block, so the UI calls them from `cx.background_spawn` and drops stale results.
- The host keeps its `Value`-based dispatcher (`Host::invoke`, `crates/orbit-host/src/host_api.rs`);
  it validates every payload.
- Native settings responses are normalized before they cross the bridge. The
  theme editor may request an explicit RSS preview through the typed bridge;
  program output instead resolves selected theme-slide RSS in Rust through a
  bounded last-good cache and never mutates the accepted raw snapshot.
- `updateOutputScene` is the single accepted state boundary for program output
  and remote status. Rust owns the v2 session/sequence protocol, full/patch
  cache, native revision, canonical hash, scene construction, and derives the
  `/api/v1/status` payload and automation cue targets from that same accepted
  snapshot. The UI only collects state and computes cheap top-level
  patches. Rust executes remote clear and live-slide navigation commands under
  that same coordinator and sends revisioned patches back to a connected
  authoring store. Document/playlist item navigation remains UI-owned;
  the UI does not construct or push a second remote-status projection.
  Audience scenes composite active announcements over the main slide when
  the look enables announcements. A manual announcement also enables its
  layer on the previewed audience screen for that take. Clearing the player
  leaves saved looks intact. Announcement-only screens use the same player
  as their base scene. Direct takes during a destination-only save resolve
  against the committed workspace cache and the active output snapshot.
- RSS fetch completion rebuilds effective scenes from the latest accepted raw
  snapshot. It does not advance the UI session, client sequence,
  canonical snapshot hash, or native snapshot revision.
- The UI registers only its automation session through the typed
  automation bridge. Rust derives the live timeline,
  atomic cue-start clock, secondary live-cue actions, builds, and completion
  directly from the accepted output snapshot and owns all due-time evaluation
  plus delayed action/media-marker/auto-clear queues and session/sequence
  idempotence. Presentation and delayed-action deadlines are converted to
  native monotonic deadlines when accepted. A replacement UI session cannot
  use presentation state until a fresh output snapshot has been accepted.
  Timeline and completion events include the
  exact native-resolved slide and arrangement-occurrence ids; the UI
  receives secondary cue actions exactly once per activation, consumes those
  typed due events, and applies their interactive store effects. Native MIDI
  input (`midi_input.rs`) opens only while a binding has a MIDI note and emits
  note-on events that the UI dispatches through the binding runtime.
  External-presentation actions start through `external_presentation.rs`,
  which drives PowerPoint (COM on a dedicated STA thread) or Keynote and
  PowerPoint (AppleScript on macOS); the UI routes next/previous slide
  commands to a running slideshow and polls its status.
  The authored timeline transport (generation, nullable running anchor,
  position) is the single clock for the timeline soundtrack: Rust derives the
  playhead from it (`native_timeline.rs`), `native_soundtrack.rs` follows it on
  the main audio output from the 25 ms ticker (seek on a new generation, drift
  correction, loop wrap, unload on a stopped or retired transport) and reports
  playback errors as `timeline-soundtrack-status`. Timecode (`timecode_input.rs`
  with the LTC decoder in `timecode_ltc.rs` and the MTC parser in
  `timecode_mtc.rs`) is decoded on its own native thread and published as
  `timecode-reading` and `timecode-status` events; the UI's chase
  turns them into transport changes for the selected
  presentation, so the soundtrack and cue clock follow timecode through the
  same transport. Waveform overviews are computed and cached natively on request.
- Accepted raw output, UI protocol metadata, and the native snapshot
  revision share one Rust coordinator lock. Cache invalidation preserves both
  protocol ordering and the monotone revision. Every incremental UI
  patch names its accepted native base revision; Rust rejects a mismatch as a
  cache miss. Full reconnect snapshots are revision-guarded too. A cache miss
  returns the current native snapshot so the UI can preserve independent
  native clears/playback changes while rebasing only its newer local fields.
- Standalone due output actions for clearing audio, backgrounds, props, or
  messages, visual or audio media selection without markers/effects, and media
  transport pause/play are executed inside that native coordinator. Rust
  updates the raw selection, message state, or playback snapshot, rebuilds all
  output consumers, advances the snapshot revision, and emits a typed
  UI-state patch. Message identity and a per-display instance id travel
  with the native snapshot so a delayed clear cannot erase a newer display and
  cache recovery can reconcile the UI selection too. The commit holds
  the output and automation boundaries together and rejects due events from a retired session
  or replaced live cue. Interactive media and music pause/play commands use
  the same revision boundary and return their patch to the current UI
  bridge. Absolute seek and relative skip mutate only the Rust-owned playback
  clock, but now
  advance the same cached native revision and return an empty typed patch so
  the UI can re-anchor and replay an in-flight authoring intent without
  inventing a browser-owned output clock. A failed output fan-out invalidates
  the staged cache while retaining its monotone revision, forcing the UI
  to re-anchor with a full snapshot. Toggle reads the current native clock, including natural
  end, instead of trusting the older authored paused flag. Changed seek/skip
  commands reset queued NDI audio before resynchronizing the fallback decoder;
  no-op commands do not interrupt that decoder. Mixed action batches keep their
  authored execution order.
- Dynamic native-audio ticks, audio/screen settings application, NDI content
  updates, and output creation use that coordinator before reading or replacing
  output content. An older playback clone therefore cannot overwrite a newer
  native pause/play commit.

## State update flow

For a normal command such as selecting an output display:

1. A GPUI view emits an intent to its controller.
2. The output controller validates the command and creates the next screen
   configuration.
3. The state entity update notifies the views that observe it.
4. The controller sends a semantic screen mutation through the combined
   settings/workspace host queue. The mutation contains a stable screen
   identity and field patch (or an add/delete, group-toggle, look-layer, or
   stage-assignment intent), never a UI-owned full snapshot.
5. Rust revision-checks the current global/effective workspace map, owns
   screen IDs and canonical defaults, and commits the accepted map together
   with any stage-assignment change. It returns that canonical result before
   the UI accepts a later mutation; runtime output synchronization is
   gated to fields that affect native windows or NDI.

Keep steps 2–4 in a controller. A view should not reproduce
normalization or persistence behavior.

Media-library duplicate/delete commands are an exception to the generic
controller-computes-next-state pattern: Rust reads the revision-accepted
workspace, applies the complete library/media mutation, persists both indexes,
and returns the canonical arrays. The media persistence lane applies that
response before executing later snapshots and refreshes stale revisions before
retrying this idempotent intent. The UI owns only menu and selection state.

Regular and smart media-library create, duplicate, rename, reorder/reparent,
collapse, and asset-add actions use the same semantic mutation lane. Rust
allocates the final title and ID for creates and duplicates, validates stable IDs
and hierarchy/media-type intent, persists authored metadata and the canonical
asset index, and reports `changed` so a no-op does not advance the workspace
revision. Smart-library duplicates persist only their metadata and source path;
their media rows remain derived by the native scanner and are absent from
`Media/index.json`, with source-path keyed settings applied after every reload.
The UI keeps only transient prompts, clipboard/selection state, and a
no-workspace UI fallback.

Presentation-library rename and reorder commands use the same semantic native
boundary. Rust applies the ID-relative intent to the revision-accepted
workspace, returns the canonical workspace, and leaves UI selection state
alone. A stale revision is reloaded and stable-ID intents are replayed once;
position-based arrangement/section intents stop on the refreshed canonical
workspace because replaying an old index could mutate a different occurrence.
Validation and disk errors are never retried. Library storage-folder collisions are rejected before
the staged generation can overwrite another library. Create allocates its
localized title and collision-free ID against that accepted workspace. Bulk
delete removes the selected libraries, their global presentation items, shared
library references, and playlist presentation entries in one Rust transaction.
Rust returns deterministic deleted IDs so the UI updates only transient
selection and live-cue state.

Presentation document lifecycle mutations use the same revision-guarded
`mutatePresentationLibrary` transaction. `copyItems` deep-copies canonical
items or explicit clipboard snapshots, allocates globally collision-free copy
IDs and titles, and inserts their library references at an optional stable
anchor. `moveItems` validates every item and target before removing all stale
memberships and appending one canonical target membership. `deleteItems`
removes the documents, every library membership, and every presentation
playlist occurrence together while preserving headers, placeholders, and
placeholder links. Rust returns copied, moved, and deleted IDs; The UI keeps
only optimistic fallback snapshots and transient selection/live cleanup.

Text-import presentation creation is one native lifecycle mutation too. The
UI sends the target library, source text, split configuration, optional
metadata, and optional playlist title. Rust derives the canonical slides,
document identity, library membership, and import playlist in one persisted
generation and returns the allocated IDs for UI-only selection state.

Edits within a presentation document cross the same bridge as a nested
`mutateItem` operation. Rust validates and persists item titles, operator notes,
batched slide contents, enabled state, media/theme/group assignment, reorder,
deletion, split, merge, paste, blank-slide insertion, and media-slide insertion, then
returns the canonical workspace. Rust allocates structural slide IDs and returns
created/selected metadata for optimistic UI reconciliation. Stable field
edits may be replayed after a revision refresh; content, structural, delete, and
position-based reorder intents stop on the refreshed workspace so stale content
or anchors cannot be applied to a different slide. The UI retains
selection, editor cursors, group-menu state, and drop targeting.

Presentation-theme assignment for libraries and playlist nodes is a native
stable-ID batch mutation. The bridge sends all selected collection IDs plus an
explicit theme, `null` for no theme, or `inherit`; Rust validates the complete
batch against the accepted theme collection and writes it in one revision.
The UI retains only the optimistic context-menu and selection patch.

Presentation arrangements and their ordered section occurrences use this same
transaction. Create, duplicate, rename, reorder, default selection, delete,
and section add/insert/remove/reorder are validated and persisted in Rust.
Native create/duplicate allocate canonical arrangement IDs and names, while
authored duplicate, unknown, and empty section IDs remain lossless. Deleting an
arrangement clears its matching default and every matching playlist-entry
override together in one workspace generation; headers, placeholders, links,
and unrelated overrides are preserved. Playlist-entry arrangement selection
uses the entry mutation boundary instead of a full workspace snapshot.

Top-level presentation-playlist creation, rename, tree reorder/reparent,
collapsed state, duplication, and descendant deletion use the same semantic
intent boundary. Rust allocates collision-free identities against the accepted
workspace, applies the tree mutation transactionally, and returns canonical
playlist data plus created/deleted IDs. The UI retains only optimistic
selection, menu, and inline-rename state.

Presentation-playlist entries use that native semantic boundary as well.
`mutatePresentationPlaylist` handles `createEntry`, `renameEntry`, `addEntry`,
`copyEntry`, `deleteEntry`, `reorderEntries`, `setEntryArrangement`, and
`setPlaceholderLink`. Rust validates the entry/playlist targets, allocates
collision-free entry identities and titles, persists the ordered entries, keeps
`itemIds` derived from presentation entries, and returns the canonical playlist
snapshot with operation-specific entry IDs. `addEntry` validates referenced
library items and its skip-existing intent; `copyEntry` can copy a native source
entry or a UI clipboard snapshot.

The UI retains transient selection and anchors, live-cue cleanup state,
context/create/link popups, and inline-rename state around the native response;
the persisted playlist data remains Rust-owned. If a legacy playlist contains
duplicate entry IDs, rename supplies the entry index and expected current title
as an occurrence anchor. Delete supplies the index, ID, and complete expected
entry snapshot for every selected occurrence. Arrangement selection likewise
carries the index and complete expected entry. Reorder, placeholder-link, and
copy mutations carry exact source/target occurrence indexes where the intent
addresses an existing entry. Rust validates these anchors before changing the
playlist, so a stale retry cannot rename, delete, link, reorder, or copy from
or into a different occurrence. Deleting an entry removes only that playlist
occurrence. Removing an
arrangement object or section is handled by the presentation-library mutation
transaction described above.

Props collection CRUD uses the same revision-guarded semantic boundary through
mutateProps. Native Rust owns prop and folder creation, canonical prop IDs,
collision-free names for newly-created entries, duplicate/paste ID allocation,
folder moves, auto-clear validation, rename, and deletion. Each accepted
operation returns the canonical props/folders workspace snapshot. Prop creation
and theme-editor document updates also use this boundary: paste allocates native
identities for a new authored snapshot, while `updateDocument` validates the
stable prop and slide IDs and replaces only its authored slide. No-op moves
and unchanged auto-clear values do not advance the workspace revision. The
native create path preserves the UI's complete default theme-text style,
while duplicate and paste preserve the authored snapshot title and slide name
and allocate only new identities. The UI retains only prompts,
clipboard snapshots, selection, and live-prop cleanup. With no active workspace
or an older bridge, the existing in-memory/full-save fallback remains
available; the active host path sends only the semantic intent.

Mask documents follow the same boundary through `mutateMasks`. Native create
owns mask and slide identity; native layer insertion owns the final layer ID.
Document edits carry only the changed title, size, and/or slide fields, and
Rust validates the stable mask/slide targets plus unique layer IDs before one
workspace transaction. The UI keeps a per-mask debounce and operation
lane solely for optimistic editor state and response ordering; the persisted
mask collection and its identities remain Rust-owned. Conflict recovery uses a
non-activating native workspace snapshot, so an old request cannot reactivate a
workspace after the user switches. Mask filenames are disambiguated
case-insensitively before disk writes, including titles that sanitize to the
same filename.

All semantic workspace mutation conflict handlers use the same
`readWorkspaceSnapshot` boundary. `loadWorkspace` is reserved for explicit
workspace activation and initial loading; background settings events and
revision recovery therefore cannot change the active workspace as a side
effect. `getSettings` activates the configured workspace only while none is
active, because every window and revision recovery calls it.

## Workspace write flow

```text
Controller
  └── shared bridge request
       └── Host command validation
            └── workspace store
                 ├── normalize version 1 data
                 ├── stage the complete JSON and asset generation
                 ├── journal and replace workspace files in commit order
                 ├── recover an unfinished generation on the next read
                 └── read the persisted workspace back
```

The current development workspace format remains strict version 1. Do not
introduce version 2 merely to prepare for future compatibility. When a real
schema change is required, add its migration, backup behavior, old-version
fixture, and idempotency tests together.

Global settings have a separate Rust-owned operation coordinator and monotonic
in-process revision. `getSettings` returns the current revision; every UI
save, audio apply, and screen-config apply supplies it as the base revision.
Rust rejects stale callers before writing or touching native runtime state, and
advances the revision only after a successful settings write. UI queues
preserve invocation order inside each window, while the Rust revision remains
the correctness boundary across windows.

Group add, rename, color/key updates, and delete use the typed
`mutateGroupSettings` semantic intent. Its UI queue waits for both the
global settings tail and the selected workspace tail, reads both current
revisions at execution time, and makes exactly one native call. A workspace
delete removes canonical and legacy group metadata, clears matching slide
assignments, and applies UI-supplied localized fallback labels in the
same native transaction. The accepted response records each returned revision
and carries canonical group settings plus the workspace snapshot when one was
supplied; a global-only call sends no workspace revision and does not mutate
library content.

An add intent can include one presentation item and slide assignment. Rust
allocates the final group identity across the global and workspace scopes,
preserves unrelated entries in each scope, adds the same canonical group to
both, and assigns the slide before either file is committed. The UI uses
this path for the cue-menu “new group” workflow when a workspace is active;
prompting and optimistic state remain UI concerns. Without a workspace,
the existing UI persistence path remains necessary because there is no
native workspace document in which to store the slide assignment.

Shortcut edits use the typed `mutateKeyMappings` intent instead of replacing a
UI-computed settings snapshot. Rust validates the command identity,
unbinds every conflicting command, and applies the intent independently to
global settings and the active workspace under their respective revision
guards. Scope-specific entries remain intact; an inherited empty workspace
override is materialized only by a real edit. If the workspace write fails
synchronously, the settings write is rolled back. Generic settings persistence
omits key mappings, group settings, countdown timers, and screen configuration
whenever their semantic native bridges are available, preventing a stale broad
save from overwriting a newer Rust-owned mutation.

## Failure boundaries

- Recoverable startup tasks become translated application status.
- Startup failures become a translated failure surface with technical details; a panic is
  logged by the panic hook and not swallowed.
- Host commands reject malformed requests before they reach storage or output
  services.
- Workspace saves stage every JSON file and copied asset before touching live
  data. A validated relative-path journal and per-file backups allow an
  interrupted uncommitted generation to roll back; a durable commit marker
  makes recovery retain the new generation. Indexes and `workspace.json` are
  committed after their referenced data.
- Hardware-dependent display, audio, media, and NDI behavior remains part of
  the operator release checklist.

## Adding a host capability

1. Add and validate the method in the host dispatcher (`crates/orbit-host/src/host_api.rs`) or a
   focused Rust module in `crates/orbit-host/`.
2. Add a typed wrapper in `orbit-app` next to the feature that needs it, and call it from a
   controller off the UI thread. If the output process needs it, add it to the output invoke
   allow-list only when it is not a workspace or settings writer, and record the decision in this guide.
3. Do not add an unstructured settings or workspace dictionary; use typed payloads.
4. Add malformed-payload, success, and failure tests.
5. Run `npm run verify`; manually test hardware-dependent behavior when applicable.

## Relevant verification

- `npm run check` is the quick gate: toolchain and workflow-pin checks, the orbit-app color-literal
  check, locale consistency, version agreement, workspace compatibility, formatting of scripts, `cargo fmt`,
  and clippy (`scripts/check-clippy.mts`: none in `orbit-app` and `orbit-ui`, `orbit-host` within its baseline).
- `npm run verify` adds the [operator end-to-end tests](e2e-testing.md).
- `node scripts/gpui-smoke.mts --dir <unpacked package>` starts a packaged build on a private data folder.
