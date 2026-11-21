---
name: ui-design
description: Build new Presenter Orbit UI (panels, dialogs, inspectors, toolbars, lists) with the project design system. Use when adding or extending any GPUI view, primitive, or theme value.
---

# UI design for Presenter Orbit

Presenter Orbit is broadcast software operated live. Optimize for density, scan speed, and unambiguous state. Read `docs/design-system.md` first; it defines every token and primitive referenced here.

## Workflow

1. Find the closest existing view in `crates/orbit-app/src/` and reuse its structure (lists in `collections/`, inspectors and tabs in `cues/` and `media/`, settings rows in `settings/`, menus in `orbit_ui::controls::ContextMenu`). Open `cargo run -p orbit-app -- --showcase` to see the primitives.
2. Compose from `orbit_ui::controls`: `PanelHeader`, `ActionButton`, `IconButton`, `Field` with `TextInput`/`Select`/`NumberInput`/`RangeInput`, `SegmentedControl`, `TabList`, `Disclosure`, `EmptyState`, `Dialog`, `ContextMenu`, `Popover`, and `orbit_ui::icons::icon`.
3. Lay out with GPUI flex and the `Metrics` fields only. Read every color from `cx.theme().colors` and every spacing, height, radius, and font size from `cx.theme().metrics`; never multiply by a scale yourself.
4. Map each state to its single color: live `colors.live`, selected `colors.selected`, drop/insert `colors.accent`, warning `colors.warning`, clear/destructive `colors.danger`.
5. Add user text to the English and Dutch `.arb` files in `crates/orbit-ui/locales` with English metadata, and read it through the `Translator`.
6. If you created a reusable pattern, add it to `crates/orbit-ui/src/controls/`, the showcase (`crates/orbit-app/src/showcase.rs`), and `docs/design-system.md`.
7. Keep host calls and normalization in a controller; never block the UI thread.

## Do not

- Wrap content in a bordered, padded, shadowed card inside a panel.
- Add gradients, glows, blur, pill shapes, decorative icons, or text glyphs as icons.
- Add uppercase eyebrow labels, hero headings, or subtitles under headings.
- Center content in a work surface, or add marketing-style empty states.
- Add more than one primary action per view.
- Build a color from `rgb(0x...)`, `hsla(...)`, or a hex string in `orbit-app` (the check rejects it outside specialist files).
- Raise a baseline in `scripts/maintenance-baseline.json`.

## Verify

Run `npm run check` and the tests of the crate you touched. Check the showcase or the app in the dark, light, and red themes, compact and spacious density, and with long Dutch labels.
