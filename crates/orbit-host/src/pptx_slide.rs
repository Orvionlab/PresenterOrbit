//! One `.pptx` slide to an Orbit slide: shapes in z-order with placeholder
//! geometry inherited from layout and master. Everything else is reported.

use super::pptx_package::{child, parse, related, Package, Relationship, Report, Theme, REL_NS};
use super::pptx_text::{self, Inherited};
use roxmltree::{Document, Node};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

pub const SLIDE_WIDTH: f64 = 1920.0;
pub const SLIDE_HEIGHT: f64 = 1080.0;
const EMU_PER_POINT: f64 = 12700.0;
const IMAGE_EXTENSIONS: [&str; 9] = [
    "png", "jpg", "jpeg", "gif", "bmp", "tif", "tiff", "webp", "svg",
];

/// Maps slide EMU to workspace pixels with one scale and centred letterboxing.
pub struct Deck {
    pub scale: f64,
    pub offset_x: f64,
    pub offset_y: f64,
}

impl Deck {
    pub fn new(width: f64, height: f64) -> Self {
        let scale = (SLIDE_WIDTH / width).min(SLIDE_HEIGHT / height);
        Self {
            scale,
            offset_x: (SLIDE_WIDTH - width * scale) / 2.0,
            offset_y: (SLIDE_HEIGHT - height * scale) / 2.0,
        }
    }
}

pub struct Media {
    pub root: PathBuf,
    extracted: HashMap<String, PathBuf>,
}

impl Media {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            extracted: HashMap::new(),
        }
    }

    fn extract(&mut self, package: &mut Package, part: &str) -> Result<PathBuf, String> {
        if let Some(path) = self.extracted.get(part) {
            return Ok(path.clone());
        }
        let extension = Path::new(part)
            .extension()
            .and_then(|value| value.to_str())
            .map(str::to_ascii_lowercase)
            .filter(|value| IMAGE_EXTENSIONS.contains(&value.as_str()))
            .ok_or_else(|| format!("unsupported image format {part}"))?;
        let target = self
            .root
            .join(format!("{}.{extension}", self.extracted.len()));
        fs::write(&target, package.bytes(part)?).map_err(|error| error.to_string())?;
        if let Err(error) = crate::portable_bundle::validate_asset(&target) {
            fs::remove_file(&target).map_err(|error| error.to_string())?;
            return Err(error);
        }
        self.extracted.insert(part.to_string(), target.clone());
        Ok(target)
    }
}

#[derive(Clone, Copy)]
struct Affine {
    sx: f64,
    sy: f64,
    tx: f64,
    ty: f64,
}

struct Rect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    rotation: f64,
    flip_h: bool,
    flip_v: bool,
}

fn number(node: Option<Node>, name: &str) -> Option<f64> {
    node?.attribute(name)?.parse().ok()
}

fn rect(xfrm: Node) -> Option<Rect> {
    let (offset, extent) = (child(xfrm, "off"), child(xfrm, "ext"));
    Some(Rect {
        x: number(offset, "x")?,
        y: number(offset, "y")?,
        w: number(extent, "cx")?,
        h: number(extent, "cy")?,
        rotation: number(Some(xfrm), "rot").unwrap_or(0.0) / 60000.0,
        flip_h: xfrm.attribute("flipH") == Some("1"),
        flip_v: xfrm.attribute("flipV") == Some("1"),
    })
}

#[derive(Clone)]
struct Placeholder {
    kind: String,
    index: Option<String>,
}

fn is(node: &Node, name: &str) -> bool {
    node.is_element() && node.tag_name().name() == name
}

fn placeholder(shape: Node) -> Option<Placeholder> {
    let properties = shape
        .children()
        .find(|node| node.is_element() && node.tag_name().name().starts_with("nv"))?;
    let ph = child(child(properties, "nvPr")?, "ph")?;
    let kind = match ph.attribute("type") {
        None | Some("obj" | "subTitle") => "body",
        Some("ctrTitle") => "title",
        Some(other) => other,
    };
    Some(Placeholder {
        kind: kind.to_string(),
        index: ph.attribute("idx").map(str::to_string),
    })
}

fn find_placeholder<'a>(document: &'a Document<'a>, wanted: &Placeholder) -> Option<Node<'a, 'a>> {
    let shapes = || {
        document
            .descendants()
            .filter(|node| is(node, "sp"))
            .filter_map(|node| Some((node, placeholder(node)?)))
    };
    shapes()
        .find(|(_, found)| wanted.index.is_some() && found.index == wanted.index)
        .or_else(|| shapes().find(|(_, found)| found.kind == wanted.kind))
        .map(|(node, _)| node)
}

