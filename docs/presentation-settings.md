# Presentation settings

With no object selected, the slide editor exposes presentation settings in the
Presentation tab and individual slide settings in the Slide tab.

- Presentation `size` contains `width` and `height` in pixels. Changing it writes
  those dimensions to every existing slide; objects retain their coordinates.
- Presentation `background` and `backgroundEnabled` are applied to every existing
  slide. New blank and media slides receive the stored presentation settings.
  Individual slides can subsequently override their background and dimensions.
- `ccli.artistCredits`, `ccli.album`, `ccli.display`, and `ccli.artwork` use the
  metadata names already read by the ProPresenter importer. Artwork is embedded
  as base64 PNG or JPEG data, so reopening a workspace does not depend on the
  originally selected file. Selection is limited to 4 MiB and 4096 × 4096 pixels.
- `ccli.display: false` suppresses linked CCLI text.
- A slide's `shortcut` is a single ASCII letter (A–Z), stored in lowercase.
  Enabled slides with an explicit shortcut take precedence over group shortcuts.
  Duplicate slide shortcuts within a presentation are rejected. Pasting or
  splitting a slide does not copy its shortcut.

These settings use the existing workspace mutation lane and revision checks.
Malformed changes are rejected before persistence. Orbit stores this data in its
own presentation JSON; this change does not extend ProPresenter export support.

Presentation, Slide, and Build settings share the same immediate editor preview while ordered saves run in the background. Switching tabs commits the focused metadata field before removing it. Later edits remain visible while earlier saves complete. A rejected save clears the pending preview and reports the error; an unsaved canvas draft is retained.
