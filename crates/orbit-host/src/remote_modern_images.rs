//! Source-backed PNG images for the current ProPresenter Remote API.

use std::{io::Cursor, path::Path, sync::OnceLock};

use image::{DynamicImage, ImageReader};
use serde_json::{json, Value};

const MAX_EDGE: usize = 960;
const ICON_SOURCE: &str = include_str!("../assets/macro-icons.json");

/// Renders an asset from the accepted workspace document as PNG bytes.
/// `themeSlide` source ids use the workspace's `theme-id::slide-id` assignment form.
pub(in super::super) fn render(
    document: &Value,
    kind: &str,
    source_id: &str,
) -> Result<Vec<u8>, String> {
    match kind {
        "prop" => render_item_slide(document, "props", source_id),
        "stageLayout" => render_item_slide(document, "stageLayouts", source_id),
        "mask" => render_item_slide(document, "masks", source_id),
        "themeSlide" => render_theme_slide(document, source_id),
        "media" => render_media(document, source_id),
        "macro" => render_macro(document, source_id),
        "clearGroup" => render_clear_group(document, source_id),
        _ => Err("The requested image source kind is unsupported.".into()),
    }
}

fn render_item_slide(
    document: &Value,
    collection: &str,
    source_id: &str,
) -> Result<Vec<u8>, String> {
    let item = unique_item(document, collection, source_id)?;
    let slide = item
        .get("slide")
        .filter(|slide| slide.is_object())
        .ok_or_else(|| format!("The {collection} slide is unavailable."))?;
    let document = document
        .as_object()
        .ok_or("Workspace document must be an object.")?;
    let theme = slide
        .get("presentationThemeId")
        .and_then(Value::as_str)
        .and_then(|assignment| crate::workspace::content_themes::layout(document, assignment));
    let composed = crate::slide_composition::compose_slide(slide, theme);
    render_slide(&composed)
}

fn render_theme_slide(document: &Value, selector: &str) -> Result<Vec<u8>, String> {
    let (theme_id, slide_id) = selector
        .split_once("::")
        .filter(|(theme, slide)| !theme.is_empty() && !slide.is_empty())
        .ok_or("Theme slide identity must contain a theme and slide id.")?;
    let theme = unique_item(document, "themes", theme_id)?;
    let slides = theme
        .get("slides")
        .and_then(Value::as_array)
        .ok_or("Theme slides are unavailable.")?;
    let slide = unique_match(slides, "id", slide_id, "theme slide")?;
    render_slide(slide)
}

fn unique_item<'a>(
    document: &'a Value,
    collection: &str,
    source_id: &str,
) -> Result<&'a Value, String> {
    let entries = document
        .get(collection)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("Workspace {collection} are unavailable."))?;
    unique_match(entries, "id", source_id, collection)
}

fn unique_match<'a>(
    entries: &'a [Value],
    field: &str,
    source_id: &str,
    description: &str,
) -> Result<&'a Value, String> {
    let mut matches = entries
        .iter()
        .filter(|entry| entry.get(field).and_then(Value::as_str) == Some(source_id));
    let item = matches
        .next()
        .ok_or_else(|| format!("The {description} is unavailable."))?;
    if matches.next().is_some() {
        return Err(format!("The {description} identity is ambiguous."));
    }
    Ok(item)
}

fn render_slide(slide: &Value) -> Result<Vec<u8>, String> {
    let scene = crate::scene::SceneSlide::from_value(slide)?;
    let scale = MAX_EDGE as f64 / scene.width.max(scene.height);
    let width = (scene.width * scale).round().clamp(1.0, MAX_EDGE as f64) as usize;
    let height = (scene.height * scale).round().clamp(1.0, MAX_EDGE as f64) as usize;
    crate::native_compositor::render_slide_still(slide, width, height)?.encode_png()
}

