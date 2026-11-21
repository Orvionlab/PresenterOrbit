use crate::{native_compositor::NativeFrame, presentation_export};
use cosmic_text::{
    Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, SwashCache, Weight, Wrap,
};
use serde::Deserialize;
use serde_json::Value;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HandoutRequest {
    pub target_type: String,
    pub target_id: String,
    pub target_ids: Option<Vec<String>>,
    pub arrangement_id: Option<String>,
    pub view: String,
    pub paper: String,
    pub columns: usize,
    pub include_information: bool,
    pub include_presentation_notes: bool,
    pub include_date: bool,
    pub include_slide_numbers: bool,
    pub include_slide_labels: bool,
    pub include_slide_notes: bool,
    pub include_disabled: bool,
}

#[derive(Debug, PartialEq)]
pub struct HandoutSection {
    pub title: String,
    pub text: Vec<String>,
    pub slides: Vec<Value>,
    pub frames: Vec<NativeFrame>,
}

pub fn prepare(workspace: &Value, request: &HandoutRequest) -> Result<Vec<HandoutSection>, String> {
    if !matches!(request.view.as_str(), "outline" | "thumbnails")
        || !matches!(request.paper.as_str(), "a4" | "letter")
        || !(1..=4).contains(&request.columns)
    {
        return Err("Invalid handout view, paper or columns.".into());
    }
    if request.view == "thumbnails" && request.include_slide_notes {
        return Err("Slide notes require outline view.".into());
    }
    let targets = targets(workspace, request)?;
    let mut sections = Vec::new();
    let mut occurrence_count = 0;
    let mut pixel_count = 0;
    for (item_id, arrangement_id) in targets {
        let export = presentation_export::ExportRequest {
            item_id,
            format: if request.view == "thumbnails" {
                "pdf"
            } else {
                "text"
            }
            .into(),
            arrangement_id,
            include_disabled: request.include_disabled,
            include_notes: false,
        };
        let (item, _, slides) = presentation_export::select_slides(workspace, &export)?;
        if request.include_slide_labels
            && slides.iter().any(|slide| {
                slide.get("label").is_some_and(|label| {
                    !label.is_null() && !label.is_string() && !label.is_number()
                })
            })
        {
            return Err("Handout slide labels must be text or numbers.".into());
        }
        if serde_json::to_vec(item)
            .map_err(|error| error.to_string())?
            .len()
            > 2_000_000
        {
            return Err("Handout presentation exceeds the 2 MB content limit.".into());
        }
        occurrence_count += slides.len();
        if occurrence_count > 500 {
            return Err("Handout is limited to 500 slide occurrences.".into());
        }
        let title = item["title"]
            .as_str()
            .ok_or("Presentation title is required.")?
            .to_string();
        let mut text = Vec::new();
        if request.include_information {
            for key in [
                "title",
                "artist",
                "author",
                "publisher",
                "copyright",
                "copyrightYear",
                "songNumber",
            ] {
                if let Some(value) = item["ccli"].get(key).filter(|value| !value.is_null()) {
                    text.push(format!(
                        "{key}: {}",
                        value
                            .as_str()
                            .map(str::to_string)
                            .unwrap_or_else(|| value.to_string())
                    ));
                }
            }
        }
        if request.include_presentation_notes {
            if let Some(notes) = item["operatorNotes"]
                .as_str()
                .filter(|text| !text.is_empty())
            {
                text.push(format!("Operator notes: {notes}"));
            }
        }
        let prepared = if request.view == "thumbnails" {
            Some(presentation_export::prepare(workspace, &export)?)
        } else {
            None
        };
        if let Some(prepared) = &prepared {
            pixel_count += prepared
                .dimensions
                .iter()
                .map(|(w, h)| w * h)
                .sum::<usize>();
            if pixel_count > 64_000_000 {
                return Err("Handout thumbnails exceed the 64 megapixel limit.".into());
            }
        }
        sections.push(HandoutSection {
            title,
            text,
            slides,
            frames: match prepared {
                Some(prepared) => presentation_export::render(&prepared)?,
                None => Vec::new(),
            },
        });
    }
    Ok(sections)
}

