//! Panel sizes of the main window, ported from `features/layout/resizable-layout.ts`. The React app keeps them in
//! the WebView's `localStorage` under `presenter-orbit-layout`, which Rust cannot read, so the GPUI app keeps the
//! same JSON object in `layout.json` in its own data directory. The workspace format is not involved.

use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

/// Height of the menu bar and toolbar together, which every height limit subtracts.
pub const SHELL_TOP_HEIGHT: f32 = 92.;
pub const MENU_BAR_HEIGHT: f32 = 28.;
pub const TOOL_BAR_HEIGHT: f32 = 64.;
const FALLBACK_VIEWPORT: Viewport = Viewport {
    width: 1280.,
    height: 760.,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LayoutKey {
    LeftWidth,
    RightWidth,
    MediaSidebarWidth,
    MediaHeight,
    LeftBrowserTopHeight,
    ThemeSlideListWidth,
    ThemeInspectorWidth,
    ThemeSlidesHeight,
    RightOutputHeight,
    RightTransportHeight,
    RightAudioHeight,
    AudioPlaylistsHeight,
}

impl LayoutKey {
    pub const ALL: [Self; 12] = [
        Self::LeftWidth,
        Self::RightWidth,
        Self::MediaSidebarWidth,
        Self::MediaHeight,
        Self::LeftBrowserTopHeight,
        Self::ThemeSlideListWidth,
        Self::ThemeInspectorWidth,
        Self::ThemeSlidesHeight,
        Self::RightOutputHeight,
        Self::RightTransportHeight,
        Self::RightAudioHeight,
        Self::AudioPlaylistsHeight,
    ];

    /// The property name in the stored JSON object.
    pub fn name(self) -> &'static str {
        match self {
            Self::LeftWidth => "leftWidth",
            Self::RightWidth => "rightWidth",
            Self::MediaSidebarWidth => "mediaSidebarWidth",
            Self::MediaHeight => "mediaHeight",
            Self::LeftBrowserTopHeight => "leftBrowserTopHeight",
            Self::ThemeSlideListWidth => "themeSlideListWidth",
            Self::ThemeInspectorWidth => "themeInspectorWidth",
            Self::ThemeSlidesHeight => "themeSlidesHeight",
            Self::RightOutputHeight => "rightOutputHeight",
            Self::RightTransportHeight => "rightTransportHeight",
            Self::RightAudioHeight => "rightAudioHeight",
            Self::AudioPlaylistsHeight => "audioPlaylistsHeight",
        }
    }

    pub fn default_value(self) -> i32 {
        match self {
            Self::LeftWidth => 320,
            Self::RightWidth => 510,
            Self::MediaSidebarWidth => 176,
            Self::MediaHeight => 194,
            Self::LeftBrowserTopHeight => 260,
            Self::ThemeSlideListWidth => 164,
            Self::ThemeInspectorWidth => 360,
            Self::ThemeSlidesHeight => 320,
            Self::RightOutputHeight => 326,
            Self::RightTransportHeight => 166,
            Self::RightAudioHeight => 112,
            Self::AudioPlaylistsHeight => 120,
        }
    }

    fn limits(self) -> (i32, i32) {
        match self {
            Self::LeftWidth => (240, 560),
            Self::RightWidth => (340, 720),
            Self::MediaSidebarWidth => (120, 360),
            Self::MediaHeight => (120, 340),
            Self::LeftBrowserTopHeight => (120, 520),
            Self::ThemeSlideListWidth => (132, 340),
            Self::ThemeInspectorWidth => (300, 560),
            Self::ThemeSlidesHeight => (104, 620),
            Self::RightOutputHeight => (180, 520),
            Self::RightTransportHeight => (128, 300),
            Self::RightAudioHeight => (78, 260),
            Self::AudioPlaylistsHeight => (48, 400),
        }
    }

    fn index(self) -> usize {
        Self::ALL
            .iter()
            .position(|key| *key == self)
            .expect("every key is listed")
    }
}