fn render_media(document: &Value, source_id: &str) -> Result<Vec<u8>, String> {
    let media = unique_item(document, "media", source_id)?;
    let media_type = media
        .get("type")
        .and_then(Value::as_str)
        .ok_or("Media image type is unavailable.")?;
    if let Some(thumbnail_url) = media.get("thumbnailUrl") {
        let path = crate::native_media::resolve_media_path(thumbnail_url)
            .filter(|path| path.is_file())
            .ok_or_else(|| "Media thumbnail source is unavailable.".to_string())?;
        return encode_png(load_image(&path)?);
    }
    let source = media
        .get("sourcePath")
        .filter(|value| value.as_str().is_some_and(|path| !path.trim().is_empty()))
        .ok_or("Media image source path is unavailable.")?;
    let path = crate::native_media::resolve_media_path(source)
        .filter(|path| path.is_file())
        .ok_or_else(|| "Media image source is unavailable.".to_string())?;
    match media_type {
        "image" => encode_png(load_image(&path)?),
        "audio" => render_glyph(icon_symbol("ImageTypeAudio")?, "#ffffff"),
        "video" => {
            let seconds = match media.get("thumbnailTimeSeconds") {
                None | Some(Value::Null) => 0.0,
                Some(value) => value
                    .as_f64()
                    .filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
                    .ok_or("Media thumbnail time is invalid.")?,
            };
            let bytes = crate::media_thumbnail::extract(&path, seconds)?;
            encode_png(
                image::load_from_memory(&bytes)
                    .map_err(|error| format!("Media thumbnail image is invalid: {error}"))?,
            )
        }
        _ => Err("Media image type is unsupported.".into()),
    }
}

fn render_macro(document: &Value, source_id: &str) -> Result<Vec<u8>, String> {
    let item = unique_item(document, "macros", source_id)?;
    let image_type = match item.get("imageType") {
        None => "ImageTypeDefault",
        Some(value) => value.as_str().ok_or("Macro icon type is invalid.")?,
    };
    if image_type == "ImageTypeCustom" {
        let image = custom_image(item)?;
        return encode_png(image);
    }
    render_glyph(icon_symbol(image_type)?, "#ffffff")
}

fn render_clear_group(document: &Value, source_id: &str) -> Result<Vec<u8>, String> {
    let item = unique_item(document, "clearGroups", source_id)?;
    let image_type = required_string(item, "imageType", "Clear-group icon type is unavailable.")?;
    let tinted = match item.get("iconTinted") {
        None => false,
        Some(value) => value.as_bool().ok_or("Clear-group tint flag is invalid.")?,
    };
    let tint = if tinted {
        required_string(item, "iconTint", "Clear-group icon tint is unavailable.")?
    } else {
        "#ffffff"
    };
    if image_type == "ImageTypeCustom" {
        let mut image = custom_image(item)?;
        if tinted {
            apply_tint(&mut image, tint)?;
        }
        return encode_png(image);
    }
    let name = image_type
        .strip_prefix("ImageType")
        .ok_or("Clear-group icon type is invalid.")?;
    let mapped = icon_catalog()?["clearAliases"]
        .get(name)
        .and_then(Value::as_str);
    let mapped_type = mapped.map(|name| format!("ImageType{name}"));
    render_glyph(
        icon_symbol(mapped_type.as_deref().unwrap_or(image_type))?,
        tint,
    )
}

fn custom_image(item: &Value) -> Result<DynamicImage, String> {
    let encoded = required_string(item, "imageData", "Custom icon data is unavailable.")?;
    let bytes = crate::macro_image::decode_data(encoded)?;
    crate::macro_image::validate(&bytes)?;
    image::load_from_memory(&bytes)
        .map_err(|error| format!("Custom icon image is invalid: {error}"))
}

fn icon_symbol(image_type: &str) -> Result<&'static str, String> {
    let symbols = declaration("symbols")?;
    for entry in symbols.split('|') {
        if let Some((name, symbol)) = entry.split_once(':') {
            if format!("ImageType{name}") == image_type {
                return Ok(symbol);
            }
        }
    }

    let digits = declaration("digits")?;
    for (index, name) in digits.split('|').enumerate() {
        if format!("ImageType{name}") == image_type {
            const DIGITS: [&str; 10] = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"];
            return Ok(DIGITS[index]);
        }
    }

    if let Some(letter) = image_type.strip_prefix("ImageTypeLetter") {
        if letter.len() == 1 && letter.as_bytes()[0].is_ascii_uppercase() {
            if declaration("letters")? != "ABCDEFGHIJKLMNOPQRSTUVWXYZ" {
                return Err("Shared built-in icon source is unavailable.".into());
            }
            const LETTERS: [&str; 26] = [
                "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P",
                "Q", "R", "S", "T", "U", "V", "W", "X", "Y", "Z",
            ];
            return Ok(LETTERS[usize::from(letter.as_bytes()[0] - b'A')]);
        }
    }
    Err(format!(
        "Built-in icon {image_type} is not in the shared icon catalog."
    ))
}

