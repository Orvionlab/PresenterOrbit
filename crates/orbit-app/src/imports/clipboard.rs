use std::{
    cell::Cell,
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    sync::Arc,
};

use gpui::{div, prelude::*, AnyElement, App, Context, Entity, Focusable, Subscription, Window};
use orbit_ui::{
    controls::{
        growing_textarea_state, range_state, ActionButton, ButtonVariant, Dialog, Field,
        InputEvent, RangeInput, SliderState, Textarea, TextareaState,
    },
    frame::FrameSize,
    ActiveTheme,
};
use serde_json::{json, Value};

use super::{form, Imports};
use crate::{
    automation::form::{Kind, Spec},
    collections::{documents::PRESENTATION_SIZES, song_metadata},
    output::preview::{Frames, Preview, Source},
    services::{choices, entries, text},
    state::runtime::Runtime,
    theme_picker::ThemePicker,
};

pub(super) struct ClipboardImport {
    picker: Entity<ThemePicker>,
    editor: Option<Editor>,
}

struct Editor {
    slides: Vec<Slide>,
    focused: Option<Entity<TextareaState>>,
    frames: Frames,
    font: Entity<SliderState>,
    zoom: Entity<SliderState>,
}

struct Slide {
    text: Entity<TextareaState>,
    preview: Entity<Preview>,
    preview_key: Cell<Option<u64>>,
    _events: Subscription,
}

fn size(draft: &Value) -> Result<(u32, u32), String> {
    draft["size"]
        .as_str()
        .and_then(|index| index.parse::<usize>().ok())
        .and_then(|index| PRESENTATION_SIZES.get(index).copied())
        .ok_or_else(|| "clipboardImport.invalidSize".into())
}

fn clipboard_text(draft: &Value) -> Result<&str, String> {
    draft["content"]
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "status.import.clipboardEmpty".into())
}

fn delimiter_count(draft: &Value) -> Result<f64, String> {
    draft["splitUnits"]
        .as_f64()
        .filter(|units| *units >= 1. && units.fract() == 0.)
        .ok_or_else(|| "clipboardImport.invalidCount".into())
}

fn parsed(draft: &Value) -> Result<Vec<String>, String> {
    Ok(orbit_host::authoring::split_presentation_text(
        clipboard_text(draft)?,
        draft["splitMode"]
            .as_str()
            .ok_or("textImport.invalidResponse")?,
        "",
        delimiter_count(draft)?,
    ))
}

impl ClipboardImport {
    pub(super) fn configure_mutation(
        &self,
        mutation: &mut Value,
        draft: &Value,
        cx: &App,
    ) -> Result<(), String> {
        let (width, height) = size(draft)?;
        mutation["size"] = json!({"width": width, "height": height});
        mutation["presentationThemeId"] = draft["themeId"].clone();
        if draft["playlistId"]
            .as_str()
            .is_some_and(|id| !id.is_empty())
        {
            mutation["playlistId"] = draft["playlistId"].clone();
        }
        if let Some(editor) = &self.editor {
            let contents: Vec<_> = editor
                .slides
                .iter()
                .map(|slide| slide.text.read(cx).value().to_string())
                .collect();
            mutation["slideContents"] = json!(contents);
            let metadata = song_metadata::Draft {
                title: text(draft, "title"),
                author: text(draft, "author"),
                publisher: text(draft, "publisher"),
                copyright_year: text(draft, "copyrightYear"),
                song_number: text(draft, "songNumber"),
            };
            let mut item = song_metadata::apply(&json!({}), &metadata)?;
            item["ccli"]["artistCredits"] = json!(text(draft, "artist"));
            item["ccli"]["display"] = json!(draft["displayCopyright"] == true);
            mutation["ccli"] = item["ccli"].clone();
        }
        clipboard_text(draft)?;
        delimiter_count(draft)?;
        Ok(())
    }
}

impl Imports {
    pub(crate) fn load_clipboard(&mut self, cx: &mut Context<Self>) {
        let clipboard = cx
            .read_from_clipboard()
            .and_then(|item| item.text())
            .unwrap_or_default();
        let content = self.runtime.content.read(cx);
        let library = content
            .library(&self.runtime.ui.read(cx).selected_library_id)
            .or_else(|| content.libraries.first())
            .map(|library| library.id.clone())
            .unwrap_or_default();
        self.form.draft["libraryId"] = json!(library);
        self.form.draft["playlistId"] = json!("");
        self.form.draft["size"] = json!("0");
        self.form.draft["themeId"] = json!(content.last_used_theme);
        self.form.draft["content"] = json!(clipboard.replace("\r\n", "\n").replace('\r', "\n"));
        self.form.draft["title"] =
            json!(Runtime::translator(cx).t("collections.default.clipboardPresentation"));
        if clipboard.trim().is_empty() {
            self.status = "status.import.clipboardEmpty".into();
        }
        self.clipboard = Some(ClipboardImport {
            picker: cx.new(|cx| ThemePicker::new(self.runtime.clone(), "clipboard-theme", cx)),
            editor: None,
        });
        cx.notify();
    }