/// What a resize handle changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resize {
    LeftWidth,
    RightWidth,
    MediaSidebarWidth,
    MediaHeight,
    LeftBrowserTop,
    ThemeSlideListWidth,
    ThemeInspectorWidth,
    RightOutput,
    RightTransport,
    AudioPlaylists,
}

impl Resize {
    pub fn key(self) -> LayoutKey {
        match self {
            Self::LeftWidth => LayoutKey::LeftWidth,
            Self::RightWidth => LayoutKey::RightWidth,
            Self::MediaSidebarWidth => LayoutKey::MediaSidebarWidth,
            Self::MediaHeight => LayoutKey::MediaHeight,
            Self::LeftBrowserTop => LayoutKey::LeftBrowserTopHeight,
            Self::ThemeSlideListWidth => LayoutKey::ThemeSlideListWidth,
            Self::ThemeInspectorWidth => LayoutKey::ThemeInspectorWidth,
            Self::RightOutput => LayoutKey::RightOutputHeight,
            Self::RightTransport => LayoutKey::RightTransportHeight,
            Self::AudioPlaylists => LayoutKey::AudioPlaylistsHeight,
        }
    }

    /// Handles that move along the vertical axis, which show a row-resize cursor.
    pub fn is_horizontal_divider(self) -> bool {
        matches!(
            self,
            Self::MediaHeight
                | Self::LeftBrowserTop
                | Self::RightOutput
                | Self::RightTransport
                | Self::AudioPlaylists
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    values: [i32; 12],
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            values: LayoutKey::ALL.map(LayoutKey::default_value),
        }
    }
}

impl Layout {
    pub fn get(&self, key: LayoutKey) -> f32 {
        self.values[key.index()] as f32
    }

    fn value(&self, key: LayoutKey) -> i32 {
        self.values[key.index()]
    }

    /// A stored object, with every missing or unusable property replaced by its default, then clamped.
    pub fn from_stored(stored: &Map<String, Value>, viewport: Viewport) -> Self {
        let mut layout = Self::default();
        for key in LayoutKey::ALL {
            let wanted = number(stored.get(key.name())).unwrap_or(key.default_value() as f64);
            // Each value is clamped against the defaults of the others, as the React app does while loading.
            layout.values[key.index()] = clamp(key, wanted, None, viewport);
        }
        layout
    }

    pub fn to_json(self) -> Value {
        Value::Object(
            LayoutKey::ALL
                .iter()
                .map(|key| (key.name().to_string(), json!(self.value(*key))))
                .collect(),
        )
    }

    /// Applies one drag step and clamps against the other current sizes.
    pub fn resized(self, resize: Resize, next: f32, viewport: Viewport) -> Self {
        let key = resize.key();
        let mut layout = self;
        layout.values[key.index()] = clamp(key, next as f64, Some(&self), viewport);
        layout
    }
}

/// `Number(value) || fallback`: numbers and numeric strings count, zero and NaN do not.
fn number(value: Option<&Value>) -> Option<f64> {
    let parsed = match value? {
        Value::Number(number) => number.as_f64()?,
        Value::String(text) => text.trim().parse::<f64>().ok()?,
        _ => return None,
    };
    (parsed != 0. && parsed.is_finite()).then_some(parsed)
}

/// JavaScript's `Math.round`, which rounds halves up.
fn js_round(value: f64) -> i32 {
    (value + 0.5).floor() as i32
}

fn clamp(key: LayoutKey, value: f64, layout: Option<&Layout>, viewport: Viewport) -> i32 {
    let (min, mut max) = key.limits();
    if let Some(dynamic) = dynamic_max(key, layout, viewport) {
        max = max.min(dynamic);
    }
    js_round(value.max(min as f64).min(max.max(min) as f64))
}

