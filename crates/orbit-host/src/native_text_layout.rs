use crate::native_text_list::{invalid_list_fields, TextList};
use crate::native_text_tabs::{
    invalid_tab_stops, parse_bar_stops, TabStops, BAR_STOPS_FIELD, INTERVAL_FIELD,
};
use cosmic_text::{
    Align, Attrs, Buffer, Color, Family, FeatureTag, FontSystem, LayoutGlyph, LayoutRun,
    LineEnding, LineIter, Metrics, Shaping, Style, SwashCache, Weight, Wrap,
};
use serde_json::{Map, Value};
use std::borrow::Cow;
use std::ops::Range;

/// Line height of unstyled text, as a multiple of the font size. Matches the renderer CSS.
pub(crate) const BASE_LINE_HEIGHT: f32 = 1.06;

/// Authored paragraph fields with their inclusive bounds. Indents and spacing use the same
/// units as `fontSize` (slide pixels); the line height multiple scales `BASE_LINE_HEIGHT`.
const PARAGRAPH_FIELDS: [(&str, f64, f64); 10] = [
    ("textLineHeightMultiple", 0.5, 3.0),
    ("textParagraphSpacingBefore", 0.0, 1000.0),
    ("textParagraphSpacingAfter", 0.0, 1000.0),
    ("textFirstLineIndent", -1000.0, 1000.0),
    ("textLeftIndent", 0.0, 1000.0),
    ("textRightIndent", 0.0, 1000.0),
    (INTERVAL_FIELD, 1.0, 2000.0),
    ("textLineSpacing", 0.0, 1000.0),
    ("textMinimumLineHeight", 0.0, 1000.0),
    ("textMaximumLineHeight", 0.0, 1000.0),
];

pub(crate) fn resolve_paragraph_style(
    base: &Map<String, Value>,
    index: usize,
) -> Map<String, Value> {
    let mut style: Map<String, Value> = base
        .iter()
        .filter(|(key, _)| key.as_str() != "textParagraphStyles")
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    if let Some(patch) = base
        .get("textParagraphStyles")
        .and_then(Value::as_array)
        .and_then(|paragraphs| paragraphs.get(index))
        .and_then(Value::as_object)
    {
        if patch
            .get("textListStyle")
            .and_then(Value::as_str)
            .is_some_and(|name| {
                matches!(
                    name,
                    "none" | "box" | "check" | "diamond" | "disc" | "circle" | "square" | "hyphen"
                )
            })
        {
            for key in ["textListPrefix", "textListPostfix", "textListStart"] {
                style.remove(key);
            }
        }
        if patch.contains_key("textAbsoluteRightEdge") {
            style.remove("textRightIndent");
        } else if patch.contains_key("textRightIndent") {
            style.remove("textAbsoluteRightEdge");
        }
        style.extend(patch.clone());
    }
    style
}

