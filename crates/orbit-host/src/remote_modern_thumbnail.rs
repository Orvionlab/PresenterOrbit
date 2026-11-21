use std::{
    collections::HashMap,
    io::Cursor,
    sync::{Mutex, OnceLock},
};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use image::{DynamicImage, ImageFormat};
use sha2::{Digest, Sha256};

use super::super::Backend;
use super::{catalog, data};

const DEFAULT_QUALITY: u32 = 400;
const MAX_QUALITY: u32 = 1920;

#[derive(Clone)]
pub(super) struct ThumbnailResponse {
    pub content_type: &'static str,
    pub body: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum ThumbnailType {
    Jpeg,
    Png,
}

type ThumbnailKey = ([u8; 32], u32, ThumbnailType);
#[derive(Default)]
struct ThumbnailCache {
    entries: HashMap<ThumbnailKey, ThumbnailResponse>,
    bytes: usize,
}
static THUMBNAILS: OnceLock<Mutex<ThumbnailCache>> = OnceLock::new();
const MAX_THUMBNAIL_CACHE_BYTES: usize = 16 * 1024 * 1024;
const MAX_THUMBNAIL_CACHE_ENTRIES: usize = 128;

fn cached_thumbnail(
    cache: &Mutex<ThumbnailCache>,
    key: ThumbnailKey,
    render: impl FnOnce() -> Result<ThumbnailResponse, String>,
) -> Result<ThumbnailResponse, String> {
    if let Some(response) = cache
        .lock()
        .map_err(|_| "Thumbnail cache is unavailable.")?
        .entries
        .get(&key)
        .cloned()
    {
        return Ok(response);
    }
    let response = render()?;
    if response.body.len() <= MAX_THUMBNAIL_CACHE_BYTES {
        let mut cache = cache
            .lock()
            .map_err(|_| "Thumbnail cache is unavailable.")?;
        if cache.entries.len() >= MAX_THUMBNAIL_CACHE_ENTRIES
            || cache.bytes + response.body.len() > MAX_THUMBNAIL_CACHE_BYTES
        {
            cache.entries.clear();
            cache.bytes = 0;
        }
        if !cache.entries.contains_key(&key) {
            cache.bytes += response.body.len();
            cache.entries.insert(key, response.clone());
        }
    }
    Ok(response)
}

struct ThumbnailRequest {
    target: ImageTarget,
    quality: u32,
    thumbnail_type: ThumbnailType,
}

enum ImageTarget {
    Presentation(String, usize),
    Playlist(String, usize, usize),
    ActivePlaylist(usize, usize),
    Asset(&'static str, String),
    ThemeSlide(String, String),
}

pub(super) fn handle(
    backend: &dyn Backend,
    target: &str,
    accept: Option<&str>,
) -> Result<ThumbnailResponse, String> {
    let request = parse_target(target, accept)?;
    let status = backend.full()?;
    let (presentation, index) = match &request.target {
        ImageTarget::Presentation(id, index) => {
            let resolved = match id.as_str() {
                "active" => super::live_resolved(&status)?,
                "focused" => super::focused_resolved(&status)?,
                _ => catalog::resolve_presentation(&status, id)?,
            };
            (resolved, *index)
        }
        ImageTarget::Playlist(id, entry, index) => (
            catalog::resolve_playlist_presentation(&status, id, *entry)?,
            *index,
        ),
        ImageTarget::ActivePlaylist(entry, index) => {
            let id = status["livePlaylistId"]
                .as_str()
                .ok_or("There is no active playlist.")?;
            let selector = catalog::identity(id, "", 0)["uuid"]
                .as_str()
                .ok_or("Active playlist UUID is unavailable.")?
                .to_string();
            (
                catalog::resolve_playlist_presentation(&status, &selector, *entry)?,
                *index,
            )
        }
        ImageTarget::Asset(kind, selector) => {
            let id = asset_source(&status, kind, selector)?;
            let bytes = backend.asset_image(kind, &id)?;
            let decoded = image::load_from_memory(&bytes)
                .map_err(|_| "The native asset preview is invalid.")?;
            return encode_thumbnail(decoded, request.quality, request.thumbnail_type);
        }
        ImageTarget::ThemeSlide(theme, slide) => {
            let theme_id = asset_source(&status, "themes", theme)?;
            let source = status["modernCatalog"]["themes"]
                .as_array()
                .and_then(|themes| themes.iter().find(|entry| entry["id"] == theme_id))
                .ok_or("Theme is unavailable.")?;
            let slide_id = resolve_source(
                source["slides"]
                    .as_array()
                    .ok_or("Theme slides are unavailable.")?,
                slide,
                "name",
            )?;
            let bytes = backend.asset_image("themeSlide", &format!("{theme_id}::{slide_id}"))?;
            let decoded = image::load_from_memory(&bytes)
                .map_err(|_| "The native theme preview is invalid.")?;
            return encode_thumbnail(decoded, request.quality, request.thumbnail_type);
        }
    };
    let slide = presentation
        .slides
        .get(index)
        .ok_or("The thumbnail cue index is unavailable.")?;
    let images = backend.slide_images(
        data::workspace_path(&status)?,
        &presentation.path,
        std::slice::from_ref(slide),
        ((request.quality.min(960) * 100 + 959) / 960).clamp(1, 100) as u8,
    )?;
    let [encoded] = images.as_slice() else {
        return Err("The slide renderer did not return exactly one thumbnail.".into());
    };
    let key = (
        Sha256::digest(encoded.as_bytes()).into(),
        request.quality,
        request.thumbnail_type,
    );
    cached_thumbnail(THUMBNAILS.get_or_init(Default::default), key, || {
        let jpeg = STANDARD
            .decode(encoded)
            .map_err(|_| "The slide renderer returned invalid image data.")?;
        let decoded = image::load_from_memory(&jpeg)
            .map_err(|_| "The slide renderer returned an unsupported image.")?;
        encode_thumbnail(decoded, request.quality, request.thumbnail_type)
    })
}

fn parse_target(target: &str, accept: Option<&str>) -> Result<ThumbnailRequest, String> {
    if target.is_empty() || target.len() > 4096 || target.contains('#') || target.contains('\\') {
        return Err("A bounded thumbnail request target is required.".into());
    }
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let path = path.strip_prefix('/').unwrap_or(path);
    let parts = path
        .split('/')
        .map(decode_path_segment)
        .collect::<Result<Vec<_>, _>>()?;
    if parts
        .iter()
        .any(|part| part.is_empty() || part == "." || part == ".." || part.contains('/'))
    {
        return Err("Invalid thumbnail request path.".into());
    }
    let path = parts.iter().map(String::as_str).collect::<Vec<_>>();
    let image_target = match path.as_slice() {
        ["v1", "presentation", id, "thumbnail", index] => {
            ImageTarget::Presentation((*id).into(), parse_index(index)?)
        }
        ["v1", "playlist", id, entry, "thumbnail", index] => {
            ImageTarget::Playlist((*id).into(), parse_index(entry)?, parse_index(index)?)
        }
        ["v1", "playlist", "active", "presentation", entry, "thumbnail", index] => {
            ImageTarget::ActivePlaylist(parse_index(entry)?, parse_index(index)?)
        }
        ["v1", "prop", id, "thumbnail"] => ImageTarget::Asset("props", (*id).into()),
        ["v1", "media", id, "thumbnail"] => ImageTarget::Asset("media", (*id).into()),
        ["v1", "mask", id, "thumbnail"] => ImageTarget::Asset("masks", (*id).into()),
        ["v1", "stage", "layout", id, "thumbnail"] => {
            ImageTarget::Asset("stageLayouts", (*id).into())
        }
        ["v1", "macro", id, "icon"] => ImageTarget::Asset("macros", (*id).into()),
        ["v1", "clear", "group", id, "icon"] => ImageTarget::Asset("clearGroups", (*id).into()),
        ["v1", "theme", id, "slides", slide, "thumbnail"] => {
            ImageTarget::ThemeSlide((*id).into(), (*slide).into())
        }
        _ => return Err("The thumbnail request path is invalid.".into()),
    };
    let mut quality = None;
    let mut thumbnail_type = None;
    for (name, value) in url::form_urlencoded::parse(query.as_bytes()) {
        match name.as_ref() {
            "quality" => {
                if quality.is_some() {
                    return Err("Duplicate quality parameter.".into());
                }
                let parsed = value
                    .parse::<u32>()
                    .map_err(|_| "The quality parameter must be an integer.")?;
                if !(1..=MAX_QUALITY).contains(&parsed) {
                    return Err(format!(
                        "The quality parameter must be between 1 and {MAX_QUALITY}."
                    ));
                }
                quality = Some(parsed);
            }
            "thumbnail_type" => {
                if thumbnail_type.is_some() {
                    return Err("Duplicate thumbnail_type parameter.".into());
                }
                thumbnail_type = Some(match value.as_ref() {
                    "jpeg" => ThumbnailType::Jpeg,
                    "png" => ThumbnailType::Png,
                    _ => return Err("thumbnail_type must be jpeg or png.".into()),
                });
            }
            _ => {}
        }
    }
    let icon = matches!(
        &image_target,
        ImageTarget::Asset("macros" | "clearGroups", _)
    );
    if icon && thumbnail_type == Some(ThumbnailType::Jpeg) {
        return Err("Icon images support PNG only.".into());
    }
    let thumbnail_type = match thumbnail_type {
        _ if icon => ThumbnailType::Png,
        Some(thumbnail_type) => thumbnail_type,
        None => negotiate_thumbnail_type(accept)?,
    };
    Ok(ThumbnailRequest {
        target: image_target,
        quality: quality.unwrap_or(DEFAULT_QUALITY),
        thumbnail_type,
    })
}

fn asset_source(status: &serde_json::Value, kind: &str, selector: &str) -> Result<String, String> {
    resolve_source(
        status["modernCatalog"][kind]
            .as_array()
            .ok_or("The preview catalog is unavailable.")?,
        selector,
        "title",
    )
}

fn resolve_source(
    entries: &[serde_json::Value],
    selector: &str,
    name: &str,
) -> Result<String, String> {
    let found = entries
        .iter()
        .enumerate()
        .filter(|(index, entry)| {
            entry["id"].as_str().is_some_and(|id| {
                catalog::matches_id(id, entry[name].as_str().unwrap_or(""), *index, selector)
            })
        })
        .collect::<Vec<_>>();
    let [(_, entry)] = found.as_slice() else {
        return Err("The preview identity must match exactly one asset.".into());
    };
    Ok(entry["id"]
        .as_str()
        .ok_or("The preview identity is invalid.")?
        .into())
}

fn negotiate_thumbnail_type(accept: Option<&str>) -> Result<ThumbnailType, String> {
    let Some(accept) = accept else {
        return Ok(ThumbnailType::Jpeg);
    };
    let mut jpeg = (0_u8, 0.0_f64);
    let mut png = (0_u8, 0.0_f64);
    let mut recognized = false;
    for range in accept.split(',') {
        let mut sections = range.trim().split(';');
        let media = sections
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        let (kind, subtype) = media
            .split_once('/')
            .ok_or("The Accept header contains an invalid media range.")?;
        if kind.is_empty() || subtype.is_empty() || subtype.contains('/') {
            return Err("The Accept header contains an invalid media range.".into());
        }
        let mut quality = 1.0_f64;
        let mut quality_seen = false;
        for parameter in sections {
            let Some((name, value)) = parameter.trim().split_once('=') else {
                continue;
            };
            if name.trim().eq_ignore_ascii_case("q") {
                if quality_seen {
                    return Err("The Accept header contains a duplicate quality value.".into());
                }
                quality_seen = true;
                quality = value
                    .trim()
                    .parse::<f64>()
                    .ok()
                    .filter(|quality| quality.is_finite() && (0.0..=1.0).contains(quality))
                    .ok_or("The Accept quality value must be between 0 and 1.")?;
            }
        }
        let (specificity, matches_jpeg, matches_png) = match (kind, subtype) {
            ("*", "*") => (0, true, true),
            ("image", "*") => (1, true, true),
            ("image", "jpeg") => (2, true, false),
            ("image", "png") => (2, false, true),
            _ => continue,
        };
        recognized = true;
        for (candidate, matches) in [(&mut jpeg, matches_jpeg), (&mut png, matches_png)] {
            if matches
                && (specificity > candidate.0
                    || (specificity == candidate.0 && quality > candidate.1))
            {
                *candidate = (specificity, quality);
            }
        }
    }
    if !recognized || (jpeg.1 == 0.0 && png.1 == 0.0) {
        return Err("The Accept header does not allow JPEG or PNG.".into());
    }
    Ok(if png.1 > jpeg.1 {
        ThumbnailType::Png
    } else {
        ThumbnailType::Jpeg
    })
}

fn parse_index(value: &str) -> Result<usize, String> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("A zero-based thumbnail cue index is required.".into());
    }
    value
        .parse()
        .map_err(|_| "The thumbnail cue index is outside the supported range.".into())
}

