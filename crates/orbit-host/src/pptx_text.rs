//! Text shapes of a slide: resolves paragraph/run formatting through the
//! shape, layout placeholder, master placeholder and master text styles.

use super::pptx_package::{child, children, Report, Theme};
use roxmltree::Node;
use serde_json::{json, Map, Value};

/// Nodes a placeholder inherits text properties from, nearest first.
#[derive(Default)]
pub struct Inherited<'a> {
    pub layout: Option<Node<'a, 'a>>,
    pub master: Option<Node<'a, 'a>>,
    pub styles: Option<Node<'a, 'a>>,
}

pub struct TextContent {
    pub text: String,
    pub runs: Option<Vec<Value>>,
    pub style: Value,
}

#[derive(Clone, PartialEq)]
struct Style {
    size: f64,
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
    color: Option<String>,
    family: Option<String>,
}

const BULLET_SETTINGS: [&str; 7] = [
    "buFont", "buClr", "buSzPct", "buSzPts", "buSzTx", "buClrTx", "buFontTx",
];

fn flag(value: Option<&str>) -> Option<bool> {
    value.map(|value| matches!(value, "1" | "true"))
}

fn attribute<'a>(nodes: &[Node<'a, 'a>], name: &str) -> Option<&'a str> {
    nodes.iter().find_map(|node| node.attribute(name))
}

fn run_style(
    properties: Option<Node<'_, '_>>,
    levels: &[Node<'_, '_>],
    theme: &Theme,
    scale: f64,
) -> Style {
    let sources: Vec<Node> = properties
        .into_iter()
        .chain(levels.iter().filter_map(|level| child(*level, "defRPr")))
        .collect();
    let points = attribute(&sources, "sz")
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(1800.0)
        / 100.0;
    Style {
        size: (points * scale * 10.0).round() / 10.0,
        bold: flag(attribute(&sources, "b")).unwrap_or(false),
        italic: flag(attribute(&sources, "i")).unwrap_or(false),
        underline: attribute(&sources, "u").is_some_and(|value| value != "none"),
        strike: attribute(&sources, "strike").is_some_and(|value| value != "noStrike"),
        color: sources
            .iter()
            .find_map(|node| child(*node, "solidFill"))
            .and_then(|fill| theme.color(fill)),
        family: sources
            .iter()
            .find_map(|node| child(*node, "latin"))
            .and_then(|latin| latin.attribute("typeface"))
            .map(|typeface| theme.font(typeface)),
    }
}

fn alignment(value: &str) -> &'static str {
    match value {
        "ctr" => "center",
        "r" => "right",
        "just" | "justLow" | "dist" | "thaiDist" => "justify",
        _ => "left",
    }
}

fn anchor(shape: Option<Node<'_, '_>>) -> Option<&'static str> {
    let body = child(child(shape?, "txBody")?, "bodyPr")?;
    body.attribute("anchor").map(|value| match value {
        "ctr" => "middle",
        "b" => "bottom",
        _ => "top",
    })
}

