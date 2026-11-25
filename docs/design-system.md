# Presenter Orbit design system

This document is the source of truth for application UI. Presenter Orbit is operated live, under pressure: density, scannability, and unambiguous state come before decoration.

The system is `crates/orbit-ui` (theme, metrics, primitives, icons, translations) on top of GPUI. Slide content, output rendering, canvas geometry, media mattes, rulers, meters, color pickers, and masks are specialist surfaces and may use fixed rendering values.

## Principles

- Reuse a primitive from `orbit_ui::controls` before adding feature-specific control markup. A new reusable control goes in `orbit-ui` and appears in the showcase.
- Primitives own appearance, interaction states, accessibility defaults, and semantic variants. Features own layout and domain behavior.
- Read color from `cx.theme().colors` and sizes from `cx.theme().metrics`. Never build a color or a scaled size from a literal in `orbit-app` (`scripts/check-architecture-boundaries.mts` rejects color literals outside the specialist files).
- Preserve visible focus, labels, keyboard access, disabled state, and sufficient contrast in every supported theme.
- User-facing text and accessibility labels use ARB translation keys (`crates/orbit-ui/locales`). Technical identifiers and persisted user data remain stable.

## Tokens

`Theme::resolve(settings, system)` builds `Colors` and `Metrics` from the theme (dark, light, red, system), the accent, the density, and the text scale. Spacing and control sizes scale with density; font sizes scale with text scale. `Metrics::new` is the only place that multiplies by a scale; views read the fields. The focused operator E2E suite exercises the theme, density, and device-scale combinations used by the main workspace.

| Purpose | `Metrics` fields (base size in logical px) |
| --- | --- |
| Spacing | `space_2xs` 2, `space_xs` 4, `space_snug` 6, `space_sm` 8, `space_md` 12, `space_lg` 16, `control_gap` 8, `panel_gap` 12, `panel_padding` 14 |
| Heights | `row_height` 22 (list/tree rows), `control_height_compact` 24 (toolbars, inspectors), `control_height` 30 (forms, dialogs), `header_height` 24 (panel headers), `tabbar_height` 28 |
| Radius | `radius_xs` 2 (rows, cards, thumbnails), `radius_sm` 3 (controls), `radius_md` 4 (popovers, dialogs), `radius_pill` (switches and slider thumbs only) |
| Font size | `font_xs` 11, `font_sm` 12, `font_md` 13, `font_base` 14, `font_lg` 15, `font_xl` 18, `font_2xl` 22, `font_3xl` 30 |
| Elevation | none for in-layout panels; `theme.shadow_soft()` for popovers and menus; `theme.shadow()` for dialogs |

| Purpose | `Colors` fields |
| --- | --- |
| Surfaces | `bg`, `shell_bg`, `panel_bg`, `panel_bg_strong`, `panel_bg_soft`, `surface_bg`, `surface_raised_bg`, `field_bg`, `control_bg`, `control_bg_hover`, `popover_bg`, `stage_bg`, `canvas_bg`, `preview_matte_bg`, `overlay_bg` |
| Text | `text_strong`, `text`, `text_muted`, `text_subtle`, `disabled_text`, `accent_text` |
| Lines | `border_soft`, `border`, `border_strong`, `control_border`, `menu_separator` |
| Accent | `accent`, `accent_hover`, `accent_soft`, `accent_selected`, `focus_ring` |
| Canvas aids | `canvas_transparent_bg` for the solid transparency preview, `slide_guide` and `object_guide` for the default guide colors |

Translucent overlays use the theme's own alpha colors (`overlay_bg`, `menu_separator`, `menu_icon_bg`) so they survive the light theme; do not blend a literal white or black.

Color pickers near a bottom toolbar can open above the swatch. With close-on-selection enabled, choosing a swatch closes the palette and the advanced tab offers a Done action.

## State colors

Each state has exactly one color and is never reused for another meaning.

| State | Field | Treatment |
| --- | --- | --- |
| Live / on air | `colors.live` | 2px inset frame or solid fill; program preview carries a 2px tally frame |
| Live announcement | `colors.live_announcement` | Green frame around the active announcement cue |
| Selected | `colors.selected` | background fill for rows, 2px inset frame for thumbnails |
| Drop target / insertion | `colors.accent` | 2px line, no glow |
| Warning | `colors.warning` | real warnings only |
| Clear / destructive / layer active | `colors.danger` | solid fill |
| Success / connected | `colors.success` | indicators only |
| Disabled | `colors.disabled_text`, reduced opacity | |

## Layout rules

- Separate panels with a 1px border or one background step. Never nest a bordered, padded, shadowed card inside a panel.
- Every panel has one header built with `PanelHeader`: `header_height`, `font_sm`, medium weight, sentence case, trailing compact icon buttons.
- Lists use `row_height` rows with no per-row border; group headers use a 3px leading color bar.
- Inspectors place labels left of compact fields; forms in dialogs may stack.
- Uppercase is reserved for tab and column headers. No eyebrow labels and no explanatory subtitles under headings; put help in a tooltip or a single muted line only when it explains a non-obvious consequence.
- One primary action per view. Buttons in toolbars are compact and transparent until hover.
- Empty states are a single muted line inside the region (`EmptyState`), not a centered illustration.
- Gradients are allowed only where they carry meaning (meters, color and opacity swatches, checkerboards, rulers).
- Motion is functional only: no decorative transitions.