    fn open_clipboard_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let sections = match parsed(&self.form.draft) {
            Ok(sections) => sections,
            Err(error) => {
                self.status = error;
                cx.notify();
                return;
            }
        };
        let frames = Frames::default();
        let slides = sections
            .into_iter()
            .map(|text| self.clipboard_slide(text, &frames, window, cx))
            .collect();
        let font = range_state(0.75, 1.75, 0.05, 1., cx);
        let zoom = range_state(1., 4., 1., 2., cx);
        for slider in [&font, &zoom] {
            cx.observe(slider, |_, _, cx| cx.notify()).detach();
        }
        if let Some(clipboard) = &mut self.clipboard {
            clipboard.editor = Some(Editor {
                slides,
                focused: None,
                frames,
                font,
                zoom,
            });
        }
        self.status.clear();
        cx.notify();
    }

    fn clipboard_slide(
        &self,
        text: String,
        frames: &Frames,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Slide {
        let text = growing_textarea_state(text, 1, 18, window, cx);
        let preview = cx.new(|cx| {
            let mut preview = Preview::new(self.runtime.host.clone(), frames.clone(), cx);
            preview.show_checkerboard(cx);
            preview
        });
        let events = cx.subscribe(&text, |this, input, event: &InputEvent, cx| {
            if let Some(editor) = this
                .clipboard
                .as_mut()
                .and_then(|clipboard| clipboard.editor.as_mut())
            {
                if matches!(event, InputEvent::Focus) {
                    editor.focused = Some(input);
                }
                cx.notify();
            }
        });
        Slide {
            text,
            preview,
            preview_key: Cell::new(None),
            _events: events,
        }
    }

    fn clipboard_split(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self
            .clipboard
            .as_ref()
            .and_then(|clipboard| clipboard.editor.as_ref())
        else {
            return;
        };
        let Some(state) = editor.focused.clone() else {
            return;
        };
        let Some(index) = editor.slides.iter().position(|slide| slide.text == state) else {
            return;
        };
        let input = state.read(cx);
        let value = input.value().to_string();
        let cursor = value.floor_char_boundary(input.cursor().min(value.len()));
        let slide = self.clipboard_slide(value[cursor..].to_string(), &editor.frames, window, cx);
        let focus = slide.text.read(cx).focus_handle(cx);
        if let Some(editor) = self
            .clipboard
            .as_mut()
            .and_then(|clipboard| clipboard.editor.as_mut())
        {
            editor.focused = Some(slide.text.clone());
            editor.slides.insert(index + 1, slide);
        }
        state.update(cx, |state, cx| {
            state.set_value(value[..cursor].to_string(), window, cx)
        });
        focus.focus(window, cx);
        cx.notify();
    }

    pub(super) fn clipboard_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Dialog {
        let theme = *cx.theme();
        let m = theme.metrics;
        let t = Runtime::translator(cx);
        let clipboard = self.clipboard.as_ref().expect("clipboard import is active");
        let editing = clipboard.editor.is_some();
        let picker = clipboard.picker.clone();
        let selected = text(&self.form.draft, "themeId");
        let owner = cx.weak_entity();
        picker.update(cx, |picker, _| {
            picker.sync(&selected, self.busy, move |value, _, cx| {
                let _ = owner.update(cx, |this, cx| {
                    this.form.draft["themeId"] = json!(value);
                    cx.notify();
                });
            })
        });
        let theme_field = || Field::new(t.t("theme.theme"), picker.clone()).inline();
        let close = self.close.clone();
        let busy = self.busy;
        let can_import = !busy
            && !selected.is_empty()
            && !text(&self.form.draft, "libraryId").is_empty()
            && clipboard_text(&self.form.draft).is_ok();
        let cancel = ActionButton::new("clipboard-cancel", t.t("common.cancel"))
            .disabled(busy)
            .on_click(cx.listener(|this, _, _, cx| (this.close)(cx)));
        let import = ActionButton::new("clipboard-import", t.t("appMenu.import"))
            .variant(ButtonVariant::Primary)
            .disabled(!can_import)
            .on_click(cx.listener(|this, _, _, cx| this.create_text(cx)));
        let width = m.control_height * if editing { 34. } else { 17. };
        let mut dialog = Dialog::new(
            "clipboard-import",
            t.t("appMenu.import"),
            t.t("common.close"),
            move |_, _, cx| {
                if !busy {
                    close(cx);
                }
            },
        )
        .width(width.min(window.viewport_size().width - m.panel_padding * 2.))
        .close_on_backdrop(false);
        if editing {
            let mut metadata = div().flex().gap(m.panel_padding);
            for (index, fields) in [
                vec![
                    ("title", "clipboardImport.title", Kind::Text),
                    ("artist", "clipboardImport.artist", Kind::Text),
                    ("author", "songMetadata.author", Kind::Text),
                    ("publisher", "clipboardImport.publisher", Kind::Text),
                ],
                vec![
                    (
                        "displayCopyright",
                        "clipboardImport.displayCopyright",
                        Kind::Check,
                    ),
                    ("songNumber", "songMetadata.songNumber", Kind::Text),
                    ("copyrightYear", "songMetadata.copyrightYear", Kind::Text),
                ],
            ]
            .into_iter()
            .enumerate()
            {
                let mut column = div().flex_1().min_w_0().flex().flex_col().gap(m.space_xs);
                for (path, label, kind) in fields {
                    column = column.child(self.form.field(
                        "clipboard",
                        &Spec::new(path, t.t(label), kind),
                        form,
                        window,
                        cx,
                    ));
                }
                if index == 1 {
                    column = column.child(theme_field());
                }
                metadata = metadata.child(column);
            }
            dialog = dialog
                .action(cancel)
                .action(import)
                .child(metadata)
                .child(self.clipboard_reflow(window, cx));
        } else {
            let workspace = &self.runtime.content.read(cx).workspace;
            let libraries = choices(&workspace["libraries"], "title");
            let playlists = entries(&workspace["playlist"])
                .into_iter()
                .filter(|entry| entry["type"] != "folder")
                .map(|entry| (text(&entry, "id"), text(&entry, "title")));
            let playlists = std::iter::once((String::new(), t.t("newPresentation.noPlaylist")))
                .chain(playlists)
                .collect();
            let sizes = PRESENTATION_SIZES
                .iter()
                .enumerate()
                .map(|(i, (w, h))| (i.to_string(), format!("{w} × {h}")))
                .collect();
            let parsing = [
                Spec::new(
                    "splitMode",
                    t.t("textImport.splitBy"),
                    Kind::Choice(vec![
                        ("paragraph".into(), t.t("clipboardImport.paragraphBreak")),
                        ("line".into(), t.t("clipboardImport.lineBreak")),
                    ]),
                ),
                Spec::new("splitUnits", t.t("textImport.perSlide"), Kind::Number),
            ];
            let mut parts = div()
                .id("clipboard-options")
                .flex()
                .flex_col()
                .gap(m.space_sm)
                .max_h(window.viewport_size().height - m.row_height * 7.)
                .overflow_y_scroll();
            for (title, specs) in [
                ("clipboardImport.parsing", Vec::from(parsing)),
                (
                    "songMetadata.presentation",
                    vec![Spec::new("size", t.t("theme.size"), Kind::Choice(sizes))],
                ),
                (
                    "clipboardImport.destination",
                    vec![
                        Spec::new(
                            "libraryId",
                            t.t("textImport.library"),
                            Kind::Choice(libraries),
                        ),
                        Spec::new(
                            "playlistId",
                            t.t("collections.menu.playlist"),
                            Kind::Choice(playlists),
                        ),
                    ],
                ),
            ] {
                let mut controls = div()
                    .flex()
                    .flex_col()
                    .gap(m.space_xs)
                    .p(m.space_sm)
                    .bg(theme.colors.panel_bg_strong);
                for spec in specs {
                    controls =
                        controls.child(self.form.field("clipboard", &spec, form, window, cx));
                }
                if title == "songMetadata.presentation" {
                    controls = controls.child(theme_field());
                }
                parts = parts.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(m.space_xs)
                        .child(div().text_size(m.font_sm).child(t.t(title)))
                        .child(controls),
                );
            }
            dialog = dialog.child(parts).footer(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        ActionButton::new("clipboard-edit", t.t("common.edit"))
                            .disabled(!can_import)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_clipboard_editor(window, cx)
                            })),
                    )
                    .child(div().flex().gap(m.space_xs).child(cancel).child(import)),
            );
        }
        if !self.status.is_empty() {
            dialog = dialog.child(
                div()
                    .text_color(theme.colors.danger)
                    .child(t.t(&self.status)),
            );
        }
        dialog
    }

    fn clipboard_reflow(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(editor) = self
            .clipboard
            .as_ref()
            .and_then(|clipboard| clipboard.editor.as_ref())
        else {
            return div().into_any_element();
        };
        let theme = *cx.theme();
        let m = theme.metrics;
        let t = Runtime::translator(cx);
        let font = m.font_base * editor.font.read(cx).value().end();
        let columns = 5 - editor.zoom.read(cx).value().end() as u16;
        let resolution = size(&self.form.draft).ok();
        let workspace = self.runtime.content.read(cx);
        let assignment = &self.form.draft["themeId"];
        let layout = orbit_host::authoring::slide_theme(
            &workspace.workspace,
            &json!({"presentationThemeId": assignment}),
        );
        let mut layout_hash = DefaultHasher::new();
        layout.hash(&mut layout_hash);
        let layout_key = layout_hash.finish();
        let mut list = div()
            .id("clipboard-slides")
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .overflow_y_scroll()
            .gap(m.space_xs)
            .p(m.space_sm);
        let mut grid = div()
            .id("clipboard-previews")
            .grid()
            .grid_cols(columns)
            .content_start()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .border_l_1()
            .border_color(theme.colors.border)
            .overflow_y_scroll()
            .gap(m.space_sm)
            .p(m.space_sm);
        for (index, slide) in editor.slides.iter().enumerate() {
            let number = index + 1;
            let state = slide.text.clone();
            let handle = state.read(cx).focus_handle(cx);
            let focused = handle.is_focused(window);
            list = list.child(
                div()
                    .flex()
                    .flex_col()
                    .flex_none()
                    .gap(m.space_xs)
                    .child(div().text_size(m.font_sm).child(format!("{number}.")))
                    .child(
                        div()
                            .debug_selector(move || format!("clipboard-slide-text-{index}"))
                            .child(
                                Textarea::new(("clipboard-text", state.entity_id()), &state)
                                    .grow()
                                    .text_size(font)
                                    .disabled(self.busy),
                            ),
                    ),
            );
            let mut thumb = div()
                .relative()
                .w_full()
                .overflow_hidden()
                .bg(theme.colors.preview_matte_bg);
            if let Some((width, height)) = resolution {
                thumb = thumb.aspect_ratio(width as f32 / height as f32);
                if let Some(layout) = layout.as_ref().filter(|_| !cfg!(test)) {
                    let content = slide.text.read(cx).value();
                    let mut hasher = DefaultHasher::new();
                    (layout_key, width, height, content.as_str()).hash(&mut hasher);
                    let key = hasher.finish();
                    if slide.preview_key.get() != Some(key) {
                        let raw = json!({"content": content, "width": width, "height": height});
                        if let Ok(composed) =
                            orbit_host::authoring::compose(&raw, Some(layout.clone()))
                        {
                            slide.preview_key.set(Some(key));
                            let source = Source::Still {
                                key: format!("clipboard-import:{key:x}"),
                                slide: Arc::new(composed),
                                transparent: true,
                            };
                            slide.preview.update(cx, |preview, cx| {
                                preview.set_source(
                                    Some(source),
                                    Some(FrameSize { width, height }),
                                    0,
                                    cx,
                                )
                            });
                        }
                    }
                }
                thumb = thumb.child(div().absolute().inset_0().child(slide.preview.clone()));
            }
            grid = grid.child(
                div()
                    .id(("clipboard-preview", state.entity_id()))
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .border_1()
                    .border_color(if focused {
                        theme.colors.focus_ring
                    } else {
                        theme.colors.border_soft
                    })
                    .cursor_pointer()
                    .on_click(move |_, window, cx| handle.focus(window, cx))
                    .child(thumb)
                    .child(
                        div()
                            .px(m.space_xs)
                            .text_size(m.font_sm)
                            .bg(theme.colors.panel_bg_strong)
                            .child(number.to_string()),
                    ),
            );
        }
        let slider = |state, key| {
            div()
                .w(m.control_height * 4.)
                .child(RangeInput::new(state).label(t.t(key)))
        };
        div()
            .flex()
            .flex_col()
            .min_h_0()
            .gap(m.space_xs)
            .child(t.t("appMenu.reflowEditor"))
            .child(
                div()
                    .flex()
                    .h(window.viewport_size().height * 0.55)
                    .border_1()
                    .border_color(theme.colors.border)
                    .bg(theme.colors.surface_bg)
                    .child(list)
                    .child(grid),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(m.space_sm)
                    .child(slider(&editor.font, "cue.reflowTextSize"))
                    .child(
                        ActionButton::new("clipboard-split", t.t("cue.insertLineBreak"))
                            .compact()
                            .disabled(self.busy || editor.focused.is_none())
                            .on_click(
                                cx.listener(|this, _, window, cx| this.clipboard_split(window, cx)),
                            ),
                    )
                    .child(div().flex_1())
                    .child(slider(&editor.zoom, "cue.reflowPreviewSize")),
            )
            .into_any_element()
    }
}