fn styled(shape: Node) -> bool {
    child(shape, "spPr").is_some_and(|properties| {
        properties
            .children()
            .filter(Node::is_element)
            .any(|node| match node.tag_name().name() {
                "solidFill" | "gradFill" | "blipFill" | "pattFill" | "custGeom" => true,
                "prstGeom" => node.attribute("prst") != Some("rect"),
                "ln" => node.children().any(|fill| {
                    fill.is_element()
                        && matches!(
                            fill.tag_name().name(),
                            "solidFill" | "gradFill" | "pattFill"
                        )
                }),
                _ => false,
            })
    })
}

fn shape_name(shape: Node) -> String {
    shape
        .descendants()
        .find(|node| is(node, "cNvPr"))
        .and_then(|node| node.attribute("name"))
        .unwrap_or("unnamed")
        .to_string()
}

struct Walker<'p, 'a> {
    package: &'p mut Package,
    media: &'p mut Media,
    report: &'p mut Report,
    theme: &'p Theme,
    relationships: &'p HashMap<String, Relationship>,
    deck: &'p Deck,
    layout: &'a Document<'a>,
    master: &'a Document<'a>,
    number: usize,
    layers: Vec<Value>,
}

impl<'a> Walker<'_, 'a> {
    fn lose(&mut self, message: String) {
        let at = self.number;
        self.report.push(format!("slide {at}: {message}"));
    }

    fn walk(&mut self, parent: Node<'a, 'a>, transform: Affine) {
        for node in parent.children().filter(Node::is_element) {
            match node.tag_name().name() {
                "sp" => self.text_shape(node, transform),
                "pic" => self.picture(node, transform),
                "grpSp" => self.group(node, transform),
                "graphicFrame" => {
                    let uri = node
                        .descendants()
                        .find(|data| is(data, "graphicData"))
                        .and_then(|data| data.attribute("uri"))
                        .unwrap_or_default();
                    let kind = match uri.rsplit('/').next() {
                        Some("table") => "table",
                        Some("chart") => "chart",
                        Some("diagram") => "SmartArt diagram",
                        _ => "embedded object",
                    };
                    self.lose(format!("{kind} {} is not imported", shape_name(node)));
                }
                "cxnSp" => self.lose(format!("connector {} is not imported", shape_name(node))),
                "nvGrpSpPr" | "grpSpPr" | "extLst" => {}
                other => self.lose(format!("{other} content is not imported")),
            }
        }
    }

    fn inherit(&self, found: Option<&Placeholder>) -> (Option<Node<'a, 'a>>, Option<Node<'a, 'a>>) {
        let Some(found) = found else {
            return (None, None);
        };
        let layout = find_placeholder(self.layout, found);
        let wanted = layout
            .and_then(placeholder)
            .unwrap_or_else(|| found.clone());
        (layout, find_placeholder(self.master, &wanted))
    }

    fn geometry(
        &self,
        shape: Node<'a, 'a>,
        inherited: (Option<Node<'a, 'a>>, Option<Node<'a, 'a>>),
    ) -> Option<Rect> {
        [Some(shape), inherited.0, inherited.1]
            .into_iter()
            .flatten()
            .find_map(|node| rect(child(child(node, "spPr")?, "xfrm")?))
    }

    fn bounds(&self, rect: &Rect, t: Affine) -> Value {
        let deck = self.deck;
        json!({
            "x": (deck.offset_x + (t.tx + t.sx * rect.x) * deck.scale).round(),
            "y": (deck.offset_y + (t.ty + t.sy * rect.y) * deck.scale).round(),
            "width": (t.sx * rect.w * deck.scale).round().max(1.0),
            "height": (t.sy * rect.h * deck.scale).round().max(1.0),
        })
    }

    fn push(&mut self, mut layer: Value) {
        layer["id"] = json!(format!("layer-{}", self.layers.len() + 1));
        self.layers.push(layer);
    }

    fn text_shape(&mut self, shape: Node<'a, 'a>, t: Affine) {
        let name = shape_name(shape);
        let found = placeholder(shape);
        let (layout, master) = self.inherit(found.as_ref());
        let style_name = match found.as_ref().map(|found| found.kind.as_str()) {
            Some("title") => "titleStyle",
            Some("body") => "bodyStyle",
            _ => "otherStyle",
        };
        let styles = self
            .master
            .descendants()
            .find(|node| is(node, "txStyles"))
            .and_then(|node| child(node, style_name));
        let at = format!("slide {}", self.number);
        let content = pptx_text::convert(
            shape,
            &Inherited {
                layout,
                master,
                styles,
            },
            self.theme,
            EMU_PER_POINT * self.deck.scale,
            self.report,
            &at,
        );
        let Some(content) = content else {
            if found.is_none() {
                self.lose(format!(
                    "shape {name} without text (geometry and fill) is not imported"
                ));
            }
            return;
        };
        if styled(shape) {
            self.lose(format!(
                "fill, outline or geometry of text box {name} is not imported"
            ));
        }
        let Some(rect) = self.geometry(shape, (layout, master)) else {
            self.lose(format!(
                "text box {name} has no position and is not imported"
            ));
            return;
        };
        let mut layer = json!({
            "name": name, "type": "Text", "role": "custom",
            "bounds": self.bounds(&rect, t), "text": content.text,
            "textStyle": content.style, "fillEnabled": false, "fillType": "none",
        });
        if let Some(runs) = content.runs {
            layer["textRuns"] = json!(runs);
        }
        if rect.rotation != 0.0 {
            layer["rotation"] = json!(rect.rotation);
        }
        self.push(layer);
    }

    fn picture(&mut self, picture: Node<'a, 'a>, t: Affine) {
        let name = shape_name(picture);
        if picture.descendants().any(|node| {
            matches!(
                node.tag_name().name(),
                "videoFile" | "audioFile" | "wavAudioFile" | "quickTimeFile"
            )
        }) {
            self.lose(format!("video or audio {name} is not imported"));
            return;
        }
        let fill = child(picture, "blipFill");
        let part = fill
            .and_then(|fill| child(fill, "blip"))
            .and_then(|blip| blip.attribute((REL_NS, "embed")))
            .and_then(|id| self.relationships.get(id))
            .map(|relationship| relationship.target.clone());
        let Some(part) = part else {
            self.lose(format!("picture {name} has no embedded image"));
            return;
        };
        if fill
            .and_then(|fill| child(fill, "srcRect"))
            .is_some_and(|crop| crop.attributes().any(|value| value.value() != "0"))
        {
            self.lose(format!("crop of picture {name} is not applied"));
        }
        let found = placeholder(picture);
        let inherited = self.inherit(found.as_ref());
        let Some(rect) = self.geometry(picture, inherited) else {
            self.lose(format!(
                "picture {name} has no position and is not imported"
            ));
            return;
        };
        let path = match self.media.extract(self.package, &part) {
            Ok(path) => path,
            Err(error) => {
                self.lose(format!("picture {name} is not imported: {error}"));
                return;
            }
        };
        let url = url::Url::from_file_path(&path)
            .map(String::from)
            .unwrap_or_default();
        self.push(json!({
            "name": name, "type": "Media", "role": "custom",
            "bounds": self.bounds(&rect, t),
            "fill": "Media", "fillType": "media", "fillEnabled": true,
            "mediaType": "image", "mediaTitle": name, "mediaScale": "Stretch to Fill",
            "mediaUrl": url, "sourceUrl": url, "mediaSourcePath": path, "sourcePath": path,
            "mediaFlipX": rect.flip_h, "mediaFlipY": rect.flip_v, "rotation": rect.rotation,
        }));
    }

    fn group(&mut self, group: Node<'a, 'a>, t: Affine) {
        let name = shape_name(group);
        let xfrm = child(group, "grpSpPr").and_then(|properties| child(properties, "xfrm"));
        let frame = xfrm.and_then(rect);
        let child_extent = xfrm.and_then(|xfrm| child(xfrm, "chExt"));
        let child_offset = xfrm.and_then(|xfrm| child(xfrm, "chOff"));
        let (Some(frame), Some(cw), Some(ch)) = (
            frame,
            number(child_extent, "cx").filter(|value| *value > 0.0),
            number(child_extent, "cy").filter(|value| *value > 0.0),
        ) else {
            self.lose(format!(
                "group {name} has no usable transform and is not imported"
            ));
            return;
        };
        if frame.rotation != 0.0 || frame.flip_h || frame.flip_v {
            self.lose(format!("rotated or flipped group {name} is not imported"));
            return;
        }
        let (sx, sy) = (frame.w / cw, frame.h / ch);
        let inner = Affine {
            sx: t.sx * sx,
            sy: t.sy * sy,
            tx: t.tx + t.sx * (frame.x - number(child_offset, "x").unwrap_or(0.0) * sx),
            ty: t.ty + t.sy * (frame.y - number(child_offset, "y").unwrap_or(0.0) * sy),
        };
        self.walk(group, inner);
    }
}

