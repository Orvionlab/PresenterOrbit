---
name: ui-refactor
description: Migrate an existing Presenter Orbit screen to the design system without changing behavior. Use when asked to clean up, densify, or de-decorate a screen, or to migrate hand-built controls to shared orbit-ui primitives.
---

# UI refactor for Presenter Orbit

Goal: the same behavior with theme values, shared primitives, and the layout rules from `docs/design-system.md`. Visual changes are allowed only where they implement a documented rule.

## Workflow

1. **Inventory** the screen: its views, state types, and controller under `crates/orbit-app/src/<feature>/`. List every element id, action, and test (`rg -n '<name>' crates`) that the behavior depends on. Those contracts must survive.
2. **Baseline**: run `npm run check` and the feature's tests so you know what was green.
3. **Theme values first**: replace literal colors and sizes with `cx.theme().colors` and `cx.theme().metrics` fields. Map off-scale spacing to the nearest scale value (10 to 8 or 12, 14 to 12 or 16) and keep the result visually equivalent.
4. **Primitives**: swap hand-built tabs, segmented groups, collapsibles, headers, empty states, checkboxes, and dialogs to `orbit_ui::controls`. If a primitive lacks a needed option, extend the primitive in `orbit-ui` rather than restyling it in the feature.
5. **Decoration**: remove cards in panels, decorative shadows, gradients, glows, pills, uppercase labels, and subtitles. Unify header heights and selection/live colors.
6. **Tests**: update only tests that assert on changed structure, and keep their behavioral intent. Never weaken an assertion to pass.
7. **Verify**: run `npm run check` and `cargo test -p orbit-app -p orbit-ui -- --skip ten_thousand`. If the screen depends on output, media, or hardware, say that manual verification in the running app is still needed.

## Rules

- One screen or one concern per commit.
- Do not change behavior, persistence, ARB keys, or controller contracts.
- Do not add dependencies or a UI framework.
- Leave specialist surfaces (slide/output rendering, canvas, rulers, meters, color pickers, masks) on their fixed values.
