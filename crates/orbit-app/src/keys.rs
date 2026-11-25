//! Keyboard shortcuts, ported from `features/settings/key-mapping-utils.ts` and the keyboard dispatch of
//! `features/shell/app-interaction-controller.ts`. A shortcut resolves to a command id through the key
//! mappings, and runs only if a feature has registered that command.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gpui::{App, KeyDownEvent, Keystroke, Window};
use orbit_ui::controls::text_entry_focused;
use serde_json::Value;

/// `THEME_TOOL_SHAPES`: the shape ids with their default keys.
pub(crate) const THEME_SHAPES: [(&str, &str); 10] = [
    ("rounded-rect", "O"),
    ("rectangle", "R"),
    ("ellipse", "E"),
    ("triangle", "G"),
    ("right-triangle", "L"),
    ("rhombus", "H"),
    ("polygon", "P"),
    ("star", "S"),
    ("double-arrow", "D"),
    ("arrow", "A"),
];

const PRESENTATION_DEFAULTS: [(&str, &str); 14] = [
    ("newSlide", "Ctrl+Shift+N"),
    ("nextSlide", "ArrowRight"),
    ("previousSlide", "ArrowLeft"),
    ("nextBackground", "Alt+ArrowRight"),
    ("previousBackground", "Alt+ArrowLeft"),
    ("clearAll", "F1"),
    ("clearSlide", "F2"),
    ("clearMedia", "F3"),
    ("clearProps", "F4"),
    ("clearAudio", "F5"),
    ("clearMessages", "F6"),
    ("clearAnnouncements", "F7"),
    ("clearToLogo", "F12"),
    ("openLooks", "Ctrl+Shift+1"),
];
const SHELL_DEFAULTS: &[(&str, &str)] = &[
    ("undo", "Ctrl+Z"),
    ("redo", "Ctrl+Y"),
    ("cut", "Ctrl+X"),
    ("copy", "Ctrl+C"),
    ("paste", "Ctrl+V"),
    ("duplicate", "Ctrl+D"),
    ("delete", "Delete"),
    ("selectAll", "Ctrl+A"),
    ("deselectAll", "Ctrl+Shift+D"),
    ("open-handouts", "Ctrl+P"),
    ("open-search", "Ctrl+F"),
    ("open-key-settings", "Alt+K"),
    ("show-workspace", "Ctrl+S"),
    ("open-slide-canvas", "Ctrl+E"),
    ("open-reflow", "Ctrl+R"),
    ("open-bibles", "Ctrl+Alt+Shift+B"),
    ("view-audio", "Ctrl+Shift+A"),
    ("view-stage", "Alt+S"),
    ("view-timers", "Ctrl+Shift+C"),
    ("view-messages", "Ctrl+M"),
    ("view-props", "Ctrl+Shift+P"),
    ("view-macros", "Ctrl+Shift+M"),
    ("view-media", "Ctrl+Shift+V"),
    ("open-timecode", "Ctrl+Shift+T"),
    ("showTriggeredPresentation", "Ctrl+L"),
    ("showTriggeredAnnouncement", "Ctrl+Shift+L"),
    ("revealInLibrary", "Ctrl+Alt+L"),
    ("stageOnly", "Ctrl+0"),
];

/// The items of the Presentation menu in order, `None` for a separator.
pub const PRESENTATION_MENU_ITEMS: [Option<&str>; 15] = [
    Some("newSlide"),
    Some("nextSlide"),
    Some("previousSlide"),
    Some("nextBackground"),
    Some("previousBackground"),
    None,
    Some("clearAll"),
    Some("clearSlide"),
    Some("clearMedia"),
    Some("clearProps"),
    Some("clearAudio"),
    Some("clearMessages"),
    Some("clearAnnouncements"),
    None,
    Some("clearToLogo"),
];

/// `Ctrl+N` is fixed and does not appear in the key mappings.
pub const NEW_PRESENTATION: &str = "newPresentation";

/// `getThemeShapeCommandId`: `rounded-rect` becomes `themeShapeRoundedRect`.
pub fn theme_shape_command_id(shape: &str) -> String {
    let mut id = String::from("themeShape");
    for part in shape.split('-').filter(|part| !part.is_empty()) {
        let mut chars = part.chars();
        if let Some(first) = chars.next() {
            id.extend(first.to_uppercase());
            id.push_str(chars.as_str());
        }
    }
    id
}

/// A shortcut as the key mappings store it, such as `Ctrl+Shift+N` or `ArrowRight`.
pub fn shortcut_from_keystroke(keystroke: &Keystroke) -> Option<String> {
    let key = normalize_command_key(&keystroke.key)?;
    let modifiers = &keystroke.modifiers;
    let mut parts = Vec::with_capacity(5);
    if modifiers.control {
        parts.push("Ctrl".to_string());
    }
    if modifiers.platform {
        parts.push("Meta".to_string());
    }
    if modifiers.alt {
        parts.push("Alt".to_string());
    }
    if modifiers.shift {
        parts.push("Shift".to_string());
    }
    parts.push(key);
    Some(parts.join("+"))
}

