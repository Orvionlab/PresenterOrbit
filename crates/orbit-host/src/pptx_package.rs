//! Reading of `.pptx` packages: ZIP parts, relationships, theme colours and the
//! import report shared by the slide converter.

use roxmltree::{Document, Node};
use std::{cell::Cell, collections::HashMap, fs::File, io::Read, path::Path};

const MAX_PART_BYTES: u64 = 64 * 1024 * 1024;
pub const REL_NS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

pub struct Relationship {
    pub kind: String,
    pub target: String,
}

pub struct Package(zip::ZipArchive<File>);

impl Package {
    pub fn open(path: &Path) -> Result<Self, String> {
        let file = File::open(path)
            .map_err(|error| format!("PowerPoint file could not be opened: {error}"))?;
        zip::ZipArchive::new(file)
            .map(Self)
            .map_err(|error| format!("PowerPoint file is not a valid .pptx package: {error}"))
    }

    pub fn bytes(&mut self, part: &str) -> Result<Vec<u8>, String> {
        let entry = self
            .0
            .by_name(part)
            .map_err(|error| format!("PowerPoint package part {part} is unavailable: {error}"))?;
        let mut bytes = Vec::new();
        entry
            .take(MAX_PART_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| {
                format!("PowerPoint package part {part} could not be read: {error}")
            })?;
        if bytes.len() as u64 > MAX_PART_BYTES {
            return Err(format!("PowerPoint package part {part} is too large."));
        }
        Ok(bytes)
    }

    pub fn text(&mut self, part: &str) -> Result<String, String> {
        String::from_utf8(self.bytes(part)?)
            .map_err(|_| format!("PowerPoint package part {part} is not UTF-8 XML."))
    }

    /// A part without a relationships file has no relationships.
    pub fn relationships(&mut self, part: &str) -> Result<HashMap<String, Relationship>, String> {
        let (directory, name) = part.rsplit_once('/').unwrap_or(("", part));
        let rels = format!("{directory}/_rels/{name}.rels");
        if self.0.by_name(&rels).is_err() {
            return Ok(HashMap::new());
        }
        let text = self.text(&rels)?;
        let document = parse(&text)?;
        Ok(document
            .root_element()
            .children()
            .filter(|node| node.is_element() && node.attribute("TargetMode") != Some("External"))
            .filter_map(|node| {
                Some((
                    node.attribute("Id")?.to_string(),
                    Relationship {
                        kind: node.attribute("Type")?.to_string(),
                        target: resolve(part, node.attribute("Target")?),
                    },
                ))
            })
            .collect())
    }
}

fn resolve(base: &str, target: &str) -> String {
    let mut parts: Vec<&str> = if target.starts_with('/') {
        Vec::new()
    } else {
        let mut parts: Vec<&str> = base.split('/').collect();
        parts.pop();
        parts
    };
    for segment in target.split('/') {
        match segment {
            ".." => {
                parts.pop();
            }
            "." | "" => {}
            segment => parts.push(segment),
        }
    }
    parts.join("/")
}

pub fn parse(text: &str) -> Result<Document<'_>, String> {
    Document::parse(text).map_err(|error| format!("PowerPoint XML is invalid: {error}"))
}

pub fn related<'a>(
    relationships: &'a HashMap<String, Relationship>,
    suffix: &str,
) -> Option<&'a str> {
    relationships
        .values()
        .find(|relationship| relationship.kind.ends_with(suffix))
        .map(|relationship| relationship.target.as_str())
}

pub fn child<'a, 'i>(node: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    node.children()
        .find(|child| child.is_element() && child.tag_name().name() == name)
}

pub fn children<'a, 'i>(
    node: Node<'a, 'i>,
    name: &'static str,
) -> impl Iterator<Item = Node<'a, 'i>> {
    node.children()
        .filter(move |child| child.is_element() && child.tag_name().name() == name)
}

/// Import report: duplicates collapse so deck-wide losses are listed once.
#[derive(Default)]
pub struct Report(pub Vec<String>);

impl Report {
    pub fn push(&mut self, message: String) {
        if !self.0.contains(&message) {
            self.0.push(message);
        }
    }
}

pub struct Theme {
    colors: HashMap<String, String>,
    color_map: HashMap<String, String>,
    major: String,
    minor: String,
    pub transformed: Cell<bool>,
}

impl Theme {
    pub fn new(theme: &Document, master: &Document) -> Self {
        let elements = theme
            .descendants()
            .find(|node| node.tag_name().name() == "themeElements");
        let colors = elements
            .and_then(|node| child(node, "clrScheme"))
            .into_iter()
            .flat_map(|scheme| scheme.children().filter(Node::is_element))
            .filter_map(|node| {
                let color = node.children().find(Node::is_element)?;
                let value = color.attribute("val").or(color.attribute("lastClr"))?;
                Some((node.tag_name().name().to_string(), format!("#{value}")))
            })
            .collect();
        let font = |name: &str| {
            elements
                .and_then(|node| child(node, "fontScheme"))
                .and_then(|scheme| child(scheme, name))
                .and_then(|font| child(font, "latin"))
                .and_then(|latin| latin.attribute("typeface"))
                .unwrap_or_default()
                .to_string()
        };
        let color_map = master
            .descendants()
            .find(|node| node.tag_name().name() == "clrMap")
            .into_iter()
            .flat_map(|map| map.attributes())
            .map(|attribute| (attribute.name().to_string(), attribute.value().to_string()))
            .collect();
        Self {
            colors,
            color_map,
            major: font("majorFont"),
            minor: font("minorFont"),
            transformed: Cell::new(false),
        }
    }

    /// Resolves the colour child of a fill-like element (`a:solidFill`, `p:bgRef`).
    pub fn color(&self, fill: Node) -> Option<String> {
        let color = fill.children().find(|node| {
            node.is_element() && matches!(node.tag_name().name(), "srgbClr" | "schemeClr")
        })?;
        if color.children().any(|node| node.is_element()) {
            self.transformed.set(true);
        }
        let value = color.attribute("val")?;
        if color.tag_name().name() == "srgbClr" {
            return Some(format!("#{value}"));
        }
        let key = self.color_map.get(value).map_or(value, String::as_str);
        self.colors.get(key).cloned()
    }

    pub fn font(&self, typeface: &str) -> String {
        match typeface {
            "+mj-lt" => self.major.clone(),
            "+mn-lt" => self.minor.clone(),
            other => other.to_string(),
        }
    }
}
