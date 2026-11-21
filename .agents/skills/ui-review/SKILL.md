---
name: ui-review
description: Review a Presenter Orbit UI change or screen against the design system and live-operation usability. Use for "review this UI", design reviews of diffs touching crates/orbit-app or crates/orbit-ui views, or auditing a screen.
---

# UI review for Presenter Orbit

Review against `docs/design-system.md`. Report findings ranked by operator impact; each finding names `file:line`, the rule, and the concrete fix. Do not edit unless asked.

## Scope

For a diff: `git diff --name-only main... -- crates/orbit-app crates/orbit-ui`. For a screen: its views under `crates/orbit-app/src/<feature>/`.

## Checks, in priority order

1. **Live state clarity**: live, selected, drop, warning, and danger each use their own color field and never share one. Program output has a tally frame. Go-live and clear actions are distinguishable and keyboard reachable.
2. **Primitives**: no hand-built tabs, segmented groups, collapsibles, panel headers, empty states, or dialogs outside `orbit_ui::controls`; checkboxes use `CheckboxField` or `ToggleSwitch`.
3. **Theme values**: run these over the changed Rust files and expect only specialist-surface hits:
   ```bash
   rg -n 'rgba?\(|hsla?\(|0x[0-9a-fA-F]{6}|"#[0-9a-fA-F]{3,8}"' <files>
   rg -n 'px\(\s*[0-9.]+\s*\)' <files>
   rg -n 'font_weight|text_size\(' <files>
   rg -n 'unwrap\(\)|expect\(' <files>
   ```
4. **Decoration**: cards in panels, gradients outside meters/swatches/checkerboards, glows, pills outside switches and slider thumbs, uppercase outside tab and column headers, subtitles under headings, centered empty states.
5. **Density and consistency**: rows at `metrics.row_height`, panel headers at `metrics.header_height`, toolbar controls at `metrics.control_height_compact`, labels left of fields in inspectors, one primary action.
6. **Boundaries**: feature code does not restyle primitive internals; host calls run off the UI thread and stale results are dropped; no logic in `Render`.
7. **Themes and access**: no literal overlays that break the light theme; visible focus; tooltips or labels on icon-only buttons; all text translated in English and Dutch.
8. **Gate**: `npm run check` passes and no baseline in `scripts/maintenance-baseline.json` was raised.

## Output

A ranked list: severity (blocks live use / inconsistency / polish), `file:line`, finding, fix. End with what you did not check (for example, themes not viewed in the running app).