fn normalize_command_key(key: &str) -> Option<String> {
    let name = match key {
        "right" => "ArrowRight",
        "left" => "ArrowLeft",
        "up" => "ArrowUp",
        "down" => "ArrowDown",
        "enter" => "Enter",
        "escape" => "Escape",
        "space" | " " => "Space",
        "tab" => "Tab",
        "backspace" => "Backspace",
        "delete" => "Delete",
        _ => "",
    };
    if !name.is_empty() {
        return Some(name.to_string());
    }
    let mut characters = key.chars();
    if let (Some(only), None) = (characters.next(), characters.next()) {
        return Some(only.to_uppercase().collect());
    }
    let digits = key.strip_prefix(['f', 'F'])?;
    matches!(digits.parse::<u8>(), Ok(1..=12)).then(|| key.to_uppercase())
}

/// `normalizeShortcutKey`: the first character, upper case, for single-key group shortcuts.
pub fn normalize_shortcut_key(value: &str) -> String {
    value
        .trim()
        .chars()
        .next()
        .map(|first| first.to_uppercase().collect())
        .unwrap_or_default()
}

/// `formatShortcutLabel`: the text shown next to a menu item.
pub fn format_shortcut_label(shortcut: &str) -> String {
    shortcut.replace("Arrow", "").replace("Control", "Ctrl")
}

/// Command id to shortcut. The defaults come first and settings override them in place, as in
/// `getKeyMappings`, so the first command with a shortcut wins when two share one.
#[derive(Clone, Debug, PartialEq)]
pub struct KeyMappings(Vec<(String, String)>);

impl Default for KeyMappings {
    fn default() -> Self {
        Self::from_settings(&Value::Null)
    }
}

impl KeyMappings {
    pub fn from_settings(settings: &Value) -> Self {
        let mut mappings: Vec<(String, String)> = PRESENTATION_DEFAULTS
            .iter()
            .chain(SHELL_DEFAULTS.iter())
            .map(|(command, key)| (command.to_string(), key.to_string()))
            .chain(
                THEME_SHAPES
                    .iter()
                    .map(|(shape, key)| (theme_shape_command_id(shape), key.to_string())),
            )
            .collect();
        if let Some(stored) = settings["keyMappings"].as_object() {
            for (command, shortcut) in stored {
                let Some(shortcut) = shortcut.as_str() else {
                    continue;
                };
                match mappings.iter_mut().find(|(known, _)| known == command) {
                    Some(entry) => entry.1 = shortcut.to_string(),
                    None => mappings.push((command.clone(), shortcut.to_string())),
                }
            }
        }
        Self(mappings)
    }

    pub fn shortcut(&self, command: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(known, _)| known == command)
            .map(|(_, shortcut)| shortcut.as_str())
    }

    /// Every assigned shortcut.
    pub fn shortcuts(&self) -> impl Iterator<Item = String> + '_ {
        self.0.iter().map(|(_, shortcut)| shortcut.clone())
    }

    pub fn command_for(&self, shortcut: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(_, mapped)| mapped == shortcut)
            .map(|(command, _)| command.as_str())
    }
}

type Handler = Rc<dyn Fn(&mut Window, &mut App)>;

struct Registered {
    repeats: bool,
    handler: Handler,
}

/// The commands the loaded features can run. A feature registers its command ids when it arrives.
#[derive(Clone, Default)]
pub struct Commands(Rc<RefCell<HashMap<String, Registered>>>);

impl Commands {
    pub fn register(
        &self,
        id: &str,
        repeats: bool,
        handler: impl Fn(&mut Window, &mut App) + 'static,
    ) {
        self.0.borrow_mut().insert(
            id.to_string(),
            Registered {
                repeats,
                handler: Rc::new(handler),
            },
        );
    }

    pub fn is_registered(&self, id: &str) -> bool {
        self.0.borrow().contains_key(id)
    }

    /// Runs a registered command. `held` is a key repeat, which only repeating commands accept.
    pub fn run(&self, id: &str, held: bool, window: &mut Window, cx: &mut App) -> bool {
        let handler = match self.0.borrow().get(id) {
            Some(entry) if !held || entry.repeats => entry.handler.clone(),
            _ => return false,
        };
        handler(window, cx);
        true
    }
}

/// What a key press means: `Ctrl+N` is a fixed command, everything else goes through the mappings.
fn command_for_keystroke(mappings: &KeyMappings, keystroke: &Keystroke) -> Option<String> {
    let shortcut = shortcut_from_keystroke(keystroke)?;
    if shortcut == "Ctrl+N" || shortcut == "Meta+N" {
        return Some(NEW_PRESENTATION.to_string());
    }
    let command = mappings
        .command_for(&shortcut)
        .or_else(|| match shortcut.as_str() {
            "Meta+Z" => mappings
                .command_for("Ctrl+Z")
                .filter(|action| *action == "undo"),
            "Meta+Shift+Z" => mappings
                .command_for("Ctrl+Y")
                .filter(|action| *action == "redo"),
            _ => None,
        });
    command.map(str::to_string)
}

/// The shell's key handler. It never runs while a text field has focus, so typing is never taken for a command.
/// Returns whether a command ran, in which case the caller stops the event.
pub fn dispatch_key_down(
    commands: &Commands,
    mappings: &KeyMappings,
    event: &KeyDownEvent,
    window: &mut Window,
    cx: &mut App,
) -> bool {
    if text_entry_focused(window) {
        return false;
    }
    match command_for_keystroke(mappings, &event.keystroke) {
        Some(command) => commands.run(&command, event.is_held, window, cx),
        None => false,
    }
}
