use gpui::{
    canvas, point, px, ContentMask, FontStyle as GpuiFontStyle, FontWeight, IndentAdjustment,
    IntoElement, LineFragment, Pixels, SharedString, Styled, TextAlign, TextRun, TruncateFrom,
};
use orbit_host::cue_view_options::{CueViewOptions, FontStyle, Horizontal, Truncate, Vertical};

pub(super) fn text(
    content: String,
    options: &CueViewOptions,
    font_size: Pixels,
    color: gpui::Rgba,
) -> impl IntoElement {
    let options = options.clone();
    canvas(
        move |bounds, window, _| {
            let line_height = font_size * 1.18;
            let count = (f32::from(bounds.size.height) / f32::from(line_height)).floor() as usize;
            if count == 0 || bounds.size.width <= px(0.) || content.is_empty() {
                return None;
            }
            let mut font = window.text_style().font();
            if let Some(family) = &options.font_family {
                font.family = family.clone().into();
            }
            if matches!(options.font_style, FontStyle::Bold | FontStyle::BoldItalic) {
                font.weight = FontWeight::BOLD;
            }
            if matches!(
                options.font_style,
                FontStyle::Italic | FontStyle::BoldItalic
            ) {
                font.style = GpuiFontStyle::Italic;
            }
            let text_system = window.text_system();
            let mut wrapper = text_system.line_wrapper(font.clone(), font_size);
            let run = |text: &str| TextRun {
                len: text.len(),
                font: font.clone(),
                color: color.into(),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let width = bounds.size.width;
            let mut rows = Vec::new();
            for paragraph in content.split('\n') {
                let paragraph = paragraph.trim_end_matches('\r');
                let mut start = 0;
                for boundary in wrapper.wrap_line(
                    &[LineFragment::text(paragraph)],
                    width,
                    IndentAdjustment::NoIndent,
                ) {
                    rows.push((
                        SharedString::from(paragraph[start..boundary.ix].trim_end().to_string()),
                        true,
                    ));
                    start = boundary.ix;
                }
                rows.push((
                    SharedString::from(paragraph[start..].trim().to_string()),
                    false,
                ));
            }
            if rows.len() > count {
                let (index, from) = match options.truncate {
                    Truncate::Start => {
                        rows.drain(..rows.len() - count);
                        (0, TruncateFrom::Start)
                    }
                    Truncate::Middle if count == 1 => {
                        rows[0].0 = format!("{} … {}", rows[0].0, rows[rows.len() - 1].0).into();
                        rows.truncate(1);
                        (0, TruncateFrom::Middle)
                    }
                    Truncate::Middle => {
                        let head = count.div_ceil(2);
                        rows.drain(head..rows.len() - (count - head));
                        (head - 1, TruncateFrom::End)
                    }
                    Truncate::End => {
                        rows.truncate(count);
                        (count - 1, TruncateFrom::End)
                    }
                };
                let line = &mut rows[index];
                let candidate: SharedString = match from {
                    TruncateFrom::Start => format!("…{}", line.0).into(),
                    TruncateFrom::End => format!("{}…", line.0).into(),
                    TruncateFrom::Middle => line.0.clone(),
                };
                let (shortened, _) =
                    wrapper.truncate_line(candidate.clone(), width, "…", &[run(&candidate)], from);
                *line = (shortened, false);
            }
            let spare = bounds.size.height - line_height * rows.len();
            let offset = match options.vertical {
                Vertical::Top => px(0.),
                Vertical::Middle => spare / 2.,
                Vertical::Bottom => spare,
            };
            let align = match options.horizontal {
                Horizontal::Left | Horizontal::Justify => TextAlign::Left,
                Horizontal::Center => TextAlign::Center,
                Horizontal::Right => TextAlign::Right,
            };
            let shape = |text: SharedString| {
                text_system.shape_line(text.clone(), font_size, &[run(&text)], None)
            };
            let mut lines = Vec::new();
            for (index, (text, wraps)) in rows.into_iter().enumerate() {
                let origin = bounds.origin + point(px(0.), offset + line_height * index);
                if options.horizontal == Horizontal::Justify && wraps {
                    let words = text
                        .split_whitespace()
                        .map(|word| shape(word.to_string().into()))
                        .collect::<Vec<_>>();
                    if words.len() > 1 {
                        let occupied = words
                            .iter()
                            .map(|word| word.width)
                            .fold(px(0.), |sum, width| sum + width);
                        let gap = (width - occupied).max(px(0.)) / (words.len() - 1) as f32;
                        let mut x = origin.x;
                        for word in words {
                            let advance = word.width + gap;
                            lines.push((word, point(x, origin.y), TextAlign::Left));
                            x += advance;
                        }
                        continue;
                    }
                }
                lines.push((shape(text), origin, align));
            }
            Some((line_height, lines))
        },
        |bounds, shaped, window, cx| {
            if let Some((line_height, lines)) = shaped {
                window.with_content_mask(Some(ContentMask { bounds }), |window| {
                    for (line, origin, align) in lines {
                        let _ = line.paint(
                            origin,
                            line_height,
                            align,
                            Some(bounds.size.width),
                            window,
                            cx,
                        );
                    }
                });
            }
        },
    )
    .size_full()
}