fn dynamic_max(key: LayoutKey, layout: Option<&Layout>, viewport: Viewport) -> Option<i32> {
    let other = |key: LayoutKey| layout.map_or(key.default_value(), |layout| layout.value(key));
    let Viewport { width, height } = viewport;
    let (width, height) = (width as i32, height as i32);
    let top = SHELL_TOP_HEIGHT as i32;
    match key {
        LayoutKey::LeftWidth => Some(width - other(LayoutKey::RightWidth) - 480),
        LayoutKey::RightWidth => Some(width - other(LayoutKey::LeftWidth) - 480),
        LayoutKey::MediaSidebarWidth => Some(width - other(LayoutKey::RightWidth) - 420),
        LayoutKey::ThemeSlideListWidth => Some(width - other(LayoutKey::ThemeInspectorWidth) - 480),
        LayoutKey::ThemeInspectorWidth => Some(width - other(LayoutKey::ThemeSlideListWidth) - 480),
        LayoutKey::MediaHeight => Some(height - top - 260),
        LayoutKey::LeftBrowserTopHeight => Some(height - top - other(LayoutKey::MediaHeight) - 90),
        LayoutKey::ThemeSlidesHeight => Some(height - top - 66 - 160),
        LayoutKey::AudioPlaylistsHeight => None,
        LayoutKey::RightOutputHeight
        | LayoutKey::RightTransportHeight
        | LayoutKey::RightAudioHeight => {
            let used = |own: LayoutKey| if key == own { 0 } else { other(own) };
            Some(
                height
                    - top
                    - used(LayoutKey::RightOutputHeight)
                    - used(LayoutKey::RightTransportHeight)
                    - used(LayoutKey::RightAudioHeight)
                    - 101,
            )
        }
    }
}

/// The viewport the React app sees as `window.innerWidth` and `innerHeight`, with its fallback for zero.
pub fn viewport_or_fallback(width: f32, height: f32) -> Viewport {
    Viewport {
        width: if width > 0. {
            width
        } else {
            FALLBACK_VIEWPORT.width
        },
        height: if height > 0. {
            height
        } else {
            FALLBACK_VIEWPORT.height
        },
    }
}

/// Where a drag puts the value, from the pointer at the start and now, in logical window pixels.
#[derive(Clone, Copy, Debug)]
pub struct Drag {
    pub resize: Resize,
    pub start_value: f32,
    pub start: (f32, f32),
    pub current: (f32, f32),
    /// Top edge of the left browser.
    pub left_top: f32,
}

pub fn next_value(drag: Drag) -> f32 {
    let dx = drag.current.0 - drag.start.0;
    let dy = drag.current.1 - drag.start.1;
    match drag.resize {
        Resize::RightWidth | Resize::ThemeInspectorWidth => drag.start_value - dx,
        Resize::MediaHeight => drag.start_value - dy,
        Resize::LeftBrowserTop => drag.current.1 - drag.left_top,
        Resize::AudioPlaylists => drag.start_value + dy,
        Resize::RightOutput | Resize::RightTransport => drag.start_value + dy,
        Resize::LeftWidth | Resize::MediaSidebarWidth | Resize::ThemeSlideListWidth => {
            drag.start_value + dx
        }
    }
}

pub fn layout_path(data_directory: &Path) -> PathBuf {
    data_directory.join("layout.json")
}

/// A missing or malformed file gives the defaults, because layout preferences are optional in the React app too.
pub fn load(path: &Path, viewport: Viewport) -> Layout {
    let stored = std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok());
    match stored {
        Some(Value::Object(stored)) => Layout::from_stored(&stored, viewport),
        _ => Layout::from_stored(&Map::new(), viewport),
    }
}

pub fn save(path: &Path, layout: Layout) -> Result<(), String> {
    std::fs::write(path, layout.to_json().to_string())
        .map_err(|error| format!("Could not save the panel sizes: {error}"))
}
