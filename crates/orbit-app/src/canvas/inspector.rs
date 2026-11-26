use super::{
    catalog::{self, Field as Spec, Kind, Target},
    content_document::DocumentKind,
    document::ThemeDocument,
    editing,
    geometry::{self, Gesture, Point},
    view::{History, SlideCanvas},
};
use gpui::{
    div, prelude::*, px, AnyElement, Context, Entity, Focusable, KeyDownEvent, MouseButton,
    SharedString, Subscription, Window,
};
use orbit_ui::controls::{
    input_state, list_row, ActionButton, ButtonVariant, CheckboxField, ColorPicker, Dialog, Field,
    IconButton, InputEvent, InputState, NumberInput, PanelHeader, Popover, Select, SelectOption,
    TabItem, TabList, TextInput,
};
use orbit_ui::{ActiveTheme, Translator};
use serde_json::{json, Value};
use std::{collections::HashMap, sync::Arc, time::Duration};

mod object;
mod visibility;

fn slide_shortcut_key(key: &gpui::Keystroke) -> Option<String> {
    let modifiers = key.modifiers;
    if modifiers.control || modifiers.alt || modifiers.platform || modifiers.function {
        return None;
    }
    if matches!(key.key.as_str(), "backspace" | "delete") {
        return Some(String::new());
    }
    (key.key.len() == 1 && key.key.as_bytes()[0].is_ascii_alphabetic())
        .then(|| key.key.to_ascii_lowercase())
}