/// The paragraph fields of `object` outside the supported range, with the reason for each.
pub(crate) fn invalid_paragraph_fields(object: &Map<String, Value>) -> Vec<(&'static str, String)> {
    let mut invalid = Vec::new();
    for (field, min, max) in PARAGRAPH_FIELDS {
        if let Some(value) = object.get(field) {
            let valid = value
                .as_f64()
                .is_some_and(|number| number.is_finite() && number >= min && number <= max);
            if !valid {
                invalid.push((
                    field,
                    format!("{field} must be a finite number between {min} and {max}."),
                ));
            }
        }
    }
    invalid.extend(invalid_tab_stops(object));
    for field in [
        "textMarginLeft",
        "textMarginRight",
        "textMarginTop",
        "textMarginBottom",
    ] {
        if object.get(field).is_some_and(|value| {
            !value
                .as_f64()
                .is_some_and(|value| value.is_finite() && value >= 0.0)
        }) {
            invalid.push((
                field,
                format!("{field} must be a finite nonnegative number."),
            ));
        }
    }
    if let Some(value) = object.get("textAbsoluteRightEdge") {
        if !value
            .as_f64()
            .is_some_and(|value| value.is_finite() && value > 0.0)
        {
            invalid.push((
                "textAbsoluteRightEdge",
                "textAbsoluteRightEdge must be a finite positive number.".into(),
            ));
        }
        if object.contains_key("textRightIndent") {
            invalid.push((
                "textAbsoluteRightEdge",
                "textAbsoluteRightEdge and textRightIndent cannot both be set.".into(),
            ));
        }
    }
    if let Some(value) = object.get(BAR_STOPS_FIELD) {
        if let Err(reason) = parse_bar_stops(value) {
            invalid.push((BAR_STOPS_FIELD, reason));
        }
    }
    invalid.extend(invalid_list_fields(object));
    if let Some(level) = object.get("textListLevel") {
        if !level.as_u64().is_some_and(|level| level <= 8) {
            invalid.push((
                "textListLevel",
                "textListLevel must be a whole number between 0 and 8.".into(),
            ));
        }
    }
    let minimum = object
        .get("textMinimumLineHeight")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let maximum = object
        .get("textMaximumLineHeight")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    if maximum > 0.0 && maximum < minimum {
        invalid.push((
            "textMaximumLineHeight",
            "textMaximumLineHeight must be zero or at least textMinimumLineHeight.".into(),
        ));
    }
    if let Some(paragraphs) = object.get("textParagraphStyles") {
        match paragraphs
            .as_array()
            .filter(|paragraphs| paragraphs.len() <= 2000)
        {
            None => invalid.push((
                "textParagraphStyles",
                "textParagraphStyles must be an array of at most 2000 paragraph overrides.".into(),
            )),
            Some(paragraphs) => {
                for (index, patch) in paragraphs.iter().enumerate() {
                    let Some(patch) = patch.as_object() else {
                        invalid.push((
                            "textParagraphStyles",
                            format!("Paragraph override {index} must be an object."),
                        ));
                        continue;
                    };
                    if patch.keys().any(|key| {
                        !PARAGRAPH_FIELDS.iter().any(|(field, _, _)| key == field)
                            && !matches!(
                                key.as_str(),
                                "textHorizontalAlign"
                                    | "textTabStops"
                                    | "textBarTabStops"
                                    | "textAbsoluteRightEdge"
                                    | "textListStyle"
                                    | "textListPrefix"
                                    | "textListPostfix"
                                    | "textListStart"
                                    | "textListLevel"
                            )
                    }) {
                        invalid.push((
                            "textParagraphStyles",
                            format!("Paragraph override {index} contains an unsupported field."),
                        ));
                        continue;
                    }
                    if patch.get("textHorizontalAlign").is_some_and(|value| {
                        !matches!(
                            value.as_str(),
                            Some("left" | "center" | "right" | "justify" | "natural")
                        )
                    }) {
                        invalid.push((
                            "textParagraphStyles",
                            format!("Paragraph override {index} has an invalid alignment."),
                        ));
                    }
                    for (_, reason) in
                        invalid_paragraph_fields(&resolve_paragraph_style(object, index))
                    {
                        invalid.push((
                            "textParagraphStyles",
                            format!("Paragraph override {index}: {reason}"),
                        ));
                    }
                }
            }
        }
    }
    invalid
}

pub(crate) fn validate_paragraph_style(object: &Map<String, Value>) -> Result<(), String> {
    match invalid_paragraph_fields(object).into_iter().next() {
        Some((_, reason)) => Err(reason),
        None => Ok(()),
    }
}

/// Paragraph layout in output pixels. Unset fields take the neutral value that reproduces
/// text without paragraph settings: line height 1.06, no indents and no spacing. The tab stops
/// stay in slide pixels; `scale` converts them to output pixels. With a list, a negative
/// first-line indent is the hanging indent that holds the marker.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ParagraphStyle {
    pub line_height_multiple: f32,
    pub spacing_before: f32,
    pub spacing_after: f32,
    pub first_line_indent: f32,
    pub left_indent: f32,
    pub right_indent: f32,
    pub tabs: Option<TabStops>,
    pub list: Option<TextList>,
    pub scale: f32,
    pub line_spacing: f32,
    pub minimum_height: f32,
    pub maximum_height: f32,
    pub level: usize,
    source: Map<String, Value>,
}