fn targets(
    workspace: &Value,
    request: &HandoutRequest,
) -> Result<Vec<(String, Option<String>)>, String> {
    if request.target_type == "selection" {
        let ids = request
            .target_ids
            .as_ref()
            .ok_or("Select presentations to print.")?;
        let unique: std::collections::HashSet<_> = ids.iter().collect();
        if ids.is_empty()
            || ids.len() > 500
            || unique.len() != ids.len()
            || ids.iter().any(|id| id.is_empty())
            || !request.target_id.is_empty()
            || request.arrangement_id.is_some()
        {
            return Err("Invalid selected presentation targets.".into());
        }
        return Ok(ids.iter().map(|id| (id.clone(), None)).collect());
    }
    if request.target_ids.is_some() {
        return Err("Selected presentation IDs require a selection target.".into());
    }
    if request.target_type == "presentation" {
        return Ok(vec![(
            request.target_id.clone(),
            request.arrangement_id.clone(),
        )]);
    }
    if request.target_type != "playlist" || request.arrangement_id.is_some() {
        return Err("Invalid handout target or playlist arrangement.".into());
    }
    let playlist = workspace["playlist"]
        .as_array()
        .and_then(|values| {
            values
                .iter()
                .find(|value| value["id"].as_str() == Some(&request.target_id))
        })
        .ok_or("Playlist is not in the active workspace.")?;
    let entries = playlist["items"]
        .as_array()
        .ok_or("Playlist entries are required.")?;
    let mut targets = Vec::new();
    for entry in entries {
        match entry["type"].as_str() {
            Some("presentation") => targets.push((
                entry["itemId"]
                    .as_str()
                    .ok_or("Playlist presentation reference is required.")?
                    .into(),
                match entry.get("arrangementId") {
                    None | Some(Value::Null) => None,
                    Some(value) => Some(
                        value
                            .as_str()
                            .filter(|id| !id.is_empty())
                            .ok_or("Playlist arrangement reference is invalid.")?
                            .into(),
                    ),
                },
            )),
            Some("header" | "placeholder" | "cue" | "media") => {}
            _ => return Err("Unsupported playlist entry type in handout.".into()),
        }
    }
    if targets.is_empty() {
        return Err("Playlist has no presentations to print.".into());
    }
    Ok(targets)
}

const MARGIN: usize = 48;
const LINE: usize = 24;
const MAX_PAGES: usize = 64;

struct Pages {
    frames: Vec<NativeFrame>,
    width: usize,
    height: usize,
    content_height: usize,
    cursor: usize,
    fonts: FontSystem,
    cache: SwashCache,
}

