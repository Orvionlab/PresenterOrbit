use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Horizontal {
    #[default]
    Left,
    Center,
    Right,
    Justify,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Vertical {
    #[default]
    Top,
    Middle,
    Bottom,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FontStyle {
    #[default]
    Regular,
    Bold,
    Italic,
    BoldItalic,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Truncate {
    Start,
    Middle,
    #[default]
    End,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CueViewOptions {
    pub group_slides: bool,
    pub show_media_and_colors: bool,
    pub use_thumbnail_color: bool,
    pub thumbnail_color: Option<String>,
    pub table_color: Option<String>,
    pub font_family: Option<String>,
    pub font_style: FontStyle,
    pub font_size: Option<u16>,
    pub text_color: Option<String>,
    pub horizontal: Horizontal,
    pub vertical: Vertical,
    pub truncate: Truncate,
}

impl CueViewOptions {
    pub fn read(settings: &Value, mode: &str) -> Self {
        let mut options: Self = serde_json::from_value(settings[mode].clone()).unwrap_or_default();
        for color in [
            &mut options.thumbnail_color,
            &mut options.table_color,
            &mut options.text_color,
        ] {
            *color = color
                .take()
                .map(|color| {
                    if color == "transparent" {
                        "#00000000".into()
                    } else {
                        color
                    }
                })
                .filter(|color| {
                    color.strip_prefix('#').is_some_and(|hex| {
                        matches!(hex.len(), 6 | 8)
                            && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
                    })
                });
        }
        options.font_family = options.font_family.filter(|font| !font.trim().is_empty());
        options.font_size = options.font_size.map(|size| size.clamp(8, 72));
        options
    }
}

pub(crate) fn normalize(value: &Value) -> Value {
    json!({
        "easy": CueViewOptions::read(value, "easy"),
        "grid": CueViewOptions::read(value, "grid"),
        "table": CueViewOptions::read(value, "table"),
    })
}