fn background(
    documents: [&Document; 3],
    theme: &Theme,
    report: &mut Report,
    number: usize,
) -> Option<String> {
    let background = documents
        .iter()
        .find_map(|document| document.descendants().find(|node| is(node, "bg")))?;
    if let Some(fill) = child(background, "bgPr").and_then(|node| child(node, "solidFill")) {
        return theme.color(fill);
    }
    if let Some(reference) =
        child(background, "bgRef").filter(|node| node.attribute("idx") == Some("1001"))
    {
        return theme.color(reference);
    }
    report.push(format!(
        "slide {number}: gradient, picture or pattern background is not imported"
    ));
    None
}

fn has_decoration(document: &Document) -> bool {
    document
        .descendants()
        .find(|node| is(node, "spTree"))
        .is_some_and(|tree| {
            tree.children().filter(Node::is_element).any(|node| {
                matches!(
                    node.tag_name().name(),
                    "sp" | "pic" | "grpSp" | "cxnSp" | "graphicFrame"
                ) && placeholder(node).is_none()
            })
        })
}

fn notes(text: &str) -> Result<String, String> {
    let document = parse(text)?;
    Ok(document
        .descendants()
        .filter(|node| is(node, "sp"))
        .filter(|node| placeholder(*node).is_some_and(|found| found.kind == "body"))
        .filter_map(|node| child(node, "txBody"))
        .map(pptx_text::plain_text)
        .collect::<Vec<_>>()
        .join("\n"))
}