impl Pages {
    fn new(paper: &str) -> Self {
        let (width, height) = if paper == "a4" {
            (794, 1123)
        } else {
            (816, 1056)
        };
        Self {
            frames: Vec::new(),
            width,
            height,
            content_height: (height - MARGIN * 2 - LINE) / LINE * LINE,
            cursor: 0,
            fonts: FontSystem::new(),
            cache: SwashCache::new(),
        }
    }
    fn ensure_page(&mut self, index: usize) -> Result<(), String> {
        if index >= MAX_PAGES {
            return Err("Handout exceeds the 64-page limit. Select a smaller document.".into());
        }
        while self.frames.len() <= index {
            self.frames.push(NativeFrame {
                width: self.width,
                height: self.height,
                pixels: vec![255; self.width * self.height * 4],
                dynamic_media: false,
            });
        }
        Ok(())
    }
    fn new_page(&mut self) {
        self.cursor = self.cursor.div_ceil(self.content_height) * self.content_height;
    }
    fn text(&mut self, text: &str, bold: bool, size: f32) -> Result<(), String> {
        let mut buffer = Buffer::new(&mut self.fonts, Metrics::new(size, LINE as f32));
        buffer.set_size(
            &mut self.fonts,
            Some((self.width - MARGIN * 2) as f32),
            None,
        );
        buffer.set_wrap(&mut self.fonts, Wrap::WordOrGlyph);
        let attrs = Attrs::new().family(Family::SansSerif).weight(if bold {
            Weight::BOLD
        } else {
            Weight::NORMAL
        });
        buffer.set_text(&mut self.fonts, text, &attrs, Shaping::Advanced, None);
        let height = buffer.layout_runs().count().max(1) * LINE;
        if height <= self.content_height
            && self.cursor % self.content_height + height > self.content_height
        {
            self.new_page();
        }
        self.ensure_page((self.cursor + height - 1) / self.content_height)?;
        let cursor = self.cursor;
        let content_height = self.content_height;
        let frames = &mut self.frames;
        let width = self.width;
        buffer.draw(
            &mut self.fonts,
            &mut self.cache,
            Color::rgb(28, 35, 45),
            |x, y, w, h, color| {
                let (r, g, b, a) = color.as_rgba_tuple();
                for dy in 0..h as isize {
                    for dx in 0..w as isize {
                        let local_x = x as isize + dx;
                        let local_y = y as isize + dy;
                        if local_x < 0
                            || local_x >= (width - MARGIN * 2) as isize
                            || local_y < 0
                            || local_y >= height as isize
                        {
                            continue;
                        }
                        let global_y = cursor + local_y as usize;
                        let page = global_y / content_height;
                        let pixel = ((MARGIN + global_y % content_height) * width
                            + MARGIN
                            + local_x as usize)
                            * 4;
                        for (channel, value) in [b, g, r].into_iter().enumerate() {
                            let old = frames[page].pixels[pixel + channel] as u32;
                            frames[page].pixels[pixel + channel] =
                                ((value as u32 * a as u32 + old * (255 - a as u32)) / 255) as u8;
                        }
                    }
                }
            },
        );
        self.cursor += height;
        Ok(())
    }
    fn thumbnails(
        &mut self,
        frames: &[NativeFrame],
        slides: &[Value],
        request: &HandoutRequest,
    ) -> Result<(), String> {
        let gap = 16;
        let cell_width = (self.width - MARGIN * 2 - gap * (request.columns - 1)) / request.columns;
        for (row_index, row) in frames.chunks(request.columns).enumerate() {
            let scaled = row
                .iter()
                .map(|frame| {
                    ((cell_width as f64 * frame.height as f64 / frame.width as f64).round()
                        as usize)
                        .min(400)
                })
                .collect::<Vec<_>>();
            let row_height = scaled.iter().max().copied().ok_or("Empty thumbnail row.")? + LINE * 2;
            if self.cursor % self.content_height + row_height > self.content_height {
                self.new_page();
            }
            let page = self.cursor / self.content_height;
            self.ensure_page(page)?;
            let top = MARGIN + self.cursor % self.content_height;
            for (column, frame) in row.iter().enumerate() {
                let height = scaled[column];
                let width = ((height as f64 * frame.width as f64 / frame.height as f64).round()
                    as usize)
                    .min(cell_width);
                let left = MARGIN + column * (cell_width + gap);
                for y in 0..height {
                    for x in 0..width {
                        let source = ((y * frame.height / height) * frame.width
                            + x * frame.width / width)
                            * 4;
                        let dest = ((top + y) * self.width + left + x) * 4;
                        self.frames[page].pixels[dest..dest + 4]
                            .copy_from_slice(&frame.pixels[source..source + 4]);
                    }
                }
                let index = row_index * request.columns + column;
                let label = slide_heading(&slides[index], index, request);
                self.caption(page, left, top + height + 4, cell_width, &label)?;
            }
            self.cursor += row_height.div_ceil(LINE) * LINE;
        }
        Ok(())
    }
    fn caption(
        &mut self,
        page: usize,
        left: usize,
        top: usize,
        width: usize,
        text: &str,
    ) -> Result<(), String> {
        let mut buffer = Buffer::new(&mut self.fonts, Metrics::new(13.0, 18.0));
        buffer.set_size(&mut self.fonts, Some(width as f32), None);
        buffer.set_wrap(&mut self.fonts, Wrap::WordOrGlyph);
        buffer.set_text(
            &mut self.fonts,
            text,
            &Attrs::new().family(Family::SansSerif),
            Shaping::Advanced,
            None,
        );
        if buffer.layout_runs().count() > 2 {
            return Err(
                "Slide label exceeds the two-line thumbnail caption. Use outline view.".into(),
            );
        }
        buffer.draw(
            &mut self.fonts,
            &mut self.cache,
            Color::rgb(28, 35, 45),
            |x, y, w, h, color| {
                let (r, g, b, a) = color.as_rgba_tuple();
                for dy in 0..h as isize {
                    for dx in 0..w as isize {
                        let x = x as isize + dx;
                        let y = y as isize + dy;
                        if x < 0 || y < 0 || x >= width as isize || y >= 36 {
                            continue;
                        }
                        let pixel = ((top + y as usize) * self.width + left + x as usize) * 4;
                        for (channel, value) in [b, g, r].into_iter().enumerate() {
                            let old = self.frames[page].pixels[pixel + channel] as u32;
                            self.frames[page].pixels[pixel + channel] =
                                ((value as u32 * a as u32 + old * (255 - a as u32)) / 255) as u8;
                        }
                    }
                }
            },
        );
        Ok(())
    }
}

