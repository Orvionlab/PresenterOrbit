use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

const MAX_IMPORT_WARNINGS: usize = 64;

fn local_source(reference: &str) -> Result<PathBuf, String> {
    let path = match url::Url::parse(reference) {
        Ok(url) if url.scheme() == "file" => url
            .to_file_path()
            .map_err(|_| "the file URL does not identify a local file on this system")?,
        Ok(url) if url.scheme().len() > 1 => {
            return Err(format!("{} chart URLs are not downloaded", url.scheme()))
        }
        _ => PathBuf::from(reference),
    };
    if !path.is_absolute() {
        return Err("the chart reference is not an absolute local path".into());
    }
    Ok(path)
}

/// Turns each imported presentation-level `chordChartUrl` into a managed chart on
/// every slide. Unresolvable references stay on the item and are reported.
/// Directories created for managed charts are appended to `created`.
pub fn attach(folder: &Path, patch: &mut Value, created: &mut Vec<PathBuf>) -> Result<(), String> {
    let mut warnings = Vec::new();
    let mut imported: HashMap<PathBuf, Result<Value, String>> = HashMap::new();
    let items = patch
        .get_mut("library")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten();
    for item in items {
        let Some(reference) = item
            .get("chordChartUrl")
            .and_then(Value::as_str)
            .filter(|reference| !reference.is_empty())
            .map(str::to_owned)
        else {
            continue;
        };
        let id = item["id"].as_str().unwrap_or("unnamed").to_owned();
        let outcome = local_source(&reference).and_then(|source| {
            let key = source.canonicalize().unwrap_or_else(|_| source.clone());
            imported
                .entry(key)
                .or_insert_with(|| {
                    let chart = crate::chord_chart::import(folder, &source)?;
                    if let Some(directory) = Path::new(&chart.source_path).parent() {
                        created.push(directory.to_path_buf());
                    }
                    serde_json::to_value(chart).map_err(|error| error.to_string())
                })
                .clone()
        });
        match outcome {
            Ok(chart) => {
                for slide in item["slides"].as_array_mut().into_iter().flatten() {
                    if let Some(slide) = slide.as_object_mut() {
                        slide.entry("chordChart").or_insert_with(|| chart.clone());
                    }
                }
                if let Some(object) = item.as_object_mut() {
                    object.remove("chordChartUrl");
                }
            }
            Err(reason) => warnings.push(format!(
                "library/{id}/chordChartUrl: chord chart {reference} was not imported ({reason}); the presentation has no stage chart"
            )),
        }
    }
    let target = patch["importWarnings"]
        .as_array_mut()
        .ok_or("Import warnings are required.")?;
    for warning in warnings {
        if target.len() < MAX_IMPORT_WARNINGS {
            target.push(json!(warning));
        }
    }
    Ok(())
}
