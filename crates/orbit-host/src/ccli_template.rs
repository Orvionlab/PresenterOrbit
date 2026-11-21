use serde_json::{json, Value};

pub const ID: &str = "ccli-template";
pub const TEXT_ID: &str = "ccli-text";
const SAMPLE_TEXT: &str =
    "Song title\nArtist or author\nPublisher © Copyright year\nCCLI song number";

pub fn default_slide() -> Value {
    json!({
        "id": ID,
        "width": 1920,
        "height": 1080,
        "backgroundEnabled": false,
        "layers": [{
            "id": TEXT_ID,
            "name": "CCLI",
            "type": "Text",
            "shape": "rectangle",
            "bounds": { "x": 850, "y": 680, "width": 1030, "height": 350 },
            "fillEnabled": false,
            "strokeEnabled": false,
            "text": SAMPLE_TEXT,
            "textStyle": {
                "fontFamily": "Arial",
                "fontSize": 45,
                "textColor": "#ffffff",
                "textHorizontalAlign": "right",
                "textVerticalAlign": "middle",
                "linkedTextEnabled": true,
                "linkedTextSource": "CCLI"
            }
        }]
    })
}

pub(crate) fn validate(slide: &Value) -> Result<(), String> {
    if slide["id"] != ID
        || slide["backgroundEnabled"] != false
        || ["width", "height"].iter().any(|axis| {
            slide[*axis]
                .as_f64()
                .is_none_or(|size| !(1.0..=16384.0).contains(&size))
        })
    {
        return Err("Invalid CCLI template identity, size, or background.".into());
    }
    let layers = slide["layers"]
        .as_array()
        .ok_or("The CCLI template must contain an object list.")?;
    let mut linked = layers.iter().filter(|layer| layer["id"] == TEXT_ID);
    let layer = linked.next().ok_or("The CCLI text object is required.")?;
    if linked.next().is_some()
        || layer["type"] != "Text"
        || layer["text"] != SAMPLE_TEXT
        || layer["hidden"] == true
        || layer["textStyle"]["linkedTextEnabled"] != true
        || layer["textStyle"]["linkedTextSource"] != "CCLI"
    {
        return Err("The CCLI text object must remain linked to copyright information.".into());
    }
    crate::scene::parse_scene_slide(slide)?;
    crate::native_compositor::validate_object_links(slide)
}
