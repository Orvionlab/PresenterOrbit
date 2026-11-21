//! Import of an editable `.pptx` deck as one new library presentation. Images
//! are copied into the workspace `Media` folder; everything that cannot be
//! represented is returned as an import report and stored in `importWarnings`.

use super::pptx_package::{parse, Package, Report, REL_NS};
use super::pptx_slide::{convert_slide, Deck, Media};
use super::{content_themes, read_workspace, safe_name, save_workspace, unique_id};
use crate::workspace_import::{place, same_title, Duplicate};
use serde_json::{json, Value};
use std::{fs, path::Path};

const MAX_IMPORT_WARNINGS: usize = 64;
const PRESENTATION_PART: &str = "ppt/presentation.xml";

/// Slide parts in presentation order plus the slide size in EMU.
fn presentation(package: &mut Package) -> Result<(Vec<String>, f64, f64), String> {
    let text = package.text(PRESENTATION_PART)?;
    let relationships = package.relationships(PRESENTATION_PART)?;
    let document = parse(&text)?;
    let size = document
        .descendants()
        .find(|node| node.tag_name().name() == "sldSz");
    let dimension = |name: &str| {
        size.and_then(|node| node.attribute(name))
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|value| *value > 0.0)
            .ok_or("PowerPoint presentation has no valid slide size.".to_string())
    };
    let (width, height) = (dimension("cx")?, dimension("cy")?);
    let slides = document
        .descendants()
        .find(|node| node.tag_name().name() == "sldIdLst")
        .map(|list| {
            list.children()
                .filter(|node| node.is_element())
                .map(|node| {
                    node.attribute((REL_NS, "id"))
                        .and_then(|id| relationships.get(id))
                        .map(|relationship| relationship.target.clone())
                        .ok_or("PowerPoint slide list references a missing slide.".to_string())
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?
        .unwrap_or_default();
    if slides.is_empty() {
        return Err("PowerPoint presentation has no slides.".into());
    }
    Ok((slides, width, height))
}

/// Returns the saved workspace and the import report (also stored in the workspace).
/// Title the deck is imported under.
pub fn title(source: &Path) -> &str {
    source
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .unwrap_or("Presentation")
}

pub fn import(
    workspace_path: &Path,
    source: &Path,
    library_id: &str,
    duplicate: Duplicate,
) -> Result<(Value, Vec<String>), String> {
    let workspace = read_workspace(workspace_path)?;
    let library_index = workspace["libraries"]
        .as_array()
        .and_then(|libraries| {
            libraries
                .iter()
                .position(|library| library["id"].as_str() == Some(library_id))
        })
        .ok_or("Selected destination library is missing.")?;
    let taken = workspace["library"]
        .as_array()
        .ok_or("Library is invalid.")?
        .iter()
        .filter_map(|item| item["id"].as_str())
        .collect::<Vec<_>>();
    let title = title(source);
    let id = match (same_title(&workspace, library_id, title), duplicate) {
        (Some(_), Duplicate::UseExisting) => return Ok((workspace, Vec::new())),
        (Some(existing), Duplicate::Replace) => existing,
        _ => unique_id(&safe_name(title, "presentation"), taken.into_iter()),
    };

    let mut package = Package::open(source)?;
    let (parts, width, height) = presentation(&mut package)?;
    let deck = Deck::new(width, height);
    let container = workspace_path
        .parent()
        .ok_or("Workspace folder is required.")?
        .join("Media")
        .join("PowerPoint Imports");
    fs::create_dir_all(&container).map_err(|error| error.to_string())?;
    let root = container.join(uuid::Uuid::new_v4().to_string());
    fs::create_dir(&root).map_err(|error| error.to_string())?;
    let mut media = Media::new(root.clone());
    let result = (|| {
        let mut report = Report::default();
        let slides = parts
            .iter()
            .enumerate()
            .map(|(index, part)| {
                convert_slide(
                    &mut package,
                    part,
                    index + 1,
                    &deck,
                    &mut media,
                    &mut report,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        // Slide graphics live in theme layouts, as for every other imported presentation.
        let mut items =
            vec![json!({ "id": id, "title": title, "libraryId": library_id, "slides": slides })];
        let mut themes = workspace["themes"]
            .as_array()
            .cloned()
            .ok_or("Themes are invalid.")?;
        content_themes::import_layouts(&mut items, &mut themes)?;
        let mut patch = workspace.clone();
        patch["themes"] = json!(themes);
        for item in items {
            place(&mut patch, library_index, item)?;
        }
        let warnings = patch["importWarnings"]
            .as_array_mut()
            .ok_or("Import warnings are required.")?;
        for message in &report.0 {
            if warnings.len() < MAX_IMPORT_WARNINGS {
                warnings.push(json!(format!("library/{id}: {message}")));
            }
        }
        Ok((save_workspace(workspace_path, &patch)?, report.0))
    })();
    let empty = fs::read_dir(&root)
        .map_err(|error| error.to_string())?
        .next()
        .is_none();
    if result.is_err() || empty {
        fs::remove_dir_all(&root).map_err(|error| {
            format!("PowerPoint import cleanup of its media folder failed: {error}")
        })?;
    }
    result
}