#[derive(Default)]
pub struct Editor {
    pub base: Option<Arc<Value>>,
    pub edit_nonce: u64,
    pub pen_tool: bool,
    pub pen: Vec<Point>,
    pub pen_preview: Option<Point>,
    pub tab: String,
    pub(super) doc_tab: String,
    pub(super) document_preview: Option<Value>,
    pub(super) document_history: bool,
    pub(super) document_before_slide: Option<Arc<Value>>,
    pub(super) document_operations:
        std::collections::VecDeque<super::documents_ui::DocumentOperation>,
    pub(super) action_picker: String,
    pub(super) artwork_data: String,
    pub(super) artwork_image: Option<Result<Arc<gpui::Image>, String>>,
    pub(super) slide_transition: Option<Entity<crate::transitions::TransitionDefaults>>,
    pub(super) item_transition: Option<Entity<crate::transitions::TransitionDefaults>>,
    pub(super) prop_transition: Option<Entity<crate::transitions::TransitionDefaults>>,
    pub node_drag: Option<super::nodes::NodeDrag>,
    pub(super) scope: String,
    inputs: HashMap<String, Entity<InputState>>,
    ranges: HashMap<String, Entity<orbit_ui::controls::SliderState>>,
    open_sections: HashMap<String, bool>,
    textareas: HashMap<String, Entity<orbit_ui::controls::TextareaState>>,
    subscriptions: Vec<Subscription>,
    pub(super) dialog: Option<(String, Entity<InputState>)>,
    pub(super) favorites: HashMap<String, String>,
    paragraph: Option<(usize, usize)>,
    pub(super) note_paragraph: usize,
    pub(super) note_open: bool,
    pub(super) shape_menu: bool,
    pub(super) guides_hidden: bool,
    pub(super) text_closed: bool,
    pub(super) notes_closed: bool,
    pub(super) transitions: Value,
    effects: Value,
    ndi: Value,
    ndi_error: String,
    loaded: bool,
    fonts: Vec<String>,
    style_clipboard: Option<Value>,
    pub navigate: Option<(super::content_document::DocumentKind, String, String)>,
    pub close_after_save: bool,
    pub(super) after_slide_action: Option<String>,
    pub(super) pending_operation: Option<super::documents_ui::DocumentOperation>,
    media_target: String,
    media_probe: String,
    media_info: Value,
    previews: HashMap<String, Entity<crate::output::preview::Preview>>,
    pub(super) rows: super::sidebar::Rows,
}
impl Editor {
    pub fn reset(&mut self) {
        self.inputs.clear();
        self.ranges.clear();
        self.textareas.clear();
        self.subscriptions.clear();
        self.scope.clear();
        self.base = None;
        self.edit_nonce += 1;
        self.pen.clear();
        self.pen_tool = false;
        self.node_drag = None;
        self.paragraph = None;
        self.pending_operation = None;
        self.document_preview = None;
        self.document_history = false;
        self.document_before_slide = None;
        self.document_operations.clear();
        self.action_picker.clear();
        self.artwork_data.clear();
        self.artwork_image = None;
        self.after_slide_action = None;
        self.prop_transition = None;
        if self
            .dialog
            .as_ref()
            .is_some_and(|(action, _)| action == "prop-auto-clear")
        {
            self.dialog = None;
        }
        if self.tab.is_empty() {
            self.tab = "shape".into();
        }
        self.previews.clear();
        self.favorites.clear();
        self.rows.reset();
    }
}
impl SlideCanvas {
    pub(super) fn emit_close_if_requested(&mut self, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.editor.close_after_save) {
            cx.emit(super::view::CanvasEvent::Close);
        }
    }
    pub(super) fn draft(&mut self, slide: Value, cx: &mut Context<Self>) {
        if self.saving || self.document.is_none() {
            return;
        }
        if self.editor.base.is_none() {
            self.editor.base = Some(self.slide.clone());
            self.redo.clear();
        }
        self.set_slide(Arc::new(slide));
        self.editor.edit_nonce += 1;
        let nonce = self.editor.edit_nonce;
        let self_kind = self.kind;
        self.request_raster(false, cx);
        cx.notify();
        cx.spawn(async move |view, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(
                    if self_kind == super::content_document::DocumentKind::Mask {
                        350
                    } else {
                        700
                    },
                ))
                .await;
            view.update(cx, |view, cx| {
                if nonce == view.editor.edit_nonce
                    && view.editor.base.is_some()
                    && !view.saving
                    && view.editor.node_drag.is_none()
                    && matches!(view.gesture, Gesture::Idle)
                {
                    view.save((*view.slide).clone(), History::Edit, cx);
                }
            })
            .ok();
        })
        .detach();
    }
    fn patch_inspector(&mut self, spec: &Spec, value: Value, cx: &mut Context<Self>) {
        if self.ccli_field_read_only(spec) {
            return;
        }
        if self.patch_document_field(spec, &value, cx) {
            return;
        }
        if self.document.is_none() {
            return;
        }
        // Indexing a layer list past its end panics, so a selection that outlived its layer is refused here.
        let layers = self.slide["layers"].as_array().map_or(0, Vec::len);
        if self.selected.is_some_and(|index| index >= layers) {
            self.status = "The object no longer exists.".into();
            cx.notify();
            return;
        }
        let result = match spec.target {
            Target::Text if spec.key == "textRightIndentMode" => (|| {
                let index = self.selected.ok_or("Select an object.")?;
                let layer = self.slide["layers"].get(index).ok_or("Select an object.")?;
                let style = editing::style(&self.slide, Some(layer));
                let width = layer["bounds"]["width"]
                    .as_f64()
                    .filter(|width| *width > 0.)
                    .unwrap_or(self.slide["width"].as_f64().unwrap_or(1920.));
                let (field, edge) = if value == "absolute" {
                    let inset = style["textRightIndent"].as_f64().unwrap_or(0.);
                    ("textAbsoluteRightEdge", (width - inset).max(1.))
                } else {
                    let edge = style["textAbsoluteRightEdge"].as_f64().unwrap_or(width);
                    ("textRightIndent", (width - edge).clamp(0., 1000.))
                };
                // Setting one form removes the other (`apply_paragraph`).
                editing::patch_text(
                    &self.slide,
                    self.selected,
                    field,
                    editing::numeric(edge),
                    self.editor.paragraph,
                )
            })(),
            Target::Text => (|| {
                let mut slide = editing::patch_text(
                    &self.slide,
                    self.selected,
                    &spec.key,
                    value.clone(),
                    self.editor.paragraph,
                )?;
                if spec.key == "linkedTextTimerId" {
                    let name = if value == "" {
                        String::new()
                    } else {
                        self.runtime.content.read(cx).workspace["countdownTimers"]
                            .as_array()
                            .and_then(|timers| timers.iter().find(|timer| timer["id"] == value))
                            .and_then(|timer| timer["name"].as_str())
                            .ok_or("The selected timer is unavailable.")?
                            .to_string()
                    };
                    slide = editing::patch_text(
                        &slide,
                        self.selected,
                        "linkedTextTimerName",
                        json!(name),
                        None,
                    )?;
                }
                Ok(slide)
            })(),
            Target::Shape => self
                .selected
                .ok_or("Select an object.".to_string())
                .and_then(|index| editing::patch_shape(&self.slide, index, &spec.key, value)),
            Target::Build => self
                .selected
                .ok_or("Select an object.".to_string())
                .and_then(|index| {
                    let layer = &self.slide["layers"][index];
                    let mut next = if let Some((prefix, name)) = spec.key.split_once("Variable:") {
                        let mut next = layer.clone();
                        if !next[prefix]["transition"]["variables"].is_object() {
                            next[prefix]["transition"]["variables"] = json!({});
                        }
                        next[prefix]["transition"]["variables"][name] = value;
                        Ok(next)
                    } else {
                        super::builds::patch(layer, &self.slide, &spec.key, value)
                    }?;
                    if let Some(prefix) = spec.key.strip_suffix("Transition") {
                        if next[prefix]["transition"]["effectId"]
                            != layer[prefix]["transition"]["effectId"]
                        {
                            if let Some(direction) = self
                                .editor
                                .transitions
                                .as_array()
                                .and_then(|catalog| {
                                    catalog.iter().find(|effect| {
                                        effect["id"] == next[prefix]["transition"]["effectId"]
                                    })
                                })
                                .and_then(|effect| effect["defaultDirection"].as_str())
                            {
                                next[prefix]["transition"]["direction"] = json!(direction);
                            }
                        }
                    }
                    let mut slide = (*self.slide).clone();
                    slide["layers"][index] = next;
                    Ok(slide)
                }),
            Target::Bounds => self
                .selected
                .ok_or("Select an object.".to_string())
                .and_then(|index| {
                    let mut bounds =
                        geometry::layer_bounds(&self.slide["layers"][index], self.view.slide);
                    let n = editing::number(&value)? as f32;
                    match spec.key.as_str() {
                        "x" => bounds.x = n,
                        "y" => bounds.y = n,
                        "width" => bounds.width = n.max(8.),
                        "height" => bounds.height = n.max(8.),
                        _ => return Err("Invalid bounds field.".into()),
                    };
                    let mut slide = (*self.slide).clone();
                    slide["layers"][index] =
                        geometry::patch_bounds(&slide["layers"][index], self.view.slide, bounds);
                    Ok(slide)
                }),
            Target::Layer | Target::Slide => (|| {
                let mut slide = (*self.slide).clone();
                if spec.target == Target::Layer {
                    if let Some(index) = self.selected {
                        if spec.key == "ndiSource" {
                            let source: Value = serde_json::from_str(
                                value.as_str().ok_or("Choose an NDI source.")?,
                            )
                            .map_err(|_| "Invalid NDI source.")?;
                            if ["name", "urlAddress"].iter().any(|key| {
                                source[*key]
                                    .as_str()
                                    .is_none_or(|s| s.trim().is_empty() || s.contains('\0'))
                            }) {
                                return Err("An NDI source name and address are required.".into());
                            }
                            slide["layers"][index]["ndiSource"] = source;
                            slide["layers"][index]["fillType"] = json!("ndi");
                        } else if spec.key == "text"
                            && self.kind == super::content_document::DocumentKind::Slide
                            && slide["layers"][index]["id"] == slide["primaryTextLayerId"]
                        {
                            slide = orbit_host::authoring::with_content(
                                &slide,
                                value.as_str().ok_or("Slide text must be text.")?,
                            )?;
                        } else if spec.key == "text"
                            && self.kind == super::content_document::DocumentKind::Theme
                            && editing::is_text(&slide["layers"][index])
                        {
                            slide["text"] = value;
                        } else {
                            slide["layers"][index][&spec.key] = value;
                        }
                    }
                } else {
                    if spec.key.starts_with("slideTransition")
                        || spec.key.starts_with("slideVariable:")
                    {
                        if spec.key == "slideTransition" && value == "" {
                            slide.as_object_mut().unwrap().remove("transition");
                            return Ok(slide);
                        }
                        let layer = json!({"id":"transition","type":"Shape","build":{"step":1,"start":"on-click","delaySeconds":0,"transition":slide["transition"]}});
                        let field = spec.key.replacen("slide", "build", 1);
                        let next = if let Some(name) = field.strip_prefix("buildVariable:") {
                            let mut next = layer.clone();
                            if !next["build"]["transition"]["variables"].is_object() {
                                next["build"]["transition"]["variables"] = json!({});
                            }
                            next["build"]["transition"]["variables"][name] = value;
                            next
                        } else {
                            super::builds::patch(&layer, &slide, &field, value)?
                        };
                        slide["transition"] = next["build"]["transition"].clone();
                        return Ok(slide);
                    }
                    match spec.key.as_str() {
                        "slideName" => slide["name"] = value,
                        "size" => {
                            if let Some((w, h)) = value.as_str().and_then(|s| s.split_once('x')) {
                                let size = |n: &str| {
                                    n.parse::<u32>()
                                        .map_err(|_| format!("'{n}' is not a slide dimension."))
                                };
                                slide["width"] = json!(size(w)?);
                                slide["height"] = json!(size(h)?);
                            }
                        }
                        "labelColor" => {
                            let color = value.as_str().ok_or("Label color must be text.")?;
                            if !color.is_empty()
                                && !(matches!(color.len(), 7 | 9)
                                    && color.starts_with('#')
                                    && color[1..].bytes().all(|c| c.is_ascii_hexdigit()))
                            {
                                return Err("Label color must be #RRGGBB or #RRGGBBAA.".into());
                            }
                            if color.is_empty() {
                                slide.as_object_mut().unwrap().remove("labelColor");
                            } else {
                                slide["labelColor"] = value;
                            }
                        }
                        "text" if self.kind == super::content_document::DocumentKind::Slide => {
                            slide = orbit_host::authoring::with_content(
                                &slide,
                                value.as_str().ok_or("Slide text must be text.")?,
                            )?
                        }
                        key => slide[key] = value,
                    }
                }
                Ok(slide)
            })(),
            Target::Document => Err("Unsupported document field.".into()),
            Target::Note => (|| {
                let key = spec.key.strip_prefix("note:").unwrap_or(&spec.key);
                let paragraph = self.editor.note_paragraph;
                if key == "textRightIndentMode" {
                    // The notes have no box of their own; the renderer's ruler is 400 wide.
                    const WIDTH: f64 = 400.;
                    let style = &self.slide["notesTextStyle"]["textParagraphStyles"][paragraph];
                    let (field, edge) = if value == "absolute" {
                        let inset = style["textRightIndent"].as_f64().unwrap_or(0.);
                        ("textAbsoluteRightEdge", (WIDTH - inset).max(1.))
                    } else {
                        let edge = style["textAbsoluteRightEdge"].as_f64().unwrap_or(WIDTH);
                        ("textRightIndent", (WIDTH - edge).clamp(0., 1000.))
                    };
                    return editing::patch_notes_style(
                        &self.slide,
                        field,
                        editing::numeric(edge),
                        paragraph,
                    );
                }
                editing::patch_notes_style(&self.slide, key, value, paragraph)
            })(),
            Target::Object => self.patch_object(spec, value, cx),
            Target::Tab | Target::Timer | Target::Trigger | Target::Path | Target::Effect => {
                self.patch_collection(spec, value)
            }
        };
        match result {
            Ok(slide) => {
                if self.kind == super::content_document::DocumentKind::Slide
                    && spec.key == "text"
                    && (spec.target == Target::Slide
                        || self.selected.is_some_and(|index| {
                            self.slide["layers"][index]["id"] == self.slide["primaryTextLayerId"]
                        }))
                {
                    self.rich = None;
                }
                if self.document_tabs() && spec.target == Target::Slide {
                    self.save_inspector_slide(slide, cx);
                } else {
                    self.draft(slide, cx);
                }
            }
            Err(error) => {
                self.status = error;
                cx.notify();
            }
        }
    }
    /// Object-level fields: conditional visibility and path closing. The renderer's reference values (a timer, an
    /// object, a video input) are built here from the selected text of the choices.
    fn patch_object(&self, spec: &Spec, value: Value, cx: &Context<Self>) -> Result<Value, String> {
        let index = self.selected.ok_or("Select an object.")?;
        let layer = self.slide["layers"]
            .get(index)
            .ok_or("The object no longer exists.")?;
        let workspace = &self.runtime.content.read(cx).workspace;
        let timer = |id: &str| -> Result<Value, String> {
            let timer = workspace["countdownTimers"]
                .as_array()
                .and_then(|timers| timers.iter().find(|timer| timer["id"] == id))
                .ok_or("Visibility condition timer was not found.")?;
            Ok(json!({"timerId": timer["id"], "timerName": timer["name"]}))
        };
        let object = |id: &str| -> Result<Value, String> {
            let object = self.slide["layers"]
                .as_array()
                .and_then(|layers| layers.iter().find(|layer| layer["id"] == id))
                .ok_or("Visibility condition object was not found.")?;
            Ok(json!({
                "objectId": object["id"],
                "objectName": object["name"].as_str().unwrap_or("")
            }))
        };
        let input = |id: &str| -> Result<Value, String> {
            let position = workspace["videoInputs"]
                .as_array()
                .and_then(|inputs| inputs.iter().position(|input| input["id"] == id))
                .ok_or("Visibility condition video input was not found.")?;
            Ok(json!({"inputIndex": position, "inputId": id}))
        };
        let text = value.as_str().unwrap_or("");
        let value = match spec.key.split(':').next().unwrap_or("") {
            "visibilityTimer" => timer(text)?,
            "visibilityObject" => object(text)?,
            "visibilityInput" => input(text)?,
            "visibilityKind" => match text {
                "timer" => {
                    let first = workspace["countdownTimers"][0]["id"]
                        .as_str()
                        .ok_or("Visibility condition timer was not found.")?;
                    let mut fields = timer(first)?;
                    fields["kind"] = json!("timer");
                    fields
                }
                "element" => {
                    let other = self.slide["layers"]
                        .as_array()
                        .and_then(|layers| {
                            layers.iter().enumerate().find(|(position, layer)| {
                                Some(*position) != self.selected && layer["id"].is_string()
                            })
                        })
                        .and_then(|(_, layer)| layer["id"].as_str())
                        .ok_or("Visibility condition object was not found.")?;
                    let mut fields = object(other)?;
                    fields["kind"] = json!("element");
                    fields
                }
                "video-input" => {
                    let first = workspace["videoInputs"][0]["id"]
                        .as_str()
                        .ok_or("Visibility condition video input was not found.")?;
                    let mut fields = input(first)?;
                    fields["kind"] = json!("video-input");
                    fields
                }
                kind => json!({"kind": kind}),
            },
            "visibilityAdd" => match workspace["countdownTimers"][0]["id"].as_str() {
                Some(id) => timer(id)?,
                None => json!({"kind": "capture"}),
            },
            _ => value,
        };
        let patched = super::object_fields::patch(layer, &spec.key, &value)?
            .ok_or("Unknown object field.")?;
        let mut slide = (*self.slide).clone();
        slide["layers"][index] = patched;
        Ok(slide)
    }
    fn patch_collection(&self, spec: &Spec, value: Value) -> Result<Value, String> {
        let index = self.selected.ok_or("Select an object.")?;
        let mut slide = (*self.slide).clone();
        let style = editing::style(&slide, Some(&slide["layers"][index]));
        if spec.target == Target::Path {
            let pieces = spec.key.split(':').collect::<Vec<_>>();
            let point = pieces[1]
                .parse::<usize>()
                .map_err(|_| "Invalid path point.")?;
            let target = slide["layers"][index]["pathPoints"]
                .as_array_mut()
                .and_then(|points| points.get_mut(point))
                .ok_or("The path point no longer exists.")?;
            if pieces[2] == "x" || pieces[2] == "y" {
                target[pieces[2]] = value;
            } else if pieces.len() == 4 {
                target[pieces[2]][pieces[3]] = value;
            } else if value == true {
                target[pieces[2]] = json!({"x":target["x"],"y":target["y"]});
            } else {
                target
                    .as_object_mut()
                    .ok_or("Invalid path point.")?
                    .remove(pieces[2]);
            }
            return Ok(slide);
        }
        if spec.target == Target::Effect {
            let text = self.editor.tab == "text";
            let key = if text {
                "mediaFillEffects"
            } else {
                "mediaEffects"
            };
            let mut chain = if text {
                style[key].as_array()
            } else {
                slide["layers"][index][key].as_array()
            }
            .cloned()
            .unwrap_or_default();
            let pieces = spec.key.split(':').collect::<Vec<_>>();
            let row = pieces[1]
                .parse::<usize>()
                .map_err(|_| "Invalid effect position.")?;
            let effect = chain.get_mut(row).ok_or("The effect no longer exists.")?;
            if pieces[2] == "enabled" {
                effect["enabled"] = value;
            } else {
                if !effect["variables"].is_object() {
                    effect["variables"] = json!({});
                }
                effect["variables"][pieces[2]] = value;
            }
            if text {
                slide["layers"][index]["textStyle"] = style;
                slide["layers"][index]["textStyle"][key] = json!(chain);
            } else {
                slide["layers"][index][key] = json!(chain);
            }
            return Ok(slide);
        }
        let (field, values) = match spec.target {
            Target::Tab if spec.key.starts_with("bar:") => {
                let mut bars = style["textBarTabStops"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                let row = spec
                    .key
                    .split(':')
                    .nth(1)
                    .and_then(|row| row.parse::<usize>().ok())
                    .ok_or("Invalid bar tab.")?;
                *bars.get_mut(row).ok_or("The bar tab no longer exists.")? = value;
                if bars.len() > 32
                    || bars.iter().enumerate().any(|(i, bar)| {
                        bar.as_f64().is_none_or(|n| {
                            !(0. ..=2000.).contains(&n)
                                || i > 0 && bars[i - 1].as_f64().is_some_and(|p| p >= n)
                        })
                    })
                {
                    return Err("Tab positions must increase and remain between 0 and 2000.".into());
                }
                ("textBarTabStops", json!(bars))
            }
            Target::Tab => {
                let mut stops = style["textTabStops"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                let pieces = spec.key.split(':').collect::<Vec<_>>();
                let row = pieces[1]
                    .parse::<usize>()
                    .map_err(|_| "Invalid tab stop.")?;
                let stop = stops.get_mut(row).ok_or("The tab stop no longer exists.")?;
                if pieces[2] == "leader" && value == "" {
                    stop.as_object_mut()
                        .ok_or("Invalid tab stop.")?
                        .remove("leader");
                } else if pieces[2] == "leader"
                    && !matches!(
                        value.as_str(),
                        Some("dot" | "equal" | "hyphen" | "thick" | "underline")
                    )
                {
                    return Err("Invalid tab leader.".into());
                } else {
                    stop[pieces[2]] = value;
                }
                if stops.len() > 32
                    || stops.iter().enumerate().any(|(i, s)| {
                        s["position"].as_f64().is_none_or(|n| {
                            !(0. ..=2000.).contains(&n)
                                || i > 0
                                    && stops[i - 1]["position"].as_f64().is_some_and(|p| p >= n)
                        })
                    })
                {
                    return Err("Tab positions must increase and remain between 0 and 2000.".into());
                }
                ("textTabStops", json!(stops))
            }
            Target::Timer => {
                let mut format=style.get("linkedTextTimerFormat").cloned().filter(Value::is_object).unwrap_or(json!({"hour":"remove_short","minute":"short","second":"long","millisecond":"none"}));
                format[spec
                    .key
                    .strip_prefix("timer:")
                    .ok_or("Invalid timer format field.")?] = value;
                ("linkedTextTimerFormat", format)
            }
            Target::Trigger => {
                let mut triggers = style["linkedTextTimerColorTriggers"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                let pieces = spec.key.split(':').collect::<Vec<_>>();
                let row = pieces[1]
                    .parse::<usize>()
                    .map_err(|_| "Invalid timer color trigger.")?;
                triggers
                    .get_mut(row)
                    .ok_or("The timer color trigger no longer exists.")?[pieces[2]] = value;
                ("linkedTextTimerColorTriggers", json!(triggers))
            }
            _ => return Err("Invalid collection field.".into()),
        };
        editing::patch_text(&slide, Some(index), field, values, self.editor.paragraph)
    }
    fn editor_collections(
        &mut self,
        section: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = *cx.global::<Translator>();
        let theme = *cx.theme();
        let mut body = div().flex().flex_col().gap(theme.metrics.space_xs);
        let Some(index) = self.selected else {
            return body.into_any_element();
        };
        let style = editing::style(&self.slide, Some(&self.slide["layers"][index]));
        if self.editor.tab == "text" && section == "paragraph" {
            let count = editing::paragraphs(
                self.slide["layers"][index]["text"]
                    .as_str()
                    .or_else(|| self.slide["content"].as_str())
                    .or_else(|| self.slide["text"].as_str())
                    .unwrap_or(""),
            )
            .len();
            let mut options = vec![SelectOption::new(
                "all".to_string(),
                t.t("theme.text.paragraphScopeAll"),
            )];
            options.extend((0..count).map(|i| {
                SelectOption::new(
                    i.to_string(),
                    format!("{} {}", t.t("theme.text.paragraph"), i + 1),
                )
            }));
            let weak = cx.weak_entity();
            body = body.child(
                Select::new(
                    "theme-paragraph-scope",
                    options,
                    self.editor
                        .paragraph
                        .map(|(start, _)| start.to_string())
                        .unwrap_or_else(|| "all".to_string()),
                )
                .label(t.t("theme.text.paragraph"))
                .on_change(move |value, _, cx| {
                    weak.update(cx, |view, cx| {
                        view.editor.paragraph =
                            value.parse::<usize>().ok().map(|index| (index, index));
                        view.editor.scope.clear();
                        cx.notify();
                    })
                    .ok();
                }),
            );
            if let Some((start, end)) = self.editor.paragraph {
                let weak = cx.weak_entity();
                body = body.child(
                    Select::new(
                        "theme-paragraph-end",
                        (start..count)
                            .map(|i| SelectOption::new(i.to_string(), (i + 1).to_string()))
                            .collect(),
                        end.to_string(),
                    )
                    .label(t.t("theme.gpui.paragraphEnd"))
                    .on_change(move |value, _, cx| {
                        weak.update(cx, |view, cx| {
                            if let Ok(end) = value.parse::<usize>() {
                                view.editor.paragraph = Some((start, end));
                                view.editor.scope.clear();
                                cx.notify();
                            }
                        })
                        .ok();
                    }),
                );
            }
            let stops = style["textTabStops"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            body = body.child(PanelHeader::new(t.t("theme.text.tabStops")));
            for (row, stop) in stops.iter().enumerate() {
                for (key, kind, label) in [
                    (
                        "position",
                        Kind::Number(0., 2000.),
                        "theme.text.tabPosition",
                    ),
                    (
                        "alignment",
                        catalog::choices(&["left", "right", "center", "decimal", "justify"]),
                        "theme.text.tabAlignment",
                    ),
                    (
                        "leader",
                        catalog::labeled(&[
                            ("", "theme.text.tabLeaderNone"),
                            ("dot", "theme.text.tabLeader.dot"),
                            ("equal", "theme.text.tabLeader.equal"),
                            ("hyphen", "theme.text.tabLeader.hyphen"),
                            ("thick", "theme.text.tabLeader.thick"),
                            ("underline", "theme.text.tabLeader.underline"),
                        ]),
                        "theme.text.tabLeader",
                    ),
                ] {
                    body = body.child(
                        self.inspector_field(
                            Spec::new(
                                &format!("tab:{row}:{key}"),
                                Target::Tab,
                                kind,
                                if key == "leader" {
                                    json!(stop[key].as_str().unwrap_or(""))
                                } else {
                                    stop[key].clone()
                                },
                            )
                            .named(label),
                            window,
                            cx,
                        ),
                    );
                }
                let weak = cx.weak_entity();
                body = body.child(
                    IconButton::new(
                        SharedString::from(format!("remove-tab-{row}")),
                        "x",
                        t.t("theme.text.tabRemove"),
                    )
                    .on_click(move |_, _, cx| {
                        weak.update(cx, |view, cx| {
                            view.collection_action("tab", row, "remove", cx)
                        })
                        .ok();
                    }),
                );
            }
            let weak = cx.weak_entity();
            body = body.child(
                ActionButton::new("add-theme-tab", t.t("theme.text.tabAdd"))
                    .disabled(stops.len() >= 32)
                    .on_click(move |_, _, cx| {
                        weak.update(cx, |view, cx| view.collection_action("tab", 0, "add", cx))
                            .ok();
                    }),
            );
            body = body.child(PanelHeader::new(t.t("theme.text.barTabs")));
            let bars = style["textBarTabStops"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            for (row, bar) in bars.iter().enumerate() {
                body = body.child(
                    self.inspector_field(
                        Spec::new(
                            &format!("bar:{row}:position"),
                            Target::Tab,
                            Kind::Number(0., 2000.),
                            bar.clone(),
                        )
                        .named("theme.text.barTabPosition"),
                        window,
                        cx,
                    ),
                );
                let weak = cx.weak_entity();
                body = body.child(
                    IconButton::new(
                        SharedString::from(format!("remove-bar-tab-{row}")),
                        "x",
                        t.t("theme.text.tabRemove"),
                    )
                    .on_click(move |_, _, cx| {
                        weak.update(cx, |view, cx| {
                            view.collection_action("bar", row, "remove", cx)
                        })
                        .ok();
                    }),
                );
            }
            let weak = cx.weak_entity();
            body = body.child(
                ActionButton::new("add-bar-tab", t.t("theme.text.barTabAdd"))
                    .disabled(bars.len() >= 32)
                    .on_click(move |_, _, cx| {
                        weak.update(cx, |view, cx| view.collection_action("bar", 0, "add", cx))
                            .ok();
                    }),
            );
        }
        if self.editor.tab == "text" && section == "linked" {
            if matches!(
                style["linkedTextSource"].as_str(),
                Some("Countdown Timer" | "Video Countdown" | "Slide Timer")
            ) {
                let format=style.get("linkedTextTimerFormat").cloned().filter(Value::is_object).unwrap_or(json!({"hour":"remove_short","minute":"short","second":"long","millisecond":"none"}));
                for unit in ["hour", "minute", "second", "millisecond"] {
                    let options = if unit == "millisecond" {
                        catalog::choices(&["none", "short", "long"])
                    } else {
                        catalog::choices(&["none", "short", "long", "remove_short", "remove_long"])
                    };
                    body = body.child(
                        self.inspector_field(
                            Spec::new(
                                &format!("timer:{unit}"),
                                Target::Timer,
                                options,
                                format[unit].clone(),
                            )
                            .named(&format!("theme.linkedText.timerFormat.{unit}")),
                            window,
                            cx,
                        ),
                    );
                }
                for flag in ["wallClock", "use24Hour", "millisecondsUnderMinuteOnly"] {
                    body = body.child(
                        self.inspector_field(
                            Spec::new(
                                &format!("timer:{flag}"),
                                Target::Timer,
                                Kind::Bool,
                                json!(format[flag] == true),
                            )
                            .named(&format!("theme.linkedText.timerFormat.{flag}")),
                            window,
                            cx,
                        ),
                    );
                }
                for (row, trigger) in style["linkedTextTimerColorTriggers"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .enumerate()
                {
                    body = body.child(
                        self.inspector_field(
                            Spec::new(
                                &format!("trigger:{row}:time"),
                                Target::Trigger,
                                Kind::Number(-100000., 100000.),
                                trigger["time"].clone(),
                            )
                            .named("theme.gpui.timerTriggerTime"),
                            window,
                            cx,
                        ),
                    );
                    body = body.child(
                        self.inspector_field(
                            Spec::new(
                                &format!("trigger:{row}:color"),
                                Target::Trigger,
                                Kind::Color,
                                trigger["color"].clone(),
                            )
                            .named("theme.gpui.textColor"),
                            window,
                            cx,
                        ),
                    );
                    let weak = cx.weak_entity();
                    body = body.child(
                        IconButton::new(
                            SharedString::from(format!("remove-trigger-{row}")),
                            "x",
                            t.t("common.delete"),
                        )
                        .on_click(move |_, _, cx| {
                            weak.update(cx, |view, cx| {
                                view.collection_action("trigger", row, "remove", cx)
                            })
                            .ok();
                        }),
                    );
                }
                let weak = cx.weak_entity();
                body = body.child(
                    ActionButton::new("add-timer-trigger", t.t("common.add")).on_click(
                        move |_, _, cx| {
                            weak.update(cx, |view, cx| {
                                view.collection_action("trigger", 0, "add", cx)
                            })
                            .ok();
                        },
                    ),
                );
            }
            for (action, key) in [
                ("file", "theme.linkedText.file"),
                ("rss", "theme.linkedText.rss"),
            ] {
                if style["linkedTextSource"] != if action == "file" { "File" } else { "RSS" } {
                    continue;
                }
                let weak = cx.weak_entity();
                body = body.child(
                    ActionButton::new(SharedString::from(format!("theme-load-{action}")), t.t(key))
                        .on_click(move |_, window, cx| {
                            weak.update(cx, |view, cx| view.editor_action(action, window, cx))
                                .ok();
                        }),
                );
            }
        }
        if section == "media"
            && ((self.editor.tab == "shape" && self.slide["layers"][index]["fillType"] == "media")
                || (self.editor.tab == "text" && style["textFillMode"] == "Media Fill"))
        {
            let preview = self.media_preview_for_selection(index, cx);
            body = body.child(div().h(px(150.)).child(preview));
            let text = self.editor.tab == "text";
            let owner = if text {
                &style
            } else {
                &self.slide["layers"][index]
            };
            let source = owner[if text {
                "mediaFillSourcePath"
            } else {
                "sourcePath"
            }]
            .as_str()
            .unwrap_or("")
            .to_string();
            if source != self.editor.media_probe {
                self.editor.media_probe = source.clone();
                self.editor.media_info = Value::Null;
                if !source.is_empty() {
                    let host = self.runtime.host.clone();
                    let requested = source.clone();
                    cx.spawn(async move |view, cx| {
                        let result = cx
                            .background_spawn(async move {
                                host.invoke("probeMediaMetadata", vec![json!(requested)])
                            })
                            .await;
                        view.update(cx, |view, cx| {
                            if view.editor.media_probe == source {
                                view.editor.media_info = match result {
                                    Ok(info) => info,
                                    Err(error) => json!({"error":error}),
                                };
                                cx.notify();
                            }
                        })
                        .ok();
                    })
                    .detach();
                }
            }
            body = body.child(orbit_ui::controls::OutputField::new(
                t.t("theme.gpui.sourcePath"),
                self.editor.media_probe.clone(),
            ));
            for key in [
                "codec",
                "width",
                "height",
                "durationSeconds",
                "fps",
                "error",
            ] {
                if let Some(value) = self
                    .editor
                    .media_info
                    .get(key)
                    .filter(|value| !value.is_null())
                {
                    body = body.child(orbit_ui::controls::OutputField::new(
                        t.t(&format!("theme.gpui.{key}")),
                        value
                            .as_str()
                            .map(str::to_string)
                            .unwrap_or_else(|| value.to_string()),
                    ));
                }
            }
            let key = if text {
                "mediaFillEffects"
            } else {
                "mediaEffects"
            };
            let chain = owner[key].as_array().cloned().unwrap_or_default();
            body = body.child(PanelHeader::new(t.t("media.effectChain.title")));
            for (row, effect) in chain.iter().enumerate() {
                let entry = self
                    .editor
                    .effects
                    .as_array()
                    .and_then(|effects| {
                        effects
                            .iter()
                            .find(|entry| entry["id"] == effect["effectId"])
                    })
                    .cloned();
                body = body.child(
                    self.inspector_field(
                        Spec::new(
                            &format!("effect:{row}:enabled"),
                            Target::Effect,
                            Kind::Bool,
                            effect["enabled"].clone(),
                        )
                        .named(
                            entry
                                .as_ref()
                                .and_then(|entry| entry["name"].as_str())
                                .unwrap_or("media.effectChain.title"),
                        ),
                        window,
                        cx,
                    ),
                );
                if let Some(vars) = entry
                    .as_ref()
                    .and_then(|entry| entry["variables"].as_array())
                {
                    for var in vars {
                        let Some(name) = var["name"].as_str() else {
                            continue;
                        };
                        let kind = if var["type"] == "color" {
                            Kind::Color
                        } else {
                            Kind::Number(
                                var["min"].as_f64().unwrap_or(0.),
                                var["max"].as_f64().unwrap_or(100.),
                            )
                        };
                        let value = effect["variables"]
                            .get(name)
                            .cloned()
                            .unwrap_or(var["value"].clone());
                        body = body.child(
                            self.inspector_field(
                                Spec::new(
                                    &format!("effect:{row}:{name}"),
                                    Target::Effect,
                                    kind,
                                    value,
                                )
                                .named(var["label"].as_str().unwrap_or(name)),
                                window,
                                cx,
                            ),
                        );
                    }
                }
                for (action, icon, label) in [
                    ("up", "arrow-up", "arrangements.moveUp"),
                    ("down", "arrow-down", "arrangements.moveDown"),
                    ("remove", "x", "common.delete"),
                ] {
                    let weak = cx.weak_entity();
                    body = body.child(
                        IconButton::new(
                            SharedString::from(format!("effect-{row}-{action}")),
                            icon,
                            t.t(label),
                        )
                        .on_click(move |_, _, cx| {
                            weak.update(cx, |view, cx| {
                                view.collection_action("effect", row, action, cx)
                            })
                            .ok();
                        }),
                    );
                }
            }
            let entries = self
                .editor
                .effects
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|entry| {
                    Some(SelectOption::new(
                        entry["id"].as_str()?.to_string(),
                        entry["name"].as_str()?.to_string(),
                    ))
                })
                .collect();
            let weak = cx.weak_entity();
            body = body.child(
                Select::new("add-media-effect", entries, String::new())
                    .label(t.t("common.add"))
                    .on_change(move |id, _, cx| {
                        weak.update(cx, |view, cx| view.add_effect(&id, cx)).ok();
                    }),
            );
            let weak = cx.weak_entity();
            body = body.child(
                ActionButton::new("choose-theme-media", t.t("common.open")).on_click(
                    move |_, window, cx| {
                        weak.update(cx, |view, cx| view.editor_action("media", window, cx))
                            .ok();
                    },
                ),
            );
        }
        if self.editor.tab == "shape" && section == "path" {
            if self.slide["layers"][index]["pathPoints"]
                .as_array()
                .is_some_and(|points| !points.is_empty())
            {
                body = body.child(
                    self.inspector_field(
                        Spec::new(
                            "pathClosed",
                            Target::Object,
                            Kind::Bool,
                            json!(self.slide["layers"][index]["closedPath"] == true),
                        )
                        .named("theme.path.closed"),
                        window,
                        cx,
                    ),
                );
            }
            let points = self.slide["layers"][index]["pathPoints"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            for (row, point) in points.iter().enumerate() {
                for axis in ["x", "y"] {
                    body = body.child(
                        self.inspector_field(
                            Spec::new(
                                &format!("point:{row}:{axis}"),
                                Target::Path,
                                Kind::Number(-100000., 100000.),
                                point[axis].clone(),
                            )
                            .named(if axis == "x" {
                                "theme.gpui.x"
                            } else {
                                "theme.gpui.y"
                            }),
                            window,
                            cx,
                        ),
                    );
                }
                for control in ["q0", "q1"] {
                    body = body.child(
                        self.inspector_field(
                            Spec::new(
                                &format!("point:{row}:{control}"),
                                Target::Path,
                                Kind::Bool,
                                json!(point[control].is_object()),
                            )
                            .named("theme.gpui.pathControl"),
                            window,
                            cx,
                        ),
                    );
                    if point[control].is_object() {
                        for axis in ["x", "y"] {
                            body = body.child(
                                self.inspector_field(
                                    Spec::new(
                                        &format!("point:{row}:{control}:{axis}"),
                                        Target::Path,
                                        Kind::Number(-100000., 100000.),
                                        point[control][axis].clone(),
                                    )
                                    .named(if axis == "x" {
                                        "theme.gpui.x"
                                    } else {
                                        "theme.gpui.y"
                                    }),
                                    window,
                                    cx,
                                ),
                            );
                        }
                    }
                }
            }
            for (action, key) in [
                ("rename-object", "common.rename"),
                ("delete-object", "common.delete"),
            ] {
                let weak = cx.weak_entity();
                body = body.child(
                    ActionButton::new(SharedString::from(action), t.t(key)).on_click(
                        move |_, window, cx| {
                            weak.update(cx, |view, cx| view.editor_action(action, window, cx))
                                .ok();
                        },
                    ),
                );
            }
        }
        if section == "media"
            && self.editor.tab == "shape"
            && self.slide["layers"][index]["fillType"] == "video-input"
        {
            let options = self.runtime.content.read(cx).workspace["videoInputs"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|input| {
                    Some((
                        input["id"].as_str()?.to_string(),
                        input["title"].as_str()?.to_string(),
                    ))
                })
                .collect();
            let current = self.slide["layers"][index]["videoInputId"].clone();
            body = body.child(
                self.inspector_field(
                    Spec::new(
                        "videoInputId",
                        Target::Layer,
                        Kind::Choice(options),
                        if current.is_string() {
                            current
                        } else {
                            json!("")
                        },
                    )
                    .named("theme.fillType.videoInput"),
                    window,
                    cx,
                ),
            );
        }
        if section == "media"
            && self.editor.tab == "shape"
            && self.slide["layers"][index]["fillType"] == "ndi"
        {
            if !self.editor.ndi_error.is_empty() {
                body = body.child(
                    div()
                        .text_color(theme.colors.warning)
                        .child(self.editor.ndi_error.clone()),
                );
            }
            let source = self.slide["layers"][index]["ndiSource"].clone();
            let mut options = self
                .editor
                .ndi
                .as_array()
                .into_iter()
                .flatten()
                .cloned()
                .collect::<Vec<_>>();
            if source.is_object() && !options.contains(&source) {
                options.insert(0, source.clone());
            }
            let options = options
                .into_iter()
                .filter_map(|source| {
                    Some((
                        serde_json::to_string(&source).ok()?,
                        source["name"].as_str()?.to_string(),
                    ))
                })
                .collect();
            body = body.child(
                self.inspector_field(
                    Spec::new(
                        "ndiSource",
                        Target::Layer,
                        Kind::Choice(options),
                        json!(if source.is_object() {
                            source.to_string()
                        } else {
                            String::new()
                        }),
                    )
                    .named("ndi.source"),
                    window,
                    cx,
                ),
            );
            let weak = cx.weak_entity();
            body = body.child(
                ActionButton::new("refresh-ndi-sources", t.t("ndi.refresh")).on_click(
                    move |_, _, cx| {
                        weak.update(cx, |view, cx| {
                            view.editor.loaded = false;
                            cx.notify();
                        })
                        .ok();
                    },
                ),
            );
        }
        if self.editor.tab == "build" && section == "build" {
            body = body.child(self.transition_favorites(cx));
        }
        body.into_any_element()
    }
    fn media_preview_for_selection(
        &mut self,
        index: usize,
        cx: &mut Context<Self>,
    ) -> Entity<crate::output::preview::Preview> {
        let mut layer = self.slide["layers"][index].clone();
        let bounds = geometry::layer_bounds(&layer, self.view.slide);
        if self.editor.tab == "text" {
            let style = editing::style(&self.slide, Some(&layer));
            layer = json!({"id":"glyph-media-preview","type":"Shape","shape":"rectangle","fillType":"media","fill":"Media","sourcePath":style["mediaFillSourcePath"]});
            for suffix in [
                "Id",
                "Url",
                "Type",
                "Scale",
                "FlipX",
                "FlipY",
                "Crop",
                "CropTop",
                "CropRight",
                "CropBottom",
                "CropLeft",
                "Rotation",
                "Playback",
                "Volume",
                "RampIn",
                "RampOut",
                "Hue",
                "Saturation",
                "Brightness",
                "Contrast",
                "Effects",
            ] {
                if let Some(value) = style.get(format!("mediaFill{suffix}")) {
                    layer[format!("media{suffix}")] = value.clone();
                }
            }
        }
        layer["bounds"] = json!({"x":0,"y":0,"width":bounds.width,"height":bounds.height});
        layer["hidden"] = json!(false);
        let slide = json!({"id":"media-inspector","width":bounds.width,"height":bounds.height,"ignoreBuilds":true,"layers":[layer]});
        self.still_preview(
            "selected-media",
            slide,
            format!(
                "{}:{}:{index}:{}",
                self.workspace, self.slide_id, self.editor.edit_nonce
            ),
            cx,
        )
    }
    fn still_preview(
        &mut self,
        id: &str,
        slide: Value,
        key: String,
        cx: &mut Context<Self>,
    ) -> Entity<crate::output::preview::Preview> {
        let host = self.runtime.host.clone();
        let frames = self.runtime.frames.clone();
        let preview = self
            .editor
            .previews
            .entry(id.to_string())
            .or_insert_with(|| cx.new(|cx| crate::output::preview::Preview::new(host, frames, cx)))
            .clone();
        let size = geometry::slide_size(&slide);
        preview.update(cx, |preview, cx| {
            preview.set_source(
                Some(crate::output::preview::Source::Still {
                    key: format!("theme-still:{key}"),
                    slide: Arc::new(slide),
                    transparent: false,
                }),
                Some(orbit_ui::FrameSize {
                    width: size.x.ceil() as u32,
                    height: size.y.ceil() as u32,
                }),
                0,
                cx,
            )
        });
        preview
    }
    fn media_picker_rows(
        &mut self,
        range: std::ops::Range<usize>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let workspace = self.runtime.content.read(cx).workspace.clone();
        let Some(media) = workspace["media"].as_array() else {
            return Vec::new();
        };
        let keep = range
            .clone()
            .map(|index| format!("picker-{index}"))
            .collect::<Vec<_>>();
        self.editor
            .previews
            .retain(|key, _| key == "selected-media" || keep.contains(key));
        range.filter_map(|index|{let media=media.get(index)?.clone();let title=media["title"].as_str().or_else(||media["name"].as_str()).or_else(||media["id"].as_str())?.to_string();let id=format!("picker-{index}");
            let slide=json!({"width":1920,"height":1080,"ignoreBuilds":true,"layers":[{"id":"image","type":"Shape","shape":"rectangle","fillType":"media","fill":"Media","mediaType":media["type"],"mediaUrl":media["sourceUrl"],"sourcePath":media["sourcePath"],"mediaScale":"Scale to Fit","bounds":{"x":0,"y":0,"width":1920,"height":1080}}]});
            let preview=self.still_preview(&id,slide,format!("{}:{index}:{}",self.workspace,media["sourceUrl"]),cx);let weak=cx.weak_entity();
            Some(div().id(SharedString::from(id)).flex().items_center().gap(cx.theme().metrics.space_xs).h(px(80.)).child(div().w(px(120.)).h(px(68.)).child(preview)).child(ActionButton::new(SharedString::from(format!("select-picker-{index}")),title).variant(ButtonVariant::Ghost).on_click(move|_,_,cx|{weak.update(cx,|view,cx|{view.apply_media(&media,cx);view.editor.dialog=None;cx.notify();}).ok();})).into_any_element())
        }).collect()
    }
    fn collection_action(&mut self, kind: &str, row: usize, action: &str, cx: &mut Context<Self>) {
        let Some(index) = self.selected else {
            return;
        };
        let style = editing::style(&self.slide, Some(&self.slide["layers"][index]));
        let key = match kind {
            "tab" => "textTabStops",
            "bar" => "textBarTabStops",
            "trigger" => "linkedTextTimerColorTriggers",
            _ => {
                if self.editor.tab == "text" {
                    "mediaFillEffects"
                } else {
                    "mediaEffects"
                }
            }
        };
        let mut values = if kind == "effect" && self.editor.tab != "text" {
            self.slide["layers"][index][key].as_array()
        } else {
            style[key].as_array()
        }
        .cloned()
        .unwrap_or_default();
        if action == "add" {
            if kind == "bar" {
                let position = values.last().and_then(Value::as_f64).unwrap_or(0.) + 100.;
                if values.len() >= 32 || position > 2000. {
                    return;
                }
                values.push(editing::numeric(position));
            } else if kind == "tab" {
                let position = values
                    .last()
                    .and_then(|v| v["position"].as_f64())
                    .unwrap_or(0.)
                    + 100.;
                if values.len() >= 32 || position > 2000. {
                    return;
                }
                values.push(json!({"position":position,"alignment":"left"}));
            } else {
                values.push(json!({"time":0,"color":editing::DEFAULT_LAYER_COLOR}));
            }
        } else if row < values.len() {
            match action {
                "remove" => {
                    values.remove(row);
                }
                "up" if row > 0 => values.swap(row, row - 1),
                "down" if row + 1 < values.len() => values.swap(row, row + 1),
                _ => return,
            }
        }
        let result = if kind == "effect" && self.editor.tab != "text" {
            let mut slide = (*self.slide).clone();
            slide["layers"][index][key] = json!(values);
            Ok(slide)
        } else {
            editing::patch_text(
                &self.slide,
                Some(index),
                key,
                json!(values),
                self.editor.paragraph,
            )
        };
        if let Ok(slide) = result {
            self.editor.scope.clear();
            self.draft(slide, cx);
        }
    }
    fn add_effect(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(index) = self.selected else {
            return;
        };
        let text = self.editor.tab == "text";
        let key = if text {
            "mediaFillEffects"
        } else {
            "mediaEffects"
        };
        let mut slide = (*self.slide).clone();
        if text && !slide["layers"][index]["textStyle"].is_object() {
            slide["layers"][index]["textStyle"] =
                editing::style(&slide, Some(&slide["layers"][index]));
        }
        let owner = if text {
            &mut slide["layers"][index]["textStyle"]
        } else {
            &mut slide["layers"][index]
        };
        if !owner[key].is_array() {
            owner[key] = json!([]);
        }
        owner[key]
            .as_array_mut()
            .unwrap()
            .push(json!({"effectId":id,"enabled":true}));
        self.draft(slide, cx);
    }
    pub(super) fn inspector_field(
        &mut self,
        spec: Spec,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = *cx.global::<Translator>();
        let label = if [
            "theme.",
            "common.",
            "media.",
            "ndi.",
            "gpui.",
            "slideEditor.",
            "transition.",
            "songMetadata.",
            "newPresentation.",
            "chordChart.",
            "cue.",
            "stage.",
            "automation.",
        ]
        .iter()
        .any(|prefix| spec.label.starts_with(prefix))
        {
            t.t(&spec.label)
        } else {
            spec.label.clone()
        };
        let inline = self.document_tabs()
            || self.selected.is_some()
            || matches!(
                self.kind,
                DocumentKind::Stage | DocumentKind::Mask | DocumentKind::Prop
            );
        let compact_control = self.selected.is_some()
            && matches!(
                spec.key.as_str(),
                "fillType"
                    | "textFillMode"
                    | "fontFamily"
                    | "fontWeight"
                    | "strokeStyle"
                    | "strokeWidth"
                    | "shadowBlur"
                    | "shadowOpacity"
                    | "shadowAngle"
                    | "shadowOffset"
                    | "rotation"
                    | "x"
                    | "y"
                    | "width"
                    | "height"
            );
        let disabled = self.document_controls_disabled()
            || self.ccli_field_read_only(&spec)
            || ((spec.target == Target::Bounds || spec.key == "rotation")
                && self
                    .selected
                    .is_some_and(|i| self.slide["layers"][i]["locked"] == true));
        let label_width = cx.theme().metrics.space_lg * 4.;
        let id = SharedString::from(format!("theme-field-{}", spec.key));
        match spec.kind.clone() {
            Kind::Multiline => {
                let state = if let Some(state) = self.editor.textareas.get(&spec.key) {
                    state.clone()
                } else {
                    let state = orbit_ui::controls::textarea_state(
                        spec.value.as_str().unwrap_or("").to_string(),
                        window,
                        cx,
                    );
                    let field = spec.clone();
                    let scope = self.editor.scope.clone();
                    self.editor.subscriptions.push(cx.subscribe(
                        &state,
                        move |view, state, event, cx| {
                            if matches!(event, InputEvent::Change) && view.editor.scope == scope {
                                view.patch_inspector(
                                    &field,
                                    json!(state.read(cx).value().to_string()),
                                    cx,
                                );
                            }
                        },
                    ));
                    self.editor
                        .textareas
                        .insert(spec.key.clone(), state.clone());
                    state
                };
                Field::new(
                    label.clone(),
                    orbit_ui::controls::Textarea::new(id, &state)
                        .label(label)
                        .rows(7.)
                        .disabled(disabled),
                )
                .into_any_element()
            }
            Kind::Readonly => orbit_ui::controls::OutputField::new(
                label,
                spec.value.as_str().unwrap_or("").to_string(),
            )
            .into_any_element(),
            Kind::Bool => {
                let weak = cx.weak_entity();
                CheckboxField::new(id, label, spec.value == true)
                    .disabled(disabled)
                    .on_change(move |value, _, cx| {
                        weak.update(cx, |view, cx| view.patch_inspector(&spec, json!(value), cx))
                            .ok();
                    })
                    .into_any_element()
            }
            Kind::Color => {
                let weak = cx.weak_entity();
                let picker = ColorPicker::new(
                    id,
                    label,
                    spec.value
                        .as_str()
                        .unwrap_or(editing::DEFAULT_LAYER_COLOR)
                        .to_string(),
                );
                let picker = if spec.target == Target::Text
                    || spec.label == "theme.gpui.textColor"
                    || spec.key.starts_with("textGradient")
                {
                    picker.compact()
                } else {
                    picker
                };
                picker
                    .disabled(disabled)
                    .on_change(move |value, _, cx| {
                        weak.update(cx, |view, cx| view.patch_inspector(&spec, json!(value), cx))
                            .ok();
                    })
                    .into_any_element()
            }
            Kind::Choice(options) => {
                let weak = cx.weak_entity();
                let mut options: Vec<SelectOption<String>> = options
                    .into_iter()
                    .map(|(value, label)| {
                        let icon = if spec.key == "strokeStyle" {
                            match value.as_str() {
                                "solid" => Some("stroke-solid"),
                                "square" => Some("stroke-square"),
                                "short" => Some("stroke-short"),
                                "long" => Some("stroke-long"),
                                _ => None,
                            }
                        } else {
                            None
                        };
                        let option = SelectOption::new(
                            value,
                            if let Some(key) = label.strip_prefix("i18n:") {
                                t.t(key)
                            } else {
                                label
                            },
                        );
                        if let Some(icon) = icon {
                            option.preview_icon(icon)
                        } else {
                            option
                        }
                    })
                    .collect();
                if let Some(value) = spec.value.as_str().filter(|value| !value.is_empty()) {
                    if !options.iter().any(|option| option.value == value) {
                        options.push(
                            SelectOption::new(value.to_string(), value.to_string()).disabled(true),
                        );
                    }
                }
                let control =
                    Select::new(id, options, spec.value.as_str().unwrap_or("").to_string())
                        .label(label.clone())
                        .compact()
                        .disabled(disabled)
                        .on_change(move |value, _, cx| {
                            weak.update(cx, |view, cx| {
                                view.patch_inspector(&spec, json!(value), cx)
                            })
                            .ok();
                        });
                if compact_control {
                    return control.into_any_element();
                }
                Field::new(label, control)
                    .map(|field| {
                        if inline {
                            field.inline().label_width(label_width)
                        } else {
                            field
                        }
                    })
                    .into_any_element()
            }
            Kind::Text | Kind::Number(..) => {
                let state = if let Some(state) = self.editor.inputs.get(&spec.key) {
                    state.clone()
                } else {
                    let initial = spec.value.as_str().map(str::to_string).unwrap_or_else(|| {
                        if spec.value.is_null() {
                            String::new()
                        } else {
                            spec.value.to_string()
                        }
                    });
                    let state = input_state(initial, window, cx);
                    if spec.key == "slideShortcut" {
                        state.update(cx, |state, cx| state.set_readonly(true, cx));
                    }
                    let field = spec.clone();
                    let scope = self.editor.scope.clone();
                    let subscription = cx.subscribe(&state, move |view, state, event, cx| {
                        if view.editor.scope != scope
                            || !matches!(event, InputEvent::Change | InputEvent::Blur)
                        {
                            return;
                        }
                        if field.target == Target::Document && !matches!(event, InputEvent::Blur) {
                            return;
                        }
                        let text = state.read(cx).value().to_string();
                        let value = match field.kind {
                            Kind::Number(min, max) => {
                                let number = text.parse::<f64>().ok().filter(|n| {
                                    n.is_finite()
                                        && *n >= min
                                        && *n <= max
                                        && (!field.integer || n.fract() == 0.)
                                });
                                let Some(number) = number else {
                                    if matches!(event, InputEvent::Blur) {
                                        view.status =
                                            cx.global::<Translator>().t("theme.gpui.invalidValue");
                                        cx.notify();
                                    }
                                    return;
                                };
                                editing::numeric(number)
                            }
                            _ => json!(text),
                        };
                        if matches!(event, InputEvent::Blur) && field.target != Target::Document {
                            return;
                        }
                        view.patch_inspector(&field, value, cx);
                    });
                    self.editor.subscriptions.push(subscription);
                    self.editor.inputs.insert(spec.key.clone(), state.clone());
                    state
                };
                let control = match spec.kind {
                    Kind::Number(min, max) => NumberInput::new(id, &state)
                        .label(label.clone())
                        .when(spec.key == "shadowOpacity", |input| input.suffix("%"))
                        .when(spec.key == "rotation", |input| input.suffix("°"))
                        .range(min, max)
                        .step(
                            if spec.key.ends_with("Delay") || spec.key.ends_with("Duration") {
                                0.1
                            } else if spec.target == Target::Path {
                                0.01
                            } else if spec.key == "letterSpacing"
                                || spec.key.ends_with("Offset") && spec.key.starts_with("linesOnly")
                            {
                                0.1
                            } else if spec.key == "textLineHeightMultiple"
                                || ["Saturation", "Brightness", "Contrast"]
                                    .iter()
                                    .any(|suffix| spec.key.ends_with(suffix))
                            {
                                0.05
                            } else {
                                1.
                            },
                        )
                        .stepper()
                        .when(
                            spec.target == Target::Bounds || spec.key == "rotation",
                            |input| {
                                let field = spec.clone();
                                let weak = cx.weak_entity();
                                let scope = self.editor.scope.clone();
                                input.on_step(move |value, _, cx| {
                                    weak.update(cx, |view, cx| {
                                        if view.editor.scope == scope {
                                            view.patch_inspector(
                                                &field,
                                                editing::numeric(value),
                                                cx,
                                            );
                                        }
                                    })
                                    .ok();
                                })
                            },
                        )
                        .disabled(disabled)
                        .into_any_element(),
                    _ => TextInput::new(id, &state)
                        .when(inline, |input| input.compact())
                        .disabled(disabled)
                        .into_any_element(),
                };
                let control = if spec.key == "slideShortcut" {
                    div()
                        .capture_key_down(move |event: &gpui::KeyDownEvent, window, cx| {
                            if event.keystroke.key == "tab" {
                                return;
                            }
                            window.prevent_default();
                            cx.stop_propagation();
                            if disabled {
                                return;
                            }
                            if let Some(letter) = slide_shortcut_key(&event.keystroke) {
                                state.update(cx, |state, cx| state.set_value(letter, window, cx));
                            }
                        })
                        .child(control)
                        .into_any_element()
                } else {
                    control
                };
                if compact_control {
                    return control;
                }
                let field = Field::new(label, control);
                if inline {
                    field.inline().label_width(label_width).into_any_element()
                } else {
                    field.into_any_element()
                }
            }
        }
    }

    fn ccli_field_read_only(&self, spec: &Spec) -> bool {
        self.selected
            .is_some_and(|index| self.required_ccli_layer(index))
            && matches!(
                spec.key.as_str(),
                "linkedTextEnabled" | "linkedTextSource" | "text"
            )
    }
    pub(super) fn set_document_tab(&mut self, tab: &str, cx: &mut Context<Self>) {
        if tab == self.document_tab() {
            return;
        }
        let edits: Vec<_> = self
            .document_fields(cx)
            .into_iter()
            .filter_map(|field| {
                if field.target != Target::Document
                    || !matches!(field.kind, Kind::Text | Kind::Number(..))
                {
                    return None;
                }
                self.editor
                    .inputs
                    .get(&field.key)
                    .map(|input| (field, input.read(cx).value().to_string()))
            })
            .collect();
        for (field, text) in edits {
            let value = match field.kind {
                Kind::Number(min, max) => {
                    let Some(number) = text.parse::<f64>().ok().filter(|number| {
                        number.is_finite()
                            && *number >= min
                            && *number <= max
                            && (!field.integer || number.fract() == 0.)
                    }) else {
                        self.status = cx.global::<Translator>().t("theme.gpui.invalidValue");
                        cx.notify();
                        return;
                    };
                    editing::numeric(number)
                }
                _ => json!(text),
            };
            if value != field.value {
                self.status.clear();
                self.patch_document_field(&field, &value, cx);
                if !self.status.is_empty() {
                    cx.notify();
                    return;
                }
            }
        }
        self.editor.doc_tab = tab.into();
        cx.notify();
    }

    pub(super) fn document_controls_disabled(&self) -> bool {
        self.saving && !(self.document_tabs() && self.editor.document_preview.is_some())
    }

    pub(super) fn background_field(
        &self,
        presentation: Option<&Value>,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = *cx.theme();
        let t = *cx.global::<Translator>();
        let value = |key: &str| {
            presentation
                .and_then(|root| root.get(key))
                .unwrap_or(&self.slide[key])
        };
        let enabled = value("backgroundEnabled") != false;
        let (target, enabled_key, color_key) = if presentation.is_some() {
            (
                Target::Document,
                "presentation:backgroundEnabled",
                "presentation:background",
            )
        } else {
            (Target::Slide, "backgroundEnabled", "background")
        };
        let disabled = self.document_controls_disabled();
        let owner = cx.weak_entity();
        let background = CheckboxField::new(enabled_key, t.t("theme.background"), enabled)
            .disabled(disabled)
            .on_change(move |value, _, cx| {
                owner
                    .update(cx, |view, cx| {
                        view.patch_inspector(
                            &Spec::new(enabled_key, target, Kind::Bool, json!(enabled)),
                            json!(value),
                            cx,
                        );
                    })
                    .ok();
            });
        let owner = cx.weak_entity();
        let color = ColorPicker::new(
            color_key,
            t.t("theme.background"),
            value("background")
                .as_str()
                .unwrap_or(editing::DEFAULT_SLIDE_BACKGROUND),
        )
        .disabled(disabled || !enabled)
        .on_change(move |color, _, cx| {
            owner
                .update(cx, |view, cx| {
                    view.patch_inspector(
                        &Spec::new(color_key, target, Kind::Color, Value::Null),
                        json!(color),
                        cx,
                    );
                })
                .ok();
        });
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap(theme.metrics.space_sm)
            .child(
                div()
                    .debug_selector(|| "canvas-background-enabled".into())
                    .child(background),
            )
            .child(
                div()
                    .debug_selector(|| "canvas-background-color".into())
                    .flex_none()
                    .w(theme.metrics.control_height * 2.)
                    .child(color),
            )
            .into_any_element()
    }

    fn layout_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let title = self
            .root_document(cx)
            .map_or(Value::Null, |root| root["title"].clone());
        let name =
            Spec::new("documentTitle", Target::Document, Kind::Text, title).named("common.name");
        let size = super::documents_ui::presentation_size(&self.slide, &self.slide);
        let sizes = super::documents_ui::presentation_sizes(&self.slide, &self.slide);
        let size =
            Spec::new("size", Target::Slide, Kind::Choice(sizes), json!(size)).named("theme.size");
        let separator = || {
            div()
                .h(theme.metrics.space_2xs / 2.)
                .bg(theme.colors.border_soft)
        };
        div()
            .id("canvas-layout-fields")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(theme.metrics.space_sm)
            .p(theme.metrics.space_sm)
            .when(self.kind != DocumentKind::Ccli, |panel| {
                panel
                    .child(self.inspector_field(name, window, cx))
                    .child(separator())
            })
            .when(self.kind == DocumentKind::Stage, |panel| {
                panel
                    .child(self.background_field(None, cx))
                    .child(separator())
            })
            .when(self.kind == DocumentKind::Prop, |panel| {
                panel.child(self.background_field(None, cx))
            })
            .child(self.inspector_field(size, window, cx))
            .when(self.kind == DocumentKind::Prop, |panel| {
                panel.children(self.prop_options(cx))
            })
            .into_any_element()
    }

    pub(super) fn editor_inspector(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if !self.editor.loaded {
            self.editor.loaded = true;
            let host = self.runtime.host.clone();
            cx.spawn(async move |view, cx| {
                let results = cx
                    .background_spawn(async move {
                        [
                            "getTransitionCatalog",
                            "getMediaEffectCatalog",
                            "listInstalledFonts",
                            "listNdiSources",
                        ]
                        .into_iter()
                        .map(|method| (method, host.invoke(method, vec![])))
                        .collect::<Vec<_>>()
                    })
                    .await;
                view.update(cx, |view, cx| {
                    for (method, result) in results {
                        match result {
                            Ok(value) => match method {
                                "getTransitionCatalog" => view.editor.transitions = value,
                                "getMediaEffectCatalog" => view.editor.effects = value,
                                "listNdiSources" => {
                                    view.editor.ndi = value;
                                    view.editor.ndi_error.clear();
                                }
                                _ => {
                                    view.editor.fonts = value
                                        .as_array()
                                        .into_iter()
                                        .flatten()
                                        .filter_map(Value::as_str)
                                        .map(str::to_string)
                                        .collect()
                                }
                            },
                            Err(error) if method == "listNdiSources" => {
                                view.editor.ndi_error = error
                            }
                            Err(error) => view.status = error,
                        }
                    }
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
        let theme = *cx.theme();
        let t = *cx.global::<Translator>();
        if self.document.is_none() {
            return div()
                .child(orbit_ui::controls::EmptyState::new(
                    t.t("gpui.editor.empty"),
                ))
                .into_any_element();
        }
        if self.selected.is_some()
            && !matches!(self.editor.tab.as_str(), "shape" | "text" | "build")
        {
            self.editor.tab = if self.editor.tab == "textMedia" {
                "text"
            } else {
                "shape"
            }
            .into();
        }
        let stage = self.kind == DocumentKind::Stage;
        if stage && self.editor.tab == "build" {
            self.editor.tab = "shape".into();
        }
        let scope = format!(
            "{}:{}:{}:{:?}:{}:{}",
            self.workspace,
            self.theme_id,
            self.slide_id,
            self.selected,
            self.editor.tab,
            self.editor.doc_tab
        );
        if self.editor.scope != scope {
            self.editor.scope = scope;
            self.editor.inputs.clear();
            self.editor.ranges.clear();
            self.editor.textareas.clear();
            self.editor.subscriptions.clear();
        }
        let mut panel = div()
            .id("theme-inspector")
            .flex_none()
            .w(px(self.inspector_width))
            .min_h_0()
            .flex()
            .flex_col()
            .border_l_1()
            .border_color(theme.colors.border)
            .bg(theme.colors.panel_bg);
        let weak = cx.weak_entity();
        let document_tabs = self.document_tabs();
        let layout = matches!(
            self.kind,
            DocumentKind::Stage | DocumentKind::Mask | DocumentKind::Ccli | DocumentKind::Prop
        ) && self.selected.is_none();
        let tabs = if layout {
            TabList::new(
                "inspector-tabs",
                t.t("theme.inspector.tabs"),
                vec![TabItem::new(
                    "layout",
                    t.t(match self.kind {
                        DocumentKind::Stage => "stage.layout",
                        DocumentKind::Ccli => "gpui.editor.tab.slide",
                        DocumentKind::Prop => "gpui.editor.tab.prop",
                        _ => "mask.mask",
                    }),
                )],
                "layout",
            )
            .equal_width()
        } else if document_tabs {
            TabList::new(
                "inspector-tabs",
                t.t("theme.inspector.tabs"),
                [
                    ("presentation", "gpui.editor.tab.presentation"),
                    ("slide", "gpui.editor.tab.slide"),
                    ("build", "theme.inspector.build"),
                ]
                .into_iter()
                .map(|(tab, label)| TabItem::new(tab, t.t(label)))
                .collect(),
                self.document_tab(),
            )
            .equal_width()
            .on_change(move |tab, _, cx| {
                weak.update(cx, |view, cx| {
                    view.set_document_tab(tab, cx);
                })
                .ok();
            })
        } else {
            TabList::new(
                "inspector-tabs",
                t.t("theme.inspector.tabs"),
                [
                    ("shape", "theme.inspector.shape"),
                    ("text", "theme.inspector.text"),
                    ("build", "theme.inspector.build"),
                ]
                .into_iter()
                .filter(|(tab, _)| !stage || *tab != "build")
                .map(|(tab, label)| TabItem::new(tab, t.t(label)))
                .collect(),
                match self.editor.tab.as_str() {
                    "text" | "textMedia" => "text",
                    "build" => "build",
                    _ => "shape",
                },
            )
            .equal_width()
            .on_change(move |tab, _, cx| {
                weak.update(cx, |view, cx| {
                    view.editor.tab = tab.into();
                    cx.notify();
                })
                .ok();
            })
        };
        panel = panel.child(
            div()
                .flex_none()
                .debug_selector(|| "canvas-inspector-tabs".into())
                .child(tabs),
        );
        if layout {
            return panel
                .child(self.layout_fields(window, cx))
                .into_any_element();
        }
        let mut fields = div()
            .id("theme-inspector-fields")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p(theme.metrics.space_xs)
            .flex()
            .flex_col()
            .gap(theme.metrics.space_xs);
        if document_tabs {
            return panel
                .child(self.document_inspector(window, cx))
                .into_any_element();
        }
        let tab = match self.editor.tab.as_str() {
            "text" | "textMedia" => "text",
            "build" => "build",
            _ => "shape",
        };
        if self.selected.is_some() {
            self.editor.tab = tab.into();
        }
        let mut inspected = (*self.slide).clone();
        if self.kind != super::content_document::DocumentKind::Theme {
            if let Some(index) = self.selected {
                if editing::is_text(&inspected["layers"][index]) {
                    inspected["text"] = inspected["layers"][index]["text"].clone();
                }
            } else if self.kind == super::content_document::DocumentKind::Slide {
                inspected["text"] = inspected["content"].clone();
            }
        }
        crate::state::text_files::overlay(
            &mut inspected,
            &self.runtime.text_files.read(cx).entries,
        );
        let mut specs = catalog::fields(
            tab,
            &inspected,
            self.selected,
            self.editor.paragraph,
            &self.editor.transitions,
        );
        if self.selected.is_some() && tab == "shape" {
            let mut geometry = catalog::fields(
                "object",
                &inspected,
                self.selected,
                self.editor.paragraph,
                &self.editor.transitions,
            );
            geometry.retain(|field| !matches!(field.key.as_str(), "hidden" | "webUrl"));
            geometry.append(&mut specs);
            specs = geometry;
        }
        if let Some(index) = self.selected {
            let layer = &inspected["layers"][index];
            let style = editing::style(&inspected, Some(layer));
            if tab == "shape" || tab == "text" && style["textFillMode"] == "Media Fill" {
                specs.extend(catalog::fields(
                    if tab == "text" { "textMedia" } else { "media" },
                    &inspected,
                    Some(index),
                    self.editor.paragraph,
                    &self.editor.transitions,
                ));
            }
        }
        if document_tabs {
            if self.document_tab() != "slide" {
                specs.clear();
            }
            let mut document = self.document_fields(cx);
            document.append(&mut specs);
            specs = document;
        } else {
            specs.extend(self.document_fields(cx));
        }
        if !self.runtime.text_files.read(cx).error.is_empty() {
            fields = fields.child(orbit_ui::controls::OutputField::new(
                t.t("theme.linkedText.file"),
                self.runtime.text_files.read(cx).error.clone(),
            ));
        }
        let mut prepared = Vec::new();
        for mut spec in specs {
            if spec.key == "fillType" {
                if let Kind::Choice(options) = &mut spec.kind {
                    let settings = &self.runtime.output.read(cx).settings;
                    let local = &self.runtime.content.read(cx).workspace;
                    let mut screens = settings["screenConfig"]
                        .as_object()
                        .cloned()
                        .unwrap_or_default();
                    if let Some(config) = local["screenConfig"].as_object() {
                        screens.extend(config.clone());
                    }
                    for (id, screen) in screens {
                        options.push((
                            format!("screen-preview:{id}"),
                            screen["label"].as_str().unwrap_or(&id).to_string(),
                        ));
                    }
                    if let Some(layer) = self
                        .selected
                        .map(|index| &self.slide["layers"][index])
                        .filter(|layer| layer["fillType"] == "screen-preview")
                    {
                        let id = layer["previewScreenId"].as_str().unwrap_or("");
                        let value = format!("screen-preview:{id}");
                        if !options.iter().any(|(key, _)| key == &value) {
                            options.push((value.clone(), id.to_string()));
                        }
                        spec.value = json!(value);
                    }
                }
            }
            if spec.key == "fontFamily" && !self.editor.fonts.is_empty() {
                let mut options = self
                    .editor
                    .fonts
                    .iter()
                    .map(|font| (font.clone(), font.clone()))
                    .collect::<Vec<_>>();
                if let Some(requested) = spec.value.as_str() {
                    if !options
                        .iter()
                        .any(|(font, _)| font.eq_ignore_ascii_case(requested))
                    {
                        options.insert(0, (requested.into(), requested.into()));
                    }
                }
                spec.kind = Kind::Choice(options);
            }
            if matches!(spec.key.as_str(), "linkedTextObjectId" | "fillObjectId") {
                spec.kind = Kind::Choice(
                    self.slide["layers"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .enumerate()
                        .filter(|(index, _)| Some(*index) != self.selected)
                        .filter_map(|(_, layer)| {
                            Some((
                                layer["id"].as_str()?.to_string(),
                                layer["name"].as_str().unwrap_or("").to_string(),
                            ))
                        })
                        .collect(),
                );
            }
            if spec.key == "linkedTextTimerId" {
                spec.kind = Kind::Choice(
                    self.runtime.content.read(cx).workspace["countdownTimers"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|timer| {
                            Some((
                                timer["id"].as_str()?.to_string(),
                                timer["name"].as_str()?.to_string(),
                            ))
                        })
                        .chain(std::iter::once((
                            String::new(),
                            t.t("theme.option.automatic"),
                        )))
                        .collect(),
                );
            }
            prepared.push(spec);
        }
        if self.selected.is_some() {
            return panel
                .child(self.object_inspector(prepared, window, cx))
                .into_any_element();
        }
        fields = fields.child(self.background_field(None, cx));
        for spec in prepared {
            fields = fields.child(self.inspector_field(spec, window, cx));
        }
        if document_tabs {
            for element in self.document_tab_widgets(window, cx) {
                fields = fields.child(element);
            }
        }
        if self.selected.is_some() {
            fields = fields.child(self.editor_collections("paragraph", window, cx));
        }
        panel.child(fields).into_any_element()
    }
    pub(super) fn editor_action(
        &mut self,
        action: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.saving {
            return;
        }
        if action == "delete-object"
            && self
                .selected
                .is_some_and(|index| self.required_ccli_layer(index))
        {
            return;
        }
        match action {
            "new-theme" | "rename-theme" | "new-slide" | "new-web" | "delete-theme"
            | "delete-slide" | "rename-object" | "delete-object" => {
                let initial = if action == "rename-theme" {
                    self.runtime
                        .content
                        .read(cx)
                        .themes
                        .iter()
                        .find(|theme| theme.id == self.theme_id)
                        .map(|theme| theme.title.clone())
                        .unwrap_or_default()
                } else if action == "rename-object" {
                    self.selected
                        .and_then(|i| self.slide["layers"][i]["name"].as_str())
                        .unwrap_or("")
                        .into()
                } else {
                    String::new()
                };
                self.editor.dialog = Some((action.into(), input_state(initial, window, cx)));
                cx.notify();
            }
            "slide-up" | "slide-down" => {
                let slides = self.layout_choices(cx);
                if let Some(index) = slides.iter().position(|slide| slide.value == self.slide_id) {
                    let target = if action == "slide-up" {
                        index.checked_sub(1)
                    } else {
                        Some(index + 1)
                    };
                    if let Some(target) = target {
                        self.move_slide(index, target, cx);
                    }
                }
            }
            "new-image" | "media" => {
                self.editor.media_target = if action == "new-image" {
                    "new-image".into()
                } else if self.editor.tab == "text" {
                    "text".into()
                } else {
                    "shape".into()
                };
                self.editor.dialog = Some(("media".into(), input_state("", window, cx)));
                cx.notify();
            }
            "new-ndi" | "new-video-input" => {
                let mut slide = (*self.slide).clone();
                let mut layer = editing::new_layer(&slide, "rectangle");
                if action == "new-ndi" {
                    layer["name"] = json!("NDI");
                    layer["fillType"] = json!("ndi");
                } else {
                    layer["name"] = json!(cx.global::<Translator>().t("theme.fillType.videoInput"));
                    layer["fillType"] = json!("video-input");
                }
                layer["fill"] = json!("transparent");
                layer["fillColors"] = json!(["transparent"]);
                layer["stroke"] = json!("0px transparent");
                layer["strokeEnabled"] = json!(false);
                layer["mediaScale"] = json!("Scale to Fit");
                if self.append_layer(&mut slide, layer).is_none() {
                    cx.notify();
                    return;
                }
                self.editor.tab = "shape".into();
                self.draft(slide, cx);
            }
            "camera" => {
                self.status = cx
                    .global::<Translator>()
                    .t("status.theme.cameraUnavailable");
                cx.notify();
            }
            "file" | "rss" | "asset" => self.load_editor_asset(action, cx),
            "style-copy" => {
                if let Some(index) = self.selected {
                    self.editor.style_clipboard = Some(editing::style(
                        &self.slide,
                        Some(&self.slide["layers"][index]),
                    ));
                }
                cx.notify();
            }
            "style-paste" => {
                if let (Some(index), Some(mut style)) =
                    (self.selected, self.editor.style_clipboard.clone())
                {
                    let mut slide = (*self.slide).clone();
                    if self.required_ccli_layer(index) {
                        style["linkedTextEnabled"] = json!(true);
                        style["linkedTextSource"] = json!("CCLI");
                    }
                    slide["layers"][index]["textStyle"] = style;
                    self.editor.scope.clear();
                    self.draft(slide, cx);
                }
            }
            _ => {}
        }
    }
    fn save_theme_document(
        &mut self,
        expected: Option<Arc<Value>>,
        theme: Value,
        slide: String,
        cx: &mut Context<Self>,
    ) {
        let Some(next_theme_id) = theme["id"].as_str().map(str::to_string) else {
            self.status = "The theme has no identity.".into();
            cx.notify();
            return;
        };
        self.saving = true;
        if let Some(document) = &self.document {
            self.memory.insert(document.key(), self.view);
        }
        self.editor.edit_nonce += 1;
        let task = ThemeDocument::replace(&self.runtime, expected, theme, &slide, cx);
        self.save_id += 1;
        let nonce = self.save_id;
        cx.spawn(async move |view, cx| {
            let result = task.await;
            view.update(cx, |view, cx| {
                if view.save_id != nonce {
                    return;
                }
                view.saving = false;
                match result {
                    Ok(document) => {
                        view.set_slide(document.slide());
                        view.document = None;
                        view.theme_id = next_theme_id;
                        view.slide_id = slide;
                        view.editor.reset();
                        view.open_after_theme_save(cx);
                        view.emit_close_if_requested(cx);
                    }
                    Err(error) => view.status = error,
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
    fn dialog_submit(&mut self, cx: &mut Context<Self>) {
        let Some((action, input)) = self.editor.dialog.as_ref() else {
            return;
        };
        let action = action.clone();
        let mut name = input.read(cx).value().trim().to_string();
        if action == "prop-auto-clear" {
            if let Some(seconds) = name
                .replace(',', ".")
                .parse::<f64>()
                .ok()
                .filter(|seconds| seconds.is_finite() && *seconds > 0.)
            {
                self.editor.dialog = None;
                self.set_prop_auto_clear(Some(seconds), cx);
            } else {
                self.status = cx.global::<Translator>().t("theme.gpui.invalidValue");
                cx.notify();
            }
            return;
        }
        if let Some(id) = action.strip_prefix("rename-mask:") {
            if name.is_empty() {
                self.status = cx.global::<Translator>().t("theme.gpui.nameRequired");
                cx.notify();
                return;
            }
            self.editor.dialog = None;
            self.mutate_mask(
                json!({"type":"updateDocument","maskId":id,"title":name}),
                cx,
            );
            cx.notify();
            return;
        }
        if let Some(prefix) = action.strip_prefix("preset:") {
            self.save_transition_favorite(prefix, &name, None, cx);
            return;
        }
        if action == "new-theme" {
            let base = name.clone();
            let titles = self
                .runtime
                .content
                .read(cx)
                .themes
                .iter()
                .map(|theme| theme.title.clone())
                .collect::<std::collections::HashSet<_>>();
            let mut index = 2;
            while titles.contains(&name) {
                name = format!("{base} {index}");
                index += 1;
            }
        }
        let current = self.runtime.content.read(cx).workspace["themes"]
            .as_array()
            .and_then(|themes| themes.iter().find(|theme| theme["id"] == self.theme_id))
            .cloned();
        if action == "delete-object" {
            self.editor.dialog = None;
            if let Some(index) = self.selected {
                if self.slide["layers"]
                    .get(index)
                    .is_some_and(|layer| layer["locked"] != true)
                {
                    let mut slide = (*self.slide).clone();
                    if let Some(layers) = slide["layers"].as_array_mut() {
                        layers.remove(index);
                    }
                    self.selected = None;
                    self.draft(slide, cx);
                }
            }
            return;
        }
        if action.starts_with("delete-") {
            self.editor.dialog = None;
            self.delete_theme_document(action == "delete-slide", cx);
            return;
        }
        if name.is_empty() {
            self.status = cx.global::<Translator>().t("theme.gpui.nameRequired");
            cx.notify();
            return;
        }
        if action == "new-web" {
            let schema = name
                .split_once("://")
                .map(|(schema, _)| schema)
                .filter(|schema| {
                    schema
                        .chars()
                        .next()
                        .is_some_and(|c| c.is_ascii_alphabetic())
                        && schema
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
                });
            let url = if schema.is_some() {
                name
            } else {
                format!("https://{name}")
            };
            let mut slide = (*self.slide).clone();
            let mut layer = editing::new_layer(&slide, "rectangle");
            layer["name"] = json!("Web");
            layer["shape"] = json!("web");
            layer["fillType"] = json!("web");
            layer["fill"] = json!("Web");
            layer["webUrl"] = json!(url);
            layer["stroke"] = json!("0px #ffffff");
            if self.append_layer(&mut slide, layer).is_none() {
                cx.notify();
                return;
            }
            self.editor.dialog = None;
            self.draft(slide, cx);
            return;
        }
        if action == "rename-object" {
            if let Some(index) = self.selected {
                let mut slide = (*self.slide).clone();
                slide["layers"][index]["name"] = json!(name);
                self.draft(slide, cx);
            }
            self.editor.dialog = None;
            return;
        }
        let mut theme = current.clone().unwrap_or(Value::Null);
        if action == "new-theme" {
            let id = editing::id("theme");
            let slide = default_slide("Theme Slide", 1920, 1080);
            theme = json!({"id":id,"title":name,"sourcePath":"","sourceType":"presenter-orbit","editable":true,"schemaVersion":2,"size":{"width":1920,"height":1080},"slides":[slide]});
        } else if theme.is_object() {
            if let Some(target) = theme["slides"]
                .as_array_mut()
                .and_then(|slides| slides.iter_mut().find(|slide| slide["id"] == self.slide_id))
            {
                *target = (*self.slide).clone();
            }
            if action == "rename-theme" {
                theme["title"] = json!(name);
            } else if action == "new-slide" {
                let width = theme["size"]["width"].as_u64().unwrap_or(1920);
                let height = theme["size"]["height"].as_u64().unwrap_or(1080);
                let mut slide = default_slide(&name, width, height);
                if let Some(previous) = theme["slides"].as_array().and_then(|slides| slides.last())
                {
                    if !previous["background"].is_null() {
                        slide["background"] = previous["background"].clone();
                    }
                    slide["backgroundEnabled"] = json!(previous["backgroundEnabled"] != false);
                }
                let Some(slides) = theme["slides"].as_array_mut() else {
                    self.status = "The theme has no slide list.".into();
                    cx.notify();
                    return;
                };
                slides.push(slide);
            }
        } else {
            return;
        }
        let slide = if matches!(action.as_str(), "new-theme" | "new-slide") {
            let Some(id) = theme["slides"]
                .as_array()
                .and_then(|slides| slides.last())
                .and_then(|slide| slide["id"].as_str())
            else {
                self.status = "The new slide has no identity.".into();
                cx.notify();
                return;
            };
            id.to_string()
        } else {
            self.slide_id.clone()
        };
        self.editor.dialog = None;
        self.save_theme_document(
            if action == "new-theme" {
                None
            } else {
                current.map(Arc::new)
            },
            theme,
            slide,
            cx,
        );
    }
    fn delete_theme_document(&mut self, layout: bool, cx: &mut Context<Self>) {
        let path = self.runtime.content.read(cx).workspace_path.clone();
        let scope = path.clone();
        let host = self.runtime.host.clone();
        let theme = self.theme_id.clone();
        let slide = if layout {
            json!(self.slide_id)
        } else {
            Value::Null
        };
        let answer = self.runtime.lanes.workspace(&path, move |revision| {
            host.invoke(
                "deleteTheme",
                vec![json!(scope), json!(theme), slide, json!(revision)],
            )
        });
        let runtime = self.runtime.clone();
        self.saving = true;
        self.save_id += 1;
        let nonce = self.save_id;
        cx.spawn(async move |view, cx| {
            let result = answer
                .await
                .map_err(|_| "The theme delete worker stopped.".to_string())
                .and_then(|r| r);
            let result = cx
                .background_spawn(async move {
                    let result = result?;
                    crate::collections::model::Snapshot::parse(&result["workspace"])
                })
                .await;
            view.update(cx, |view, cx| {
                if view.save_id != nonce || runtime.content.read(cx).workspace_path != path {
                    return;
                }
                view.saving = false;
                match result {
                    Ok(snapshot) => {
                        runtime.content.update(cx, |content, cx| {
                            content.apply_document(snapshot);
                            cx.notify();
                        });
                        view.editor.reset();
                        view.document = None;
                        view.open_after_theme_save(cx);
                        view.emit_close_if_requested(cx);
                    }
                    Err(error) => view.status = error,
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
    fn open_after_theme_save(&mut self, cx: &mut Context<Self>) {
        if let Some((kind, owner, slide)) = self.editor.navigate.take() {
            self.open_kind(kind, &owner, &slide, cx);
        } else {
            self.open_selected(cx);
        }
    }

    pub(super) fn editor_dialog(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let (action, input) = self.editor.dialog.as_ref()?;
        let action = action.clone();
        let input = input.clone();
        let t = *cx.global::<Translator>();
        let weak = cx.weak_entity();
        let mut dialog = Dialog::new(
            "theme-editor-dialog",
            t.t(if action == "media" {
                "theme.fillType.media"
            } else if action.starts_with("delete-") {
                "common.delete"
            } else if action == "prop-auto-clear" {
                "props.autoClear.title"
            } else if action.starts_with("rename-mask:") {
                "common.rename"
            } else if action.starts_with("preset:") {
                "transition.favoriteName"
            } else if action == "new-web" {
                "theme.gpui.webUrl"
            } else {
                "theme.name"
            }),
            t.t("common.close"),
            move |_, _, cx| {
                weak.update(cx, |view, cx| {
                    view.editor.dialog = None;
                    cx.notify();
                })
                .ok();
            },
        );
        if action.starts_with("rename-mask:") || action == "prop-auto-clear" {
            dialog = dialog.initial_focus(input.read(cx).focus_handle(cx));
        }
        if action == "media" {
            let weak = cx.weak_entity();
            dialog = dialog.child(
                ActionButton::new("theme-asset-file", t.t("common.open")).on_click(
                    move |_, _, cx| {
                        weak.update(cx, |view, cx| view.load_editor_asset("asset", cx))
                            .ok();
                    },
                ),
            );
            let count = self.runtime.content.read(cx).workspace["media"]
                .as_array()
                .map_or(0, Vec::len);
            dialog = dialog.child(
                gpui::uniform_list(
                    "theme-media-picker",
                    count,
                    cx.processor(|view, range, window, cx| {
                        view.media_picker_rows(range, window, cx)
                    }),
                )
                .h(px(340.)),
            );
        } else {
            if action.starts_with("delete-") {
                dialog = dialog.child(t.t("common.delete"));
            } else if action == "prop-auto-clear" {
                let label = t.t("gpui.editor.autoClearSeconds");
                dialog = dialog.child(Field::new(
                    label.clone(),
                    NumberInput::new("prop-editor-custom-seconds", &input).label(label),
                ));
            } else {
                dialog = dialog.child(Field::new(
                    t.t(if action.starts_with("preset:") {
                        "transition.favoriteName"
                    } else if action == "new-web" {
                        "theme.gpui.webUrl"
                    } else {
                        "theme.name"
                    }),
                    div()
                        .debug_selector(|| "canvas-document-name".into())
                        .child(TextInput::new("theme-document-name", &input)),
                ));
            }
            let weak = cx.weak_entity();
            dialog = dialog.footer(
                ActionButton::new(
                    "theme-dialog-submit",
                    t.t(if action.starts_with("delete-") {
                        "common.delete"
                    } else {
                        "common.done"
                    }),
                )
                .variant(if action.starts_with("delete-") {
                    ButtonVariant::Danger
                } else {
                    ButtonVariant::Primary
                })
                .disabled(self.saving)
                .on_click(move |_, _, cx| {
                    weak.update(cx, |view, cx| view.dialog_submit(cx)).ok();
                }),
            );
        }
        Some(dialog.into_any_element())
    }
    fn apply_media(&mut self, media: &Value, cx: &mut Context<Self>) {
        if !media["sourceUrl"].is_string() {
            self.status = "The selected media has no source URL.".into();
            cx.notify();
            return;
        }
        let text = self.editor.media_target == "text";
        let mut slide = (*self.slide).clone();
        let index = if let Some(index) = self
            .selected
            .filter(|_| self.editor.media_target != "new-image")
        {
            index
        } else {
            let layer = editing::new_layer(&slide, "rectangle");
            let Some(index) = self.append_layer(&mut slide, layer) else {
                cx.notify();
                return;
            };
            index
        };
        let owner = if text {
            if !slide["layers"][index]["textStyle"].is_object() {
                slide["layers"][index]["textStyle"] =
                    editing::style(&slide, Some(&slide["layers"][index]));
            }
            &mut slide["layers"][index]["textStyle"]
        } else {
            &mut slide["layers"][index]
        };
        let prefix = if text { "mediaFill" } else { "media" };
        for (suffix, source) in [
            ("Id", "id"),
            ("Url", "sourceUrl"),
            ("Title", "title"),
            ("Type", "type"),
        ] {
            owner[format!("{prefix}{suffix}")] = media[source].clone();
        }
        if text {
            owner["mediaFillSourcePath"] = media["sourcePath"].clone();
        } else {
            owner["fillType"] = json!("media");
            owner["fill"] = json!("Media");
            owner["sourcePath"] = media["sourcePath"].clone();
            owner["sourceUrl"] = media["sourceUrl"].clone();
            if owner["mediaScale"].is_null() {
                owner["mediaScale"] = json!("Scale to Fit");
            }
        }
        self.draft(slide, cx);
    }
    fn load_editor_asset(&mut self, kind: &str, cx: &mut Context<Self>) {
        let method = match kind {
            "asset" => "selectThemeAssetFile",
            "file" => "selectPresentationTextFile",
            _ => "fetchRssFeed",
        };
        let host = self.runtime.host.clone();
        let index = self.selected;
        let scope = self.editor.scope.clone();
        let args = if kind == "rss" {
            vec![
                editing::style(&self.slide, index.map(|i| &self.slide["layers"][i]))
                    ["linkedTextRssUrl"]
                    .clone(),
            ]
        } else {
            vec![]
        };
        cx.spawn(async move |view, cx| {
            let result = cx
                .background_spawn(async move { host.invoke(method, args) })
                .await;
            view.update(cx, |view, cx| {
                if view.editor.scope != scope {
                    return;
                }
                match result {
                    Ok(result) if result["canceled"] != true => {
                        if method == "selectThemeAssetFile" {
                            view.apply_media(&result["media"], cx);
                            view.editor.dialog = None;
                        } else if let Some(index) = index.filter(|i| {
                            view.slide["layers"]
                                .as_array()
                                .is_some_and(|l| *i < l.len())
                        }) {
                            let mut slide = (*view.slide).clone();
                            if !slide["layers"][index]["textStyle"].is_object() {
                                slide["layers"][index]["textStyle"] =
                                    editing::style(&slide, Some(&slide["layers"][index]));
                            }
                            let style = &mut slide["layers"][index]["textStyle"];
                            if method == "selectPresentationTextFile" {
                                style["linkedTextFilePath"] = result["filePath"].clone();
                                style["linkedTextFileContent"] = result["text"].clone();
                            } else {
                                style["linkedTextRssItems"] = json!(result["items"].to_string());
                                match editing::format_rss(style) {
                                    Ok(text) => style["linkedTextRssContent"] = json!(text),
                                    Err(error) => {
                                        view.status = error;
                                        cx.notify();
                                        return;
                                    }
                                }
                            }
                            view.draft(slide, cx);
                        }
                    }
                    Ok(_) => {}
                    Err(error) => view.status = error,
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
    /// Appends `layer` to the slide's layer list and selects it. None when `draft` would refuse the edit or
    /// the slide has no layer list.
    pub(super) fn append_layer(&mut self, slide: &mut Value, layer: Value) -> Option<usize> {
        if self.saving || self.document.is_none() {
            return None;
        }
        let Some(layers) = slide["layers"].as_array_mut() else {
            self.status = "No slide is open.".into();
            return None;
        };
        let index = layers.len();
        layers.push(layer);
        self.selected = Some(index);
        Some(index)
    }
    fn add_object(&mut self, shape: &str, cx: &mut Context<Self>) {
        if self.document.is_none() || self.saving {
            return;
        }
        self.editor.pen.clear();
        self.editor.pen_tool = false;
        let mut slide = (*self.slide).clone();
        let layer = editing::new_layer(&slide, shape);
        if self.append_layer(&mut slide, layer).is_none() {
            cx.notify();
            return;
        }
        self.editor.scope.clear();
        self.draft(slide, cx);
    }
    /// The shapes tool: an icon whose menu lists every shape with its shortcut.
    fn shape_menu(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let t = *cx.global::<Translator>();
        let open = self.editor.shape_menu;
        let (toggle, dismiss) = (cx.weak_entity(), cx.weak_entity());
        let trigger = IconButton::new("theme-shape-tool", "shapes", t.t("theme.tool.shapes"))
            .selected(open)
            .disabled(self.saving || self.document.is_none())
            .on_click(move |_, _, cx| {
                toggle
                    .update(cx, |view, cx| {
                        view.editor.shape_menu = !view.editor.shape_menu;
                        cx.notify();
                    })
                    .ok();
            });
        let mut popover =
            Popover::new("theme-shape-popover", trigger).on_dismiss(move |_, _, cx| {
                dismiss
                    .update(cx, |view, cx| {
                        view.editor.shape_menu = false;
                        cx.notify();
                    })
                    .ok();
            });
        if open {
            let mappings =
                crate::keys::KeyMappings::from_settings(&self.runtime.output.read(cx).settings);
            let mut panel = div()
                .flex()
                .flex_col()
                .w(px(200.))
                .py(px(3.))
                .border_1()
                .border_color(theme.colors.border)
                .bg(theme.colors.popover_bg)
                .shadow(theme.shadow_soft());
            for (shape, glyph) in [
                ("rectangle", "square"),
                ("rounded-rect", "square-rounded"),
                ("ellipse", "circle"),
                ("triangle", "triangle"),
                ("right-triangle", "triangle-right"),
                ("rhombus", "diamond"),
                ("polygon", "hexagon"),
                ("star", "star"),
                ("double-arrow", "arrows-horizontal"),
                ("arrow", "arrow-right"),
            ] {
                let weak = cx.weak_entity();
                let shortcut = mappings
                    .shortcut(&crate::keys::theme_shape_command_id(shape))
                    .map(crate::keys::format_shortcut_label)
                    .unwrap_or_default();
                panel = panel.child(
                    list_row(
                        SharedString::from(format!("theme-shape-{shape}")),
                        false,
                        cx,
                    )
                    .debug_selector(move || format!("canvas-shape-{shape}"))
                    .cursor_pointer()
                    .child(orbit_ui::icon(glyph).text_color(theme.colors.text_muted))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(t.t(&format!("theme.shape.{shape}"))),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_color(theme.colors.text_muted)
                            .child(shortcut),
                    )
                    .on_click(move |_, _, cx| {
                        weak.update(cx, |view, cx| {
                            view.editor.shape_menu = false;
                            view.add_object(shape, cx);
                        })
                        .ok();
                    }),
                );
            }
            popover = popover.content(panel);
        }
        div()
            .flex_none()
            .w(theme.metrics.control_height)
            .child(popover)
            .into_any_element()
    }
    pub(super) fn editor_tools(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let t = *cx.global::<Translator>();
        let mut tools = div()
            .flex()
            .items_center()
            .gap(theme.metrics.space_2xs)
            .p(theme.metrics.space_2xs)
            .rounded(theme.metrics.radius_sm)
            .bg(theme.colors.control_bg);
        let weak = cx.weak_entity();
        tools = tools.child(
            ActionButton::new("theme-text-tool", t.t("theme.tool.text"))
                .icon("letter-t")
                .variant(ButtonVariant::Ghost)
                .disabled(self.saving || self.document.is_none())
                .on_click(move |_, _, cx| {
                    weak.update(cx, |view, cx| view.add_object("text", cx)).ok();
                }),
        );
        let shapes = self.shape_menu(cx);
        tools = tools.child(shapes);
        let weak = cx.weak_entity();
        tools = tools.child(
            IconButton::new("theme-pen", "pencil", t.t("theme.tool.pen"))
                .selected(self.editor.pen_tool)
                .disabled(self.saving || self.document.is_none())
                .on_click(move |_, _, cx| {
                    weak.update(cx, |view, cx| {
                        view.editor.pen_tool = !view.editor.pen_tool;
                        view.editor.pen.clear();
                        cx.notify();
                    })
                    .ok();
                }),
        );
        let tools_for_document = [
            ("new-image", "theme.fillType.media", "polaroid"),
            ("new-video-input", "theme.fillType.videoInput", "video"),
            ("new-web", "theme.fillType.web", "world"),
            ("new-ndi", "ndi.tool", "video"),
            ("camera", "theme.fillType.videoInput", "video"),
            ("style-copy", "theme.inspector.copyStyle", "copy-check"),
            (
                "style-paste",
                "theme.inspector.pasteStyle",
                "clipboard-text",
            ),
        ];
        for (action, key, icon) in
            tools_for_document
                .into_iter()
                .filter(|(action, _, _)| match self.kind {
                    DocumentKind::Mask => *action == "new-image",
                    DocumentKind::Ccli => {
                        matches!(*action, "new-image" | "new-video-input" | "new-web")
                    }
                    _ => *action != "new-video-input",
                })
        {
            let weak = cx.weak_entity();
            tools = tools.child(
                IconButton::new(
                    SharedString::from(format!("theme-{action}")),
                    icon,
                    t.t(key),
                )
                .disabled(self.saving || self.document.is_none())
                .on_click(move |_, window, cx| {
                    weak.update(cx, |view, cx| view.editor_action(action, window, cx))
                        .ok();
                }),
            );
        }
        tools.into_any_element()
    }
    pub(super) fn editor_sidebar(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = *cx.theme();
        let t = *cx.global::<Translator>();
        let mut sidebar = div()
            .id("theme-sidebar")
            .flex_none()
            .w(px(self.sidebar_width))
            .min_h_0()
            .flex()
            .flex_col()
            .bg(theme.colors.panel_bg)
            .border_r_1()
            .border_color(theme.colors.border);
        let mut header = PanelHeader::new(t.t("theme.themes"));
        for (action, label, icon) in [
            ("new-theme", "common.new", "plus"),
            ("rename-theme", "common.rename", "pencil"),
        ] {
            let weak = cx.weak_entity();
            header = header.action(
                IconButton::new(SharedString::from(action), icon, t.t(label))
                    .disabled(self.saving)
                    .on_click(move |_, window, cx| {
                        weak.update(cx, |view, cx| view.editor_action(action, window, cx))
                            .ok();
                    }),
            );
        }
        sidebar = sidebar.child(header);
        let content = self.runtime.content.read(cx);
        let themes = content.themes.clone();
        let options = themes
            .iter()
            .map(|theme| SelectOption::new(theme.id.clone(), theme.title.clone()))
            .collect();
        let first_slides: HashMap<String, String> = themes
            .into_iter()
            .filter_map(|theme| Some((theme.id, theme.slides.first()?.0.clone())))
            .collect();
        let weak = cx.weak_entity();
        sidebar = sidebar.child(
            div()
                .debug_selector(|| "theme-select".into())
                .p(theme.metrics.space_xs)
                .child(
                    Select::new("theme-select", options, self.theme_id.clone())
                        .compact()
                        .label(t.t("theme.theme"))
                        .disabled(self.saving)
                        .on_change(move |id, _, cx| {
                            if let Some(slide) = first_slides.get(&id) {
                                weak.update(cx, |view, cx| {
                                    if !view.saving {
                                        view.open(&id, slide, cx);
                                    }
                                })
                                .ok();
                            }
                        }),
                ),
        );
        let slide_header = PanelHeader::new(t.t("theme.slides")).action(
            IconButton::new("new-slide", "plus", t.t("common.add"))
                .disabled(self.saving || self.document.is_none())
                .on_click(
                    cx.listener(|view, _, window, cx| view.editor_action("new-slide", window, cx)),
                ),
        );
        let upper = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(slide_header)
            .child(
                div()
                    .id("theme-slides-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(self.slide_list(cx)),
            );
        let sidebar = sidebar.child(self.sidebar_panels(upper, cx));
        self.track_rows(sidebar, cx).into_any_element()
    }
    /// Moves a slide of the open theme to index `to`, keeping the open slide's unsaved edits.
    pub(super) fn move_theme_slide(&mut self, from: usize, to: usize, cx: &mut Context<Self>) {
        let Some(current) = self.runtime.content.read(cx).workspace["themes"]
            .as_array()
            .and_then(|themes| themes.iter().find(|theme| theme["id"] == self.theme_id))
            .cloned()
        else {
            return;
        };
        let mut theme = current.clone();
        let Some(slides) = theme["slides"].as_array_mut() else {
            return;
        };
        if let Some(open) = slides.iter().position(|slide| slide["id"] == self.slide_id) {
            slides[open] = (*self.slide).clone();
        }
        if super::sidebar::moved(slides, from, to) {
            self.save_theme_document(Some(Arc::new(current)), theme, self.slide_id.clone(), cx);
        }
    }
    pub(super) fn object_action(&mut self, index: usize, action: &str, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        if action == "hide" && self.required_ccli_layer(index) {
            return;
        }
        let mut slide = (*self.slide).clone();
        let Some(layers) = slide["layers"].as_array_mut() else {
            return;
        };
        if index >= layers.len() {
            return;
        }
        self.selected = Some(index);
        match action {
            "hide" => layers[index]["hidden"] = json!(layers[index]["hidden"] != true),
            "lock" => layers[index]["locked"] = json!(layers[index]["locked"] != true),
            _ => return,
        };
        self.draft(slide, cx);
    }
    pub(super) fn editor_pen_down(
        &mut self,
        p: Point,
        button: MouseButton,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.editor.pen_tool || button != MouseButton::Left || self.saving {
            return false;
        }
        let point = self.view.to_slide(p);
        let size = self.view.slide;
        let point = Point::new(
            geometry::round(point.x.clamp(0., size.x)),
            geometry::round(point.y.clamp(0., size.y)),
        );
        let close = self.editor.pen.len() >= 3
            && self.editor.pen.first().is_some_and(|first| {
                (point.x - first.x).hypot(point.y - first.y)
                    <= 14f32.max(size.x.min(size.y) * 0.015)
            });
        if close {
            self.finish_pen(true, cx);
        } else {
            self.editor.pen.push(point);
            cx.notify();
        }
        true
    }
    fn finish_pen(&mut self, closed: bool, cx: &mut Context<Self>) {
        let points = std::mem::take(&mut self.editor.pen);
        self.editor.pen_tool = false;
        if points.len() < 2 {
            cx.notify();
            return;
        }
        match editing::pen_layer(&self.slide, &points, closed) {
            Ok(layer) => {
                let mut slide = (*self.slide).clone();
                if self.append_layer(&mut slide, layer).is_none() {
                    cx.notify();
                    return;
                }
                self.draft(slide, cx);
            }
            Err(error) => {
                self.status = error;
                cx.notify();
            }
        }
    }
    pub(super) fn editor_key(
        &mut self,
        event: &KeyDownEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.editor.pen_tool && matches!(event.keystroke.key.as_str(), "enter" | "escape") {
            self.finish_pen(false, cx);
            cx.stop_propagation();
            return true;
        }
        if self.editor.dialog.is_some() || self.tools.editing_zoom() {
            return false;
        }
        let mappings =
            crate::keys::KeyMappings::from_settings(&self.runtime.output.read(cx).settings);
        let Some(shortcut) = crate::keys::shortcut_from_keystroke(&event.keystroke) else {
            return false;
        };
        let shape = crate::keys::THEME_SHAPES
            .iter()
            .find(|(shape, _)| {
                mappings.shortcut(&crate::keys::theme_shape_command_id(shape))
                    == Some(shortcut.as_str())
            })
            .map(|(shape, _)| *shape);
        let Some(shape) = shape else {
            return false;
        };
        self.add_object(shape, cx);
        cx.stop_propagation();
        true
    }
}

fn default_slide(name: &str, width: u64, height: u64) -> Value {
    json!({"id":editing::id("slide"),"name":name,"text":"Text","background":editing::DEFAULT_SLIDE_BACKGROUND,"backgroundEnabled":true,"width":width,"height":height,"layers":[{"id":"text","name":"Text","type":"Text","role":"slide-text","hidden":false,"textStyle":editing::defaults()}]})
}