/// Plain text of a text body; paragraphs and line breaks both become `\n`.
pub fn plain_text(body: Node<'_, '_>) -> String {
    children(body, "p")
        .map(|paragraph| {
            paragraph
                .children()
                .filter(Node::is_element)
                .filter_map(|run| match run.tag_name().name() {
                    "r" | "fld" => child(run, "t")
                        .and_then(|text| text.text())
                        .map(str::to_string),
                    "br" => Some("\n".to_string()),
                    _ => None,
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn has_bullet(levels: &[Node<'_, '_>]) -> bool {
    levels
        .iter()
        .find_map(|level| {
            level
                .children()
                .filter(Node::is_element)
                .map(|node| node.tag_name().name())
                .find(|name| name.starts_with("bu") && !BULLET_SETTINGS.contains(name))
        })
        .is_some_and(|name| name != "buNone")
}

pub fn convert<'a>(
    shape: Node<'a, 'a>,
    inherited: &Inherited<'a>,
    theme: &Theme,
    px_per_pt: f64,
    report: &mut Report,
    at: &str,
) -> Option<TextContent> {
    let body = child(shape, "txBody")?;
    let list_styles = [
        Some(body),
        inherited.layout.and_then(|sp| child(sp, "txBody")),
        inherited.master.and_then(|sp| child(sp, "txBody")),
    ]
    .into_iter()
    .flatten()
    .filter_map(|body| child(body, "lstStyle"))
    .chain(inherited.styles)
    .collect::<Vec<_>>();
    let font_scale = child(body, "bodyPr")
        .and_then(|properties| child(properties, "normAutofit"))
        .and_then(|fit| fit.attribute("fontScale"))
        .and_then(|value| value.parse::<f64>().ok())
        .map_or(1.0, |value| value / 100_000.0);
    let scale = px_per_pt * font_scale;
    let mut runs: Vec<(String, Style)> = Vec::new();
    let mut alignments = Vec::new();
    for (index, paragraph) in children(body, "p").enumerate() {
        let properties = child(paragraph, "pPr");
        let level = properties
            .and_then(|node| node.attribute("lvl"))
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0)
            .min(8);
        let levels = list_styles
            .iter()
            .filter_map(|styles| child(*styles, &format!("lvl{}pPr", level + 1)))
            .collect::<Vec<_>>();
        let own = properties
            .into_iter()
            .chain(levels.iter().copied())
            .collect::<Vec<_>>();
        alignments.push(alignment(attribute(&own, "algn").unwrap_or("l")));
        if has_bullet(&levels) {
            report.push(format!("{at}: bullets and numbering are not imported"));
        }
        if index > 0 {
            let style = runs.last().map_or_else(
                || run_style(None, &levels, theme, scale),
                |(_, style)| style.clone(),
            );
            runs.push(("\n".to_string(), style));
        }
        for run in paragraph.children().filter(Node::is_element) {
            let text = match run.tag_name().name() {
                "r" | "fld" => child(run, "t")
                    .and_then(|text| text.text())
                    .unwrap_or_default(),
                "br" => "\n",
                _ => continue,
            };
            runs.push((
                text.to_string(),
                run_style(child(run, "rPr"), &levels, theme, scale),
            ));
        }
    }
    let mut merged: Vec<(String, Style)> = Vec::new();
    for (text, style) in runs {
        match merged.last_mut() {
            Some((previous, last)) if *last == style => previous.push_str(&text),
            _ => merged.push((text, style)),
        }
    }
    let text = merged
        .iter()
        .map(|(text, _)| text.as_str())
        .collect::<String>();
    if text.trim().is_empty() {
        return None;
    }
    let first = merged[0].1.clone();
    if merged
        .iter()
        .any(|(_, style)| style.size != first.size || style.family != first.family)
    {
        report.push(format!(
            "{at}: font size or family varies inside one text box; the first run is used"
        ));
    }
    if alignments.iter().any(|value| *value != alignments[0]) {
        report.push(format!(
            "{at}: paragraph alignment varies inside one text box; the first paragraph is used"
        ));
    }
    let vertical = [Some(shape), inherited.layout, inherited.master]
        .into_iter()
        .find_map(anchor)
        .unwrap_or("top");
    let mut style = Map::new();
    style.insert("fontSize".into(), json!(first.size));
    style.insert("bold".into(), json!(first.bold));
    style.insert("italic".into(), json!(first.italic));
    style.insert("underline".into(), json!(first.underline));
    style.insert("strikethrough".into(), json!(first.strike));
    style.insert("textHorizontalAlign".into(), json!(alignments[0]));
    style.insert("textVerticalAlign".into(), json!(vertical));
    if let Some(color) = &first.color {
        style.insert("textColor".into(), json!(color));
    }
    if let Some(family) = first.family.as_ref().filter(|family| !family.is_empty()) {
        style.insert("fontFamily".into(), json!(family));
    }
    let runs = (merged.len() > 1).then(|| {
        merged
            .iter()
            .map(|(text, style)| {
                let mut value = json!({
                    "bold": style.bold, "italic": style.italic,
                    "underline": style.underline, "strikethrough": style.strike
                });
                if let Some(color) = &style.color {
                    value["textColor"] = json!(color);
                }
                json!({ "text": text, "style": value })
            })
            .collect()
    });
    Some(TextContent {
        text,
        runs,
        style: Value::Object(style),
    })
}