impl ParagraphStyle {
    /// `scale` converts slide pixels to output pixels, like the font size.
    pub fn from_style(style: &Map<String, Value>, scale: f32) -> Self {
        let number = |field: &str, neutral: f64| {
            style.get(field).and_then(Value::as_f64).unwrap_or(neutral) as f32
        };
        Self {
            line_height_multiple: number("textLineHeightMultiple", 1.0),
            spacing_before: number("textParagraphSpacingBefore", 0.0) * scale,
            spacing_after: number("textParagraphSpacingAfter", 0.0) * scale,
            first_line_indent: number("textFirstLineIndent", 0.0) * scale,
            left_indent: number("textLeftIndent", 0.0) * scale,
            right_indent: number("textRightIndent", 0.0) * scale,
            tabs: TabStops::from_style(style),
            list: TextList::from_style(style),
            scale,
            line_spacing: number("textLineSpacing", 0.0) * scale,
            minimum_height: number("textMinimumLineHeight", 0.0) * scale,
            maximum_height: number("textMaximumLineHeight", 0.0) * scale,
            level: style
                .get("textListLevel")
                .and_then(Value::as_u64)
                .unwrap_or(0) as usize,
            source: style.clone(),
        }
    }

    pub fn line_height(&self, font_size: f32) -> f32 {
        let height = (font_size * BASE_LINE_HEIGHT * self.line_height_multiple + self.line_spacing)
            .max(self.minimum_height);
        if self.maximum_height > 0.0 {
            height.min(self.maximum_height)
        } else {
            height
        }
    }

    /// Width of the marker column, when the first line hangs.
    fn hang(&self) -> Option<f32> {
        (self.first_line_indent < 0.0).then_some(-self.first_line_indent)
    }
}

/// Glyph metadata bits that carry a run's decorations to the drawing pass.
pub(crate) const RUN_UNDERLINE: usize = 1;
pub(crate) const RUN_STRIKETHROUGH: usize = 2;
const RUN_UNDERLINE_REVEAL_SHIFT: usize = 2;
const RUN_SOURCE_SHIFT: usize = usize::BITS as usize / 2;

/// Character formatting of one run of the authored text. `range` is in bytes of that text; a
/// field left `None` or `false` keeps the text box style. `font_size` is in output pixels.
#[derive(Clone)]
pub(crate) struct TextRun {
    pub range: Range<usize>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: bool,
    pub underline_reveal_index: Option<usize>,
    pub source_index: usize,
    pub strikethrough: bool,
    pub color: Option<Color>,
    pub family: Option<String>,
    pub font_size: Option<f32>,
    pub letter_spacing: Option<f32>,
    pub superscript: Option<i32>,
}

impl TextRun {
    pub fn is_plain(&self) -> bool {
        self.bold.is_none()
            && self.italic.is_none()
            && !self.underline
            && !self.strikethrough
            && self.color.is_none()
            && self.family.is_none()
            && self.font_size.is_none()
            && self.letter_spacing.is_none()
            && self.superscript.is_none()
    }

    fn attrs<'a>(
        &'a self,
        base: &Attrs<'a>,
        paragraph: &ParagraphStyle,
        base_font_size: f32,
    ) -> Attrs<'a> {
        let mut attrs = base.clone();
        if let Some(family) = &self.family {
            attrs = attrs.family(Family::Name(family));
        }
        if let Some(size) = self.font_size {
            attrs = attrs.metrics(Metrics::new(size, paragraph.line_height(size)));
        }
        if let Some(level) = self.superscript {
            attrs = with_superscript(attrs, level);
        }
        if let Some(spacing) = self.letter_spacing {
            attrs =
                attrs.letter_spacing(spacing / self.font_size.unwrap_or(base_font_size).max(1.0));
        }
        match self.bold {
            Some(true) => attrs = attrs.weight(Weight::BOLD),
            Some(false) => attrs = attrs.weight(Weight::NORMAL),
            None => {}
        }
        match self.italic {
            Some(true) => attrs = attrs.style(Style::Italic),
            Some(false) => attrs = attrs.style(Style::Normal),
            None => {}
        }
        if let Some(color) = self.color {
            attrs = attrs.color(color);
        }
        let mut decorations = 0;
        if self.underline {
            decorations |= RUN_UNDERLINE;
        }
        if self.strikethrough {
            decorations |= RUN_STRIKETHROUGH;
        }
        if let Some(index) = self.underline_reveal_index {
            decorations |= index.saturating_add(1) << RUN_UNDERLINE_REVEAL_SHIFT;
        }
        decorations |= self.source_index.saturating_add(1) << RUN_SOURCE_SHIFT;
        attrs.metadata(decorations)
    }
}