pub fn convert_slide(
    package: &mut Package,
    part: &str,
    number: usize,
    deck: &Deck,
    media: &mut Media,
    report: &mut Report,
) -> Result<Value, String> {
    let relationships = package.relationships(part)?;
    let missing = |what: &str| format!("PowerPoint slide {part} has no {what}.");
    let layout_part = related(&relationships, "/slideLayout")
        .ok_or_else(|| missing("layout"))?
        .to_string();
    let layout_relationships = package.relationships(&layout_part)?;
    let master_part = related(&layout_relationships, "/slideMaster")
        .ok_or_else(|| missing("master"))?
        .to_string();
    let master_relationships = package.relationships(&master_part)?;
    let theme_part = related(&master_relationships, "/theme")
        .ok_or_else(|| missing("theme"))?
        .to_string();
    let notes_text = match related(&relationships, "/notesSlide") {
        Some(notes_part) => Some(package.text(notes_part)?),
        None => None,
    };
    let slide_text = package.text(part)?;
    let layout_text = package.text(&layout_part)?;
    let master_text = package.text(&master_part)?;
    let theme_text = package.text(&theme_part)?;
    let (slide, layout, master, theme_document) = (
        parse(&slide_text)?,
        parse(&layout_text)?,
        parse(&master_text)?,
        parse(&theme_text)?,
    );
    let theme = Theme::new(&theme_document, &master);
    let tree = slide
        .descendants()
        .find(|node| is(node, "spTree"))
        .ok_or_else(|| missing("shape tree"))?;
    let mut walker = Walker {
        package,
        media,
        report,
        theme: &theme,
        relationships: &relationships,
        deck,
        layout: &layout,
        master: &master,
        number,
        layers: Vec::new(),
    };
    walker.walk(
        tree,
        Affine {
            sx: 1.0,
            sy: 1.0,
            tx: 0.0,
            ty: 0.0,
        },
    );
    let mut layers = walker.layers;
    if slide.descendants().any(|node| is(&node, "transition")) {
        report.push(format!(
            "slide {number}: the slide transition is not imported"
        ));
    }
    if slide.descendants().any(|node| is(&node, "timing")) {
        report.push(format!("slide {number}: animations are not imported"));
    }
    let shows_master = [&slide, &layout]
        .iter()
        .all(|document| document.root_element().attribute("showMasterSp") != Some("0"));
    if has_decoration(&layout) || (shows_master && has_decoration(&master)) {
        report
            .push("layout and master decorative shapes and pictures are not imported".to_string());
    }
    let order = layers
        .iter()
        .map(|layer| layer["id"].clone())
        .collect::<Vec<_>>();
    let mut item = json!({
        "id": format!("slide-{number}"), "name": format!("Slide {number}"), "width": SLIDE_WIDTH, "height": SLIDE_HEIGHT,
        "content": "", "text": "",
    });
    if let Some(layer) = layers.iter_mut().find(|layer| layer["type"] == "Text") {
        layer["role"] = json!("slide-text");
        item["primaryTextLayerId"] = layer["id"].clone();
        item["content"] = layer["text"].clone();
        item["text"] = layer["text"].clone();
        if let Some(runs) = layer
            .as_object_mut()
            .and_then(|layer| layer.remove("textRuns"))
        {
            item["contentRuns"] = runs;
        }
    }
    if let Some(color) = background([&slide, &layout, &master], &theme, report, number) {
        item["background"] = json!(color);
        item["backgroundEnabled"] = json!(true);
    }
    if theme.transformed.get() {
        report.push(
            "colour transforms (tint, shade, lumMod) are not applied; base colours are used"
                .to_string(),
        );
    }
    if let Some(text) = notes_text {
        let text = notes(&text)?;
        if !text.trim().is_empty() {
            item["notes"] = json!(text);
        }
    }
    item["layers"] = json!(layers);
    item["themeLayerOrder"] = json!(order);
    Ok(item)
}