## Primitives

| Need | Use |
| --- | --- |
| Action | `ActionButton` (`ButtonVariant` primary, secondary, ghost, danger, soft; `.compact()` for toolbars), `IconButton` for icon-only (`.inline()` inside a chip), `ToolButton`, `BarButton`, `BrandButton` for the bars |
| Form field | `Field` with `TextInput`, `Textarea`, `Select`, `NumberInput`, `RangeInput` (`RangeInput::scrub` for transport seeking), `CheckboxField`, `ColorPicker` / `ColorPopover`, `OutputField` for read-only values |
| Name filter | `FilterInput` with `filter_state`, with a funnel icon and localized placeholder; keep its bar outside scrolling item lists |
| Boolean | `CheckboxField` in forms (`.bare()` for compact grid cells with accessible names), `ToggleSwitch` for immediate on/off (its translated label sits beside the switch and both activate the same handler) |
| Exclusive choice | `SegmentedControl` |
| Tabs | `TabList` with `TabItem` (`TabItem::leading_icon` shows an icon beside the label) |
| Collapsible section | `Disclosure` |
| Panel header | `PanelHeader` (`.contrast()` for stronger separation between adjacent list sections) |
| List row | `list_row`, or `list_row_without_hover` when only selection changes its background |
| Empty region | `EmptyState` |
| Modal | `Dialog` (the only dialog skin), `BusyDialog` while waiting (`.page()` for centered full-page loading) |
| Menu | `ContextMenu` with `MenuItem` and `MenuEntry`; `Header` groups destinations, `Control` embeds an interactive field that keeps the menu open, and `MenuItem::content` supports rich previews while preserving the accessible label and action |
| Anchored overlay | `Popover` |
| Collection selector | `CollectionPicker` with `CollectionOption` and `CollectionAction`: add, inline rename, duplicate, confirmed delete and drag insertion in one popover |
| Slide text and notes | `RichText` (shared field colors, compact formatting controls, focus ring, IME, commit on blur; weight, style, decoration and color per run) |
| Icon | `icon(...)` from `orbit_ui::icons` (Tabler set); no text glyphs as icons |

Specialist controls whose pixels carry meaning (canvas handles, rulers, meters, mattes) draw with GPUI elements directly and use theme colors for chrome. Add a short comment when the exception is not obvious.

## States and layout

Every new primitive must define normal, hover, focus-visible, active/selected, and disabled states. Add danger, warning, success, read-only, or validation states only where the behavior exists. Feature code must not restyle the internals of a shared primitive; add a variant to `orbit-ui` instead.

Layouts must work at compact, comfortable, and spacious density; every text scale; the dark, light, red, and system themes; and window widths from 1120 through 1920 pixels. Prefer flexible layouts and intentional scrolling over fixed clipping, and check long Dutch labels.

## Enforcement

- `scripts/check-architecture-boundaries.mts` (part of `npm run check`) rejects color literals in `orbit-app` outside the specialist files it lists.
- `scripts/check-i18n.mts` keeps the English and Dutch ARB files in step (keys, placeholders, Dutch product terms, `@key` descriptions).
- `crates/orbit-app/src/e2e/` exercises complete operator workflows across the supported appearance settings.
- `scripts/check-clippy.mts` (in `npm run check`) keeps both crates free of clippy findings.

## Showcase

`cargo run -p orbit-app -- --showcase` opens every primitive without a workspace. Check each theme, density, text scale, keyboard path, and long translated label there before changing a shared primitive.

## Change checklist

1. Reuse or extend a primitive without breaking its existing builders and callers.
2. Add or update its showcase entry and this document when introducing a reusable pattern.
3. Add English and Dutch ARB entries, with the English `@key` description, for new UI text.
4. Run `npm run check` and `npm run test:e2e`.
5. Run the app and verify the affected workflow manually when it depends on layout, media, output, audio, or hardware.

Use `TabList::equal_width()` when tabs divide the full panel width into equal sections, such as the three slide-document inspector tabs.
Use `Field::label_width()` with a metrics value to align a group of inline inspector labels.
Selected canvas objects share three equal-width inspector tabs: Shape, Text, and Build. Position and size use paired compact fields; effect checkboxes reveal their settings. Keep source-specific media and linked-text controls in the owning fill or text section.
Pair the `AngleDial` with a degree input for pointer and keyboard rotation.
Use `SelectOption::preview_icon` for graphical choices such as line patterns; retain descriptive accessible labels. `NumberInput::suffix` displays units without putting them into the editable numeric value. Feathering uses one amount slider and retains the imported style in the document.