pub(crate) fn with_superscript(mut attrs: Attrs<'_>, level: i32) -> Attrs<'_> {
    let mut features = attrs.font_features.clone();
    features.set(FeatureTag::new(b"sups"), u32::from(level > 0));
    features.set(FeatureTag::new(b"subs"), u32::from(level < 0));
    attrs = attrs.font_features(features);
    attrs
}

/// The parts of `runs` inside `start..end`, as ranges of that slice shifted right by `shift`.
fn runs_within(runs: &[TextRun], start: usize, end: usize, shift: usize) -> Vec<TextRun> {
    runs.iter()
        .filter_map(|run| {
            let from = run.range.start.max(start);
            let to = run.range.end.min(end);
            (from < to).then(|| TextRun {
                range: from - start + shift..to - start + shift,
                ..run.clone()
            })
        })
        .collect()
}

pub(crate) struct TextLayoutSpec<'a> {
    pub font_size: f32,
    /// Wrap width in output pixels; `None` lays every paragraph out on one line.
    pub width: Option<f32>,
    pub attrs: &'a Attrs<'a>,
    pub align: Align,
    pub paragraph: ParagraphStyle,
    /// Formatting runs over the text passed to `layout_text`; empty when the box style applies throughout.
    pub runs: &'a [TextRun],
}

/// One laid-out piece of a paragraph at an offset inside the text box.
pub(crate) struct TextBlock {
    buffer: Buffer,
    pub x: f32,
    pub y: f32,
    pub paragraph_index: usize,
    run_limit: usize,
    justified_glyphs: Vec<Option<Vec<LayoutGlyph>>>,
    justified_width: Option<f32>,
    bar_rules: Vec<(f32, f32, f32, f32)>,
}

impl TextBlock {
    pub fn runs(&self) -> impl Iterator<Item = LayoutRun<'_>> {
        self.buffer
            .layout_runs()
            .take(self.run_limit)
            .enumerate()
            .map(|(index, mut run)| {
                if let Some(Some(glyphs)) = self.justified_glyphs.get(index) {
                    run.glyphs = glyphs;
                    run.line_w = self.justified_width.unwrap_or(run.line_w);
                }
                run
            })
    }

    fn justify_tab(&mut self, width: f32) {
        self.justified_glyphs = self
            .buffer
            .layout_runs()
            .map(|run| {
                let spaces = run
                    .glyphs
                    .iter()
                    .filter(|glyph| {
                        run.text
                            .get(glyph.start..glyph.end)
                            .is_some_and(|text| text.chars().all(char::is_whitespace))
                    })
                    .count();
                if spaces == 0 || run.line_w >= width {
                    return None;
                }
                let extra = (width - run.line_w) / spaces as f32;
                let mut shift = 0.0;
                let glyphs = run
                    .glyphs
                    .iter()
                    .map(|glyph| {
                        let mut glyph = glyph.clone();
                        glyph.x += shift;
                        if run
                            .text
                            .get(glyph.start..glyph.end)
                            .is_some_and(|text| text.chars().all(char::is_whitespace))
                        {
                            shift += extra;
                        }
                        glyph
                    })
                    .collect();
                Some(glyphs)
            })
            .collect();
        self.justified_width = Some(width);
    }

    fn height(&self) -> f32 {
        self.runs()
            .map(|run| run.line_top + run.line_height)
            .fold(0.0, f32::max)
    }
}

pub(crate) fn text_height(blocks: &[TextBlock]) -> f32 {
    blocks
        .iter()
        .map(|block| block.y + block.height())
        .fold(0.0, f32::max)
}

