use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChordChart {
    pub source_path: String,
    pub source_fingerprint: String,
    pub page_paths: Vec<String>,
    pub page: usize,
}

fn fingerprint(bytes: &[u8]) -> String {
    let mut value = 0xcbf29ce484222325u64;
    for byte in bytes {
        value = (value ^ u64::from(*byte)).wrapping_mul(0x100000001b3);
    }
    format!("{value:016x}")
}

pub fn validate_source_fingerprint(bytes: &[u8], expected: &str) -> Result<(), String> {
    if bytes.len() > 32 * 1024 * 1024 || fingerprint(bytes) != expected {
        return Err("Chord chart source changed or is corrupt. Import it again.".into());
    }
    Ok(())
}

fn read_bounded(path: &Path) -> Result<Vec<u8>, String> {
    let file =
        std::fs::File::open(path).map_err(|e| format!("Chord chart asset is unavailable: {e}"))?;
    let mut bytes = Vec::new();
    file.take(32 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 32 * 1024 * 1024 {
        return Err("Chord chart asset exceeds 32 MiB.".into());
    }
    Ok(bytes)
}

fn decode(bytes: &[u8]) -> Result<image::DynamicImage, String> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    reader
        .decode()
        .map_err(|e| format!("Chord chart image is corrupt or exceeds bounds: {e}"))
}

pub fn validate_page(path: &Path) -> Result<(), String> {
    let metadata =
        std::fs::metadata(path).map_err(|e| format!("Chord chart page is missing: {e}"))?;
    if !metadata.is_file() || metadata.len() > 32 * 1024 * 1024 {
        return Err("Chord chart page exceeds bounds or is not a file.".into());
    }
    decode(&read_bounded(path)?).map(|_| ())
}

fn managed_path(workspace: &Path, value: &str) -> Result<PathBuf, String> {
    let root = workspace
        .join("Media")
        .join("Charts")
        .canonicalize()
        .map_err(|e| format!("Chord chart folder is unavailable: {e}"))?;
    let workspace_root = workspace.canonicalize().map_err(|e| e.to_string())?;
    if !root.starts_with(workspace_root) {
        return Err("Chord chart folder escapes the workspace.".into());
    }
    let path = Path::new(value)
        .canonicalize()
        .map_err(|e| format!("Chord chart asset is missing: {e}"))?;
    if !path.starts_with(root) || !path.is_file() {
        return Err("Chord chart asset must be a managed workspace file.".into());
    }
    Ok(path)
}

pub fn validate(workspace: &Path, chart: &ChordChart) -> Result<(), String> {
    if chart.page_paths.is_empty()
        || chart.page_paths.len() > 64
        || chart.page >= chart.page_paths.len()
    {
        return Err("Chord chart page is outside the document.".into());
    }
    let source = managed_path(workspace, &chart.source_path)?;
    if std::fs::metadata(&source).map_err(|e| e.to_string())?.len() > 32 * 1024 * 1024 {
        return Err("Chord chart source exceeds 32 MiB.".into());
    }
    let original = read_bounded(&source)?;
    if fingerprint(&original) != chart.source_fingerprint {
        return Err("Chord chart source changed or is corrupt. Import it again.".into());
    }
    for page in &chart.page_paths {
        let bytes = read_bounded(&managed_path(workspace, page)?)?;
        if bytes.len() > 32 * 1024 * 1024 {
            return Err("Chord chart page exceeds 32 MiB.".into());
        }
        decode(&bytes)?;
    }
    Ok(())
}

pub fn import(workspace: &Path, source: &Path) -> Result<ChordChart, String> {
    let metadata =
        std::fs::metadata(source).map_err(|e| format!("Chord chart is unavailable: {e}"))?;
    if !metadata.is_file() || metadata.len() > 32 * 1024 * 1024 {
        return Err("Chord charts must be files no larger than 32 MiB.".into());
    }
    let extension = source
        .extension()
        .and_then(|v| v.to_str())
        .ok_or("Chord chart file type is required.")?
        .to_ascii_lowercase();
    let source_bytes = read_bounded(source)?;
    let pages = match extension.as_str() {
        "pdf" => crate::chord_chart_pdf::pdf_pages(&source_bytes)?,
        "png" | "jpg" | "jpeg" => vec![decode(&source_bytes)?],
        _ => return Err("Chord charts must be PDF, PNG or JPEG.".into()),
    };
    if read_bounded(source)? != source_bytes {
        return Err("Chord chart source changed during import. Select it again.".into());
    }
    let workspace_root = workspace.canonicalize().map_err(|e| e.to_string())?;
    let media_root = workspace.join("Media");
    std::fs::create_dir_all(&media_root).map_err(|e| e.to_string())?;
    if !media_root
        .canonicalize()
        .map_err(|e| e.to_string())?
        .starts_with(&workspace_root)
    {
        return Err("Chord chart media folder escapes the workspace.".into());
    }
    let charts_root = media_root.join("Charts");
    std::fs::create_dir_all(&charts_root).map_err(|e| e.to_string())?;
    if !charts_root
        .canonicalize()
        .map_err(|e| e.to_string())?
        .starts_with(&workspace_root)
    {
        return Err("Chord chart folder escapes the workspace.".into());
    }
    let directory = charts_root.join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let result = (|| {
        let copied = directory.join(format!("source.{extension}"));
        std::fs::write(&copied, &source_bytes).map_err(|e| e.to_string())?;
        let mut page_paths = Vec::new();
        for (index, page) in pages.iter().enumerate() {
            let path = directory.join(format!("page-{index}.png"));
            page.save_with_format(&path, image::ImageFormat::Png)
                .map_err(|e| e.to_string())?;
            page_paths.push(path.to_string_lossy().into_owned());
        }
        let chart = ChordChart {
            source_path: copied.to_string_lossy().into_owned(),
            source_fingerprint: fingerprint(&source_bytes),
            page_paths,
            page: 0,
        };
        validate(workspace, &chart)?;
        Ok(chart)
    })();
    if result.is_err() {
        std::fs::remove_dir_all(directory).map_err(|e| e.to_string())?;
    }
    result
}