fn declaration(name: &str) -> Result<&'static str, String> {
    icon_catalog()?
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| "Shared icon data is invalid.".into())
}

fn icon_catalog() -> Result<&'static Value, String> {
    static CATALOG: OnceLock<Result<Value, String>> = OnceLock::new();
    CATALOG
        .get_or_init(|| {
            serde_json::from_str(ICON_SOURCE).map_err(|_| "Shared icon data is invalid.".into())
        })
        .as_ref()
        .map_err(Clone::clone)
}

fn render_glyph(glyph: &str, color: &str) -> Result<Vec<u8>, String> {
    validate_color(color)?;
    let width = MAX_EDGE;
    let height = MAX_EDGE;
    let slide = json!({
        "width": width,
        "height": height,
        "layers": [{
            "id":"icon",
            "type":"Text",
            "text":glyph,
            "bounds":{"x":0,"y":0,"width":width,"height":height},
            "textStyle":{
                "fontFamily":"Arial",
                "fontSize":720,
                "textColor":color,
                "textHorizontalAlign":"center",
                "textVerticalAlign":"middle"
            }
        }]
    });
    let content = json!({
        "backgroundCss":"#00000000",
        "text":glyph,
        "presentationThemeSlide":slide
    });
    let mut pixels = vec![0; width * height * 4];
    let mut compositor = crate::native_compositor::NativeCompositor::new_preview();
    compositor.render_into(&mut pixels, width, height, &content)?;
    if !pixels.chunks_exact(4).any(|pixel| pixel[3] > 0) {
        return Err("The built-in icon glyph could not be rendered.".into());
    }
    crate::native_compositor::NativeFrame {
        width,
        height,
        pixels,
        dynamic_media: false,
    }
    .encode_png()
}

fn load_image(path: &Path) -> Result<DynamicImage, String> {
    let mut reader = ImageReader::open(path)
        .map_err(|error| format!("Media image could not be opened: {error}"))?
        .with_guessed_format()
        .map_err(|error| format!("Media image format could not be read: {error}"))?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    reader
        .decode()
        .map_err(|error| format!("Media image could not be decoded: {error}"))
}

fn encode_png(image: DynamicImage) -> Result<Vec<u8>, String> {
    let thumbnail = if image.width() > MAX_EDGE as u32 || image.height() > MAX_EDGE as u32 {
        image.thumbnail(MAX_EDGE as u32, MAX_EDGE as u32)
    } else {
        image
    };
    let mut bytes = Cursor::new(Vec::new());
    thumbnail
        .write_to(&mut bytes, image::ImageFormat::Png)
        .map_err(|error| format!("PNG image encoding failed: {error}"))?;
    Ok(bytes.into_inner())
}

fn apply_tint(image: &mut DynamicImage, color: &str) -> Result<(), String> {
    let [red, green, blue, alpha] = color_components(color)?;
    let mut rgba = image.to_rgba8();
    for pixel in rgba.pixels_mut() {
        pixel[0] = red;
        pixel[1] = green;
        pixel[2] = blue;
        pixel[3] = ((u16::from(pixel[3]) * u16::from(alpha)) / 255) as u8;
    }
    *image = DynamicImage::ImageRgba8(rgba);
    Ok(())
}

fn validate_color(color: &str) -> Result<(), String> {
    color_components(color).map(|_| ())
}

fn color_components(color: &str) -> Result<[u8; 4], String> {
    let hex = color
        .strip_prefix('#')
        .ok_or("Icon color must be a hexadecimal color.")?;
    if !matches!(hex.len(), 6 | 8) || !hex.chars().all(|value| value.is_ascii_hexdigit()) {
        return Err("Icon color must be a hexadecimal color.".into());
    }
    let component = |index| {
        u8::from_str_radix(&hex[index..index + 2], 16)
            .map_err(|_| "Icon color is invalid.".to_string())
    };
    Ok([
        component(0)?,
        component(2)?,
        component(4)?,
        if hex.len() == 8 { component(6)? } else { 255 },
    ])
}

fn required_string<'a>(item: &'a Value, field: &str, message: &str) -> Result<&'a str, String> {
    item.get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| message.to_string())
}