/// Lays out `text` paragraph by paragraph: left and right indents narrow the wrap width, the
/// first line of a paragraph starts at its first-line indent, and spacing separates paragraphs.
/// Without tab settings a tab is a single space, as in the renderer's `pre-line` text. With a
/// list, every non-empty paragraph gets a marker (see `layout_paragraph`).
pub(crate) fn layout_text(
    font_system: &mut FontSystem,
    text: &str,
    spec: &TextLayoutSpec,
) -> Vec<TextBlock> {
    let mut blocks = Vec::new();
    let mut y = 0.0;
    let paragraphs = split_paragraphs(text);
    let mut lists: Vec<Option<(TextList, i64)>> = vec![None; 9];
    for (index, (start, text)) in paragraphs.iter().enumerate() {
        let style = resolve_paragraph_style(&spec.paragraph.source, index);
        let paragraph = if spec.paragraph.source.contains_key("textParagraphStyles") {
            ParagraphStyle::from_style(&style, spec.paragraph.scale)
        } else {
            spec.paragraph.clone()
        };
        let marker = paragraph
            .list
            .as_ref()
            .filter(|_| !text.is_empty())
            .map(|list| {
                let level = paragraph.level;
                for child in lists.iter_mut().skip(level + 1) {
                    *child = None;
                }
                let state = lists[level].get_or_insert_with(|| (list.clone(), list.start()));
                if !state.0.same_format(list) {
                    *state = (list.clone(), list.start());
                }
                let marker = list.marker(state.1);
                state.1 += 1;
                marker
            });
        let align = match style.get("textHorizontalAlign").and_then(Value::as_str) {
            Some("left") => Align::Left,
            Some("right") => Align::Right,
            Some("center") => Align::Center,
            Some("justify") => Align::Justified,
            _ => spec.align,
        };
        let mut local = TextLayoutSpec {
            paragraph,
            align,
            ..*spec
        };
        if let (Some(width), Some(edge)) = (
            local.width,
            style.get("textAbsoluteRightEdge").and_then(Value::as_f64),
        ) {
            local.paragraph.right_indent = width - edge as f32 * local.paragraph.scale;
        }
        if index > 0 {
            y += local.paragraph.spacing_before;
        }
        let text = if local.paragraph.tabs.is_none() && text.contains('\t') {
            Cow::Owned(text.replace('\t', " "))
        } else {
            Cow::Borrowed(*text)
        };
        let first_block = blocks.len();
        layout_paragraph(
            font_system,
            &text,
            runs_within(spec.runs, *start, start + text.len(), 0),
            marker.as_deref(),
            &local,
            y,
            &mut blocks,
        );
        for block in &mut blocks[first_block..] {
            block.paragraph_index = index;
        }
        if first_block < blocks.len() {
            let bottom = blocks[first_block..]
                .iter()
                .map(|block| block.y + block.height())
                .fold(y, f32::max);
            let bars = local
                .paragraph
                .source
                .get(BAR_STOPS_FIELD)
                .and_then(|value| parse_bar_stops(value).ok())
                .unwrap_or_default();
            blocks[first_block].bar_rules = bars
                .into_iter()
                .map(|position| {
                    (
                        position as f32 * local.paragraph.scale,
                        y,
                        local.paragraph.scale.max(1.0),
                        (bottom - y).max(1.0),
                    )
                })
                .collect();
        }
        y = text_height(&blocks);
        if index + 1 < paragraphs.len() {
            y += local.paragraph.spacing_after;
        }
    }
    blocks
}

/// Each paragraph with its byte offset in `text`.
fn split_paragraphs(text: &str) -> Vec<(usize, &str)> {
    let mut paragraphs = Vec::new();
    let mut ended_with_break = text.is_empty();
    for (range, ending) in LineIter::new(text) {
        paragraphs.push((range.start, &text[range]));
        ended_with_break = ending != LineEnding::None;
    }
    if ended_with_break {
        paragraphs.push((text.len(), ""));
    }
    paragraphs
}