fn decode_path_segment(value: &str) -> Result<String, String> {
    let mut decoded = Vec::with_capacity(value.len());
    let mut bytes = value.bytes();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let high = bytes.next().and_then(|byte| (byte as char).to_digit(16));
            let low = bytes.next().and_then(|byte| (byte as char).to_digit(16));
            decoded.push(match (high, low) {
                (Some(high), Some(low)) => (high * 16 + low) as u8,
                _ => return Err("Invalid thumbnail path encoding.".into()),
            });
        } else {
            decoded.push(byte);
        }
    }
    String::from_utf8(decoded).map_err(|_| "The thumbnail path must be UTF-8.".into())
}

fn encode_thumbnail(
    image: DynamicImage,
    quality: u32,
    thumbnail_type: ThumbnailType,
) -> Result<ThumbnailResponse, String> {
    let (width, height) = (image.width(), image.height());
    if width == 0 || height == 0 {
        return Err("The slide renderer returned an empty image.".into());
    }
    let longest = width.max(height);
    let output_width = ((u64::from(width) * u64::from(quality) + u64::from(longest) / 2)
        / u64::from(longest))
    .max(1) as u32;
    let output_height = ((u64::from(height) * u64::from(quality) + u64::from(longest) / 2)
        / u64::from(longest))
    .max(1) as u32;
    let resized = image.resize_exact(
        output_width,
        output_height,
        image::imageops::FilterType::Lanczos3,
    );
    let mut body = Vec::new();
    let content_type = match thumbnail_type {
        ThumbnailType::Jpeg => {
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut body, 85)
                .encode_image(&resized.to_rgb8())
                .map_err(|_| "The thumbnail could not be encoded as JPEG.")?;
            "image/jpeg"
        }
        ThumbnailType::Png => {
            resized
                .write_to(&mut Cursor::new(&mut body), ImageFormat::Png)
                .map_err(|_| "The thumbnail could not be encoded as PNG.")?;
            "image/png"
        }
    };
    Ok(ThumbnailResponse { content_type, body })
}
