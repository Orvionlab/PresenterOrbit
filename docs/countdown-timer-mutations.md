# Countdown timer definition mutations

Persisted countdown timer definitions are owned by the Rust workspace writer.
The renderer sends one semantic `CountdownTimerMutation` at a time through the
typed `PresenterOrbitBridge.mutateCountdownTimers` method:

- `create` accepts a base name and the native host allocates a collision-free
  `timer-*` id. The requested name is preserved exactly after trimming.
- `update` applies a validated definition patch by stable timer id.
- `reorder` moves one timer before or after another stable id.
- `delete` removes one definition by stable id.

Every mutation includes the renderer's workspace base revision. Rust rejects a
stale revision before reading or writing and returns the canonical workspace
snapshot plus the new revision after a successful staged transaction. The
bridge serializes mutations per workspace. On a revision conflict the renderer
reloads once and replays the same semantic intent, so a retry never reuses a
stale full-array snapshot.

Definition actions are also serialized before their next array is derived.
This keeps the older full-array compatibility path from overwriting a preceding
edit with stale renderer state.

Delete and timer-type changes also report `removedTimerIds`. Rust clears the
corresponding in-memory run only after the workspace commit succeeds; a failed
write therefore leaves both the definition and its active run intact. Loading
or switching workspaces resets native and renderer timer runs to prevent a run
from one workspace being applied to a timer with the same id in another.
Runtime controls validate their timer against the persisted definition under
the same workspace-to-timer lock order. The renderer additionally rejects a
late run response when its definition was deleted or changed in the meantime.

The legacy `saveProps` path remains available for older hosts and only writes
timer definitions when a timer array is explicitly supplied. Current hosts use
the semantic timer mutation path for the timer panel, while React continues to
own presentation and input rendering.