/// A first line that hangs (negative first-line indent) puts the marker in its own unwrapped
/// block at the line start, in a column as wide as the hang, and the text starts at the left
/// indent on every line. Without a hanging indent the marker and one space are prepended to the
/// text, so the marker follows the first-line indent and wraps with the text.
fn layout_paragraph(
    font_system: &mut FontSystem,
    text: &str,
    runs: Vec<TextRun>,
    marker: Option<&str>,
    spec: &TextLayoutSpec,
    y: f32,
    blocks: &mut Vec<TextBlock>,
) {
    if text.contains('\u{2028}') {
        let mut offset = 0;
        let mut line_y = y;
        for (index, part) in text.split('\u{2028}').enumerate() {
            let mut paragraph = spec.paragraph.clone();
            if index > 0 {
                paragraph.first_line_indent = 0.0;
            }
            let local = TextLayoutSpec { paragraph, ..*spec };
            layout_paragraph(
                font_system,
                part,
                runs_within(&runs, offset, offset + part.len(), 0),
                if index == 0 { marker } else { None },
                &local,
                line_y,
                blocks,
            );
            line_y = text_height(blocks);
            offset += part.len() + '\u{2028}'.len_utf8();
        }
        return;
    }
    let paragraph = &spec.paragraph;
    let hang = marker.and(paragraph.hang());
    let mut first_line_indent = paragraph.first_line_indent;
    let (text, runs) = match (marker, hang) {
        (Some(marker), Some(_)) => {
            let line_start = paragraph.left_indent + paragraph.first_line_indent;
            blocks.push(new_block(
                font_system,
                marker,
                &[],
                spec,
                None,
                line_start,
                y,
            ));
            first_line_indent = 0.0;
            (Cow::Borrowed(text), runs)
        }
        (Some(marker), None) => {
            let prefix = marker.len() + 1;
            let runs = runs_within(&runs, 0, text.len(), prefix);
            (Cow::Owned(format!("{marker} {text}")), runs)
        }
        (None, _) => (Cow::Borrowed(text), runs),
    };
    let text = text.as_ref();
    let first_x = paragraph.left_indent + first_line_indent;
    if let Some(tabs) = paragraph.tabs.as_ref().filter(|_| text.contains('\t')) {
        layout_tabbed(font_system, text, &runs, tabs, hang, spec, y, blocks);
        return;
    }
    let Some(width) = spec.width else {
        blocks.push(new_block(font_system, text, &runs, spec, None, first_x, y));
        return;
    };
    let available = (width - paragraph.left_indent - paragraph.right_indent).max(1.0);
    let first_width = (available - first_line_indent).max(1.0);
    let mut head = new_block(
        font_system,
        text,
        &runs,
        spec,
        Some(first_width),
        first_x,
        y,
    );
    // cosmic-text wraps every line at one width, so a paragraph with a first-line indent is laid
    // out twice: once at the first-line width, keeping only its first line, and the remainder
    // at the full width.
    let split = if first_line_indent == 0.0 {
        None
    } else {
        first_line_end(&head)
    };
    let Some(split) = split else {
        blocks.push(head);
        return;
    };
    head.run_limit = 1;
    let tail_y = y + head.height();
    blocks.push(head);
    // Spaces and tabs at a soft wrap collapse, as they do in the renderer's `pre-line` text.
    let rest = text[split..].trim_start_matches([' ', '\t']);
    if !rest.is_empty() {
        let rest_runs = runs_within(&runs, text.len() - rest.len(), text.len(), 0);
        blocks.push(new_block(
            font_system,
            rest,
            &rest_runs,
            spec,
            Some(available),
            paragraph.left_indent,
            tail_y,
        ));
    }
}

