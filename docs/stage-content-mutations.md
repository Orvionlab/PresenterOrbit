# Stage content mutations

Stage layouts and the stage message are persisted through semantic workspace
mutations. The renderer owns editing intent and immediate UI feedback; Rust owns
validation, stable identifiers, revision checks, and the staged filesystem
transaction.

`mutateStageContent` supports:

- creating a layout with a native collision-free layout and slide identifier;
- replacing one layout by its stable identifier;
- changing message text without replacing message visibility;
- changing message visibility without replacing message text;
- clearing the message atomically.

Stage-screen assignments are deliberately outside this contract. They remain
owned by the native screen-configuration mutation, which commits the hardware
configuration and assignment together.

UI mutations are serialized per application instance. Native responses
are merged only into the field they acknowledge, and an older response cannot
replace newer optimistic text or layout edits. A workspace revision conflict is
reloaded and retried once. If an active workspace exposes neither the semantic
mutation nor the legacy full-document save, the controller refuses to present a
local-only edit as persisted.

The legacy `saveStage` bridge remains temporarily available for hosts that do
not expose semantic mutations and for old screen-assignment fallback paths. It
is not used by the stage message, layout creation, or stage-layout editor paths
on the current host.