fn slide_heading(slide: &Value, index: usize, request: &HandoutRequest) -> String {
    let mut parts = Vec::new();
    if request.include_slide_numbers {
        parts.push(format!("Slide {}", index + 1));
    }
    if request.include_slide_labels {
        match slide.get("label") {
            Some(Value::String(label)) if !label.is_empty() => parts.push(label.clone()),
            Some(Value::Number(label)) => parts.push(label.to_string()),
            _ => {}
        }
    }
    if slide["disabled"] == true {
        parts.push("disabled".into());
    }
    parts.join(" - ")
}

pub fn render(
    sections: &[HandoutSection],
    request: &HandoutRequest,
) -> Result<Vec<NativeFrame>, String> {
    let mut pages = Pages::new(&request.paper);
    if pages.fonts.db().faces().next().is_none() {
        return Err("Handout rendering requires an installed font.".into());
    }
    for section in sections {
        pages.new_page();
        pages.text(&section.title, true, 20.0)?;
        if request.include_date {
            pages.text(
                &chrono::Local::now().format("%Y-%m-%d").to_string(),
                false,
                14.0,
            )?;
        }
        for text in &section.text {
            pages.text(text, false, 16.0)?;
        }
        pages.text("", false, 16.0)?;
        if request.view == "thumbnails" {
            pages.thumbnails(&section.frames, &section.slides, request)?;
        } else {
            for (index, slide) in section.slides.iter().enumerate() {
                let heading = slide_heading(slide, index, request);
                if !heading.is_empty() {
                    pages.text(&heading, true, 16.0)?;
                }
                pages.text(
                    slide["content"]
                        .as_str()
                        .or_else(|| slide["text"].as_str())
                        .unwrap_or(""),
                    false,
                    16.0,
                )?;
                if request.include_slide_notes {
                    let notes = slide["notes"].as_str().map(str::to_string).or_else(|| {
                        slide["notesRuns"].as_array().map(|runs| {
                            runs.iter()
                                .filter_map(|run| run["text"].as_str())
                                .collect::<String>()
                        })
                    });
                    if let Some(notes) = notes {
                        pages.text(&format!("Notes: {notes}"), false, 14.0)?;
                    }
                }
                pages.text("", false, 16.0)?;
            }
        }
    }
    let count = pages.frames.len();
    for index in 0..count {
        pages.caption(
            index,
            MARGIN,
            pages.height - MARGIN,
            pages.width - MARGIN * 2,
            &format!("Presenter Orbit | {} / {count}", index + 1),
        )?;
    }
    Ok(pages.frames)
}