/// Tab segments wrap independently inside their available columns. The next paragraph starts
/// below the tallest column; decimal stops anchor the separator and justified stops distribute
/// word spacing. A hanging marker reserves the start of the first column.
fn layout_tabbed(
    font_system: &mut FontSystem,
    text: &str,
    runs: &[TextRun],
    tabs: &TabStops,
    hang: Option<f32>,
    spec: &TextLayoutSpec,
    y: f32,
    blocks: &mut Vec<TextBlock>,
) {
    let paragraph = &spec.paragraph;
    let scale = f64::from(paragraph.scale);
    let mut cursor = paragraph.left_indent + paragraph.first_line_indent + hang.unwrap_or(0.0);
    let mut cursor_y = y;
    let segments: Vec<_> = text.split('\t').collect();
    let mut start = 0;
    let mut anchor: Option<crate::native_text_tabs::TabStop> = None;
    let mut space_before = false;
    for (index, source) in segments.iter().enumerate() {
        let prefix = usize::from(space_before);
        let segment = if space_before {
            Cow::Owned(format!(" {source}"))
        } else {
            Cow::Borrowed(*source)
        };
        let segment_runs = runs_within(runs, start, start + source.len(), prefix);
        start += source.len() + 1;
        let local = TextLayoutSpec {
            align: Align::Left,
            paragraph: spec.paragraph.clone(),
            ..*spec
        };
        let measured = new_block(
            font_system,
            &segment,
            &segment_runs,
            &local,
            None,
            0.0,
            cursor_y,
        );
        let measured_width = measured
            .runs()
            .map(|run| run.line_w)
            .fold(0.0_f32, f32::max);
        let position = if let Some(stop) = anchor {
            let stop_x = (stop.position * scale) as f32;
            match stop.alignment {
                crate::native_text_tabs::TabAlignment::Right => {
                    (stop_x - measured_width).max(cursor)
                }
                crate::native_text_tabs::TabAlignment::Center => {
                    (stop_x - measured_width / 2.0).max(cursor)
                }
                crate::native_text_tabs::TabAlignment::Decimal => {
                    let separator = segment.find(['.', ',']).unwrap_or(segment.len());
                    let prefix = new_block(
                        font_system,
                        &segment[..separator],
                        &runs_within(&segment_runs, 0, separator, 0),
                        &local,
                        None,
                        0.0,
                        cursor_y,
                    );
                    let prefix_width = prefix.runs().map(|run| run.line_w).fold(0.0_f32, f32::max);
                    (stop_x - prefix_width).max(cursor)
                }
                _ => stop_x.max(cursor),
            }
        } else {
            cursor
        };
        if let Some(leader) = anchor.and_then(|stop| stop.leader) {
            let glyph = match leader {
                "dot" => ".",
                "equal" => "=",
                "hyphen" => "-",
                "thick" => "━",
                "underline" => "_",
                _ => "",
            };
            let sample = new_block(font_system, glyph, &[], &local, None, cursor, cursor_y);
            let glyph_width = sample.runs().map(|run| run.line_w).fold(0.0_f32, f32::max);
            if glyph_width > 0.0 {
                let count = ((position - cursor) / glyph_width).floor().max(0.0) as usize;
                if count > 0 {
                    blocks.push(new_block(
                        font_system,
                        &glyph.repeat(count),
                        &[],
                        &local,
                        None,
                        cursor,
                        cursor_y,
                    ));
                }
            }
        }
        let width = spec
            .width
            .map(|width| (width - paragraph.right_indent - position).max(1.0));
        let mut block = new_block(
            font_system,
            &segment,
            &segment_runs,
            &local,
            width,
            position,
            cursor_y,
        );
        let split = first_line_end(&block);
        if let Some(split) = split {
            block.run_limit = 1;
            let tail_y = cursor_y + block.height();
            blocks.push(block);
            let tail_text = segment[split..].trim_start_matches(' ');
            let tail_start = segment.len() - tail_text.len();
            let tail_runs = runs_within(&segment_runs, tail_start, segment.len(), 0);
            let available = spec
                .width
                .map(|width| (width - paragraph.right_indent - paragraph.left_indent).max(1.0));
            block = new_block(
                font_system,
                tail_text,
                &tail_runs,
                &local,
                available,
                paragraph.left_indent,
                tail_y,
            );
        }
        let last = block.runs().last().map(|run| (run.line_top, run.line_w));
        if let Some((line_top, line_width)) = last {
            cursor = block.x + line_width;
            cursor_y = block.y + line_top;
        }
        let next = tabs.next_after(f64::from(cursor) / scale);
        if anchor
            .is_some_and(|stop| stop.alignment == crate::native_text_tabs::TabAlignment::Justify)
        {
            if let Some(stop) = next {
                block.justify_tab(((stop.position * scale) as f32 - block.x).max(1.0));
            }
        }
        blocks.push(block);
        anchor = next;
        space_before = index + 1 < segments.len() && next.is_none();
    }
}

/// Byte offset where the first wrapped line ends, or `None` when the paragraph fits one line.
fn first_line_end(block: &TextBlock) -> Option<usize> {
    let mut runs = block.buffer.layout_runs();
    let first = runs.next()?;
    runs.next()?;
    first.glyphs.iter().map(|glyph| glyph.end).max()
}

fn new_block(
    font_system: &mut FontSystem,
    text: &str,
    runs: &[TextRun],
    spec: &TextLayoutSpec,
    width: Option<f32>,
    x: f32,
    y: f32,
) -> TextBlock {
    let metrics = Metrics::new(spec.font_size, spec.paragraph.line_height(spec.font_size));
    let mut buffer = Buffer::new(font_system, metrics);
    buffer.set_size(font_system, width, None);
    buffer.set_wrap(
        font_system,
        if width.is_some() {
            Wrap::WordOrGlyph
        } else {
            Wrap::None
        },
    );
    if runs.is_empty() {
        buffer.set_text(
            font_system,
            text,
            spec.attrs,
            Shaping::Advanced,
            (spec
                .paragraph
                .source
                .get("textHorizontalAlign")
                .and_then(Value::as_str)
                != Some("natural"))
            .then_some(spec.align),
        );
    } else {
        // A line is as tall as its tallest glyph override, so once one run sizes itself the
        // unsized text needs its own override, or a smaller run would shrink the line.
        let base = if runs.iter().any(|run| run.font_size.is_some()) {
            spec.attrs.clone().metrics(metrics)
        } else {
            spec.attrs.clone()
        };
        let mut spans = Vec::new();
        let mut cursor = 0;
        for run in runs {
            if run.range.start > cursor {
                spans.push((&text[cursor..run.range.start], base.clone()));
            }
            spans.push((
                &text[run.range.clone()],
                run.attrs(&base, &spec.paragraph, spec.font_size),
            ));
            cursor = run.range.end;
        }
        if cursor < text.len() {
            spans.push((&text[cursor..], base.clone()));
        }
        buffer.set_rich_text(
            font_system,
            spans,
            spec.attrs,
            Shaping::Advanced,
            (spec
                .paragraph
                .source
                .get("textHorizontalAlign")
                .and_then(Value::as_str)
                != Some("natural"))
            .then_some(spec.align),
        );
    }
    TextBlock {
        buffer,
        x,
        y,
        paragraph_index: 0,
        run_limit: usize::MAX,
        justified_glyphs: Vec::new(),
        justified_width: None,
        bar_rules: Vec::new(),
    }
}

/// Reports glyph coverage in text-box coordinates, like `Buffer::draw` for a single buffer.
pub(crate) fn draw_blocks(
    blocks: &[TextBlock],
    font_system: &mut FontSystem,
    cache: &mut SwashCache,
    color: Color,
    visible_underlines: Option<usize>,
    base_underline: bool,
    paragraph_opacities: Option<&[f32]>,
    child_opacities: Option<&std::collections::HashMap<usize, f32>>,
    initial_visible_underlines: usize,
    isolated_child: Option<usize>,
    mut callback: impl FnMut(i32, i32, u32, u32, Color, f32),
) {
    for block in blocks {
        let paragraph_opacity = paragraph_opacities
            .and_then(|opacities| opacities.get(block.paragraph_index))
            .copied()
            .unwrap_or(1.0);
        if paragraph_opacity <= 0.0 {
            continue;
        }
        if isolated_child.is_none() && !(base_underline && visible_underlines == Some(0)) {
            for &(x, y, width, height) in &block.bar_rules {
                callback(
                    x.round() as i32,
                    y.round() as i32,
                    width.ceil() as u32,
                    height.ceil() as u32,
                    color,
                    paragraph_opacity,
                );
            }
        }
        for run in block.runs() {
            for glyph in run.glyphs {
                if visible_underlines.is_some_and(|count| {
                    !underline_glyph_visible(glyph.metadata, count, base_underline)
                }) {
                    continue;
                }
                let reveal_index =
                    underline_reveal_index(glyph.metadata).or_else(|| base_underline.then_some(0));
                let child_opacity = match (isolated_child, reveal_index) {
                    (Some(isolated), Some(index)) => {
                        if index == isolated {
                            1.0
                        } else {
                            0.0
                        }
                    }
                    (Some(_), None) => 0.0,
                    (None, Some(index)) => child_opacities.map_or(1.0, |opacities| {
                        opacities.get(&index).copied().unwrap_or_else(|| {
                            if index < initial_visible_underlines {
                                1.0
                            } else {
                                0.0
                            }
                        })
                    }),
                    (None, None) => 1.0,
                };
                let reveal_opacity = paragraph_opacity * child_opacity;
                if reveal_opacity <= 0.0 {
                    continue;
                }
                let physical = glyph.physical((block.x, block.y + run.line_y), 1.0);
                let glyph_color = glyph.color_opt.unwrap_or(color);
                cache.with_pixels(
                    font_system,
                    physical.cache_key,
                    glyph_color,
                    |x, y, pixel| {
                        callback(physical.x + x, physical.y + y, 1, 1, pixel, reveal_opacity)
                    },
                );
            }
        }
    }
}

pub(crate) fn underline_glyph_visible(
    metadata: usize,
    visible_count: usize,
    base_underline: bool,
) -> bool {
    if base_underline {
        return visible_count > 0;
    }
    underline_reveal_index(metadata).map_or(true, |index| index < visible_count)
}

pub(crate) fn decoration_source_index(metadata: usize) -> Option<usize> {
    (metadata >> RUN_SOURCE_SHIFT).checked_sub(1)
}

pub(crate) fn underline_reveal_index(metadata: usize) -> Option<usize> {
    let encoded = (metadata & ((1usize << RUN_SOURCE_SHIFT) - 1)) >> RUN_UNDERLINE_REVEAL_SHIFT;
    encoded.checked_sub(1)
}
