use super::*;
use orbit_ui::controls::{
    range_state, CheckboxField, ColorPicker, ContextMenu, Field, InputEvent, MenuEntry, MenuItem,
    RangeInput, SliderEvent,
};

#[derive(Clone, Copy)]
pub(super) enum EffectMenu {
    Presets,
    Add,
    Row(usize),
}

pub(super) enum PresetDialog {
    Name {
        id: Option<String>,
        input: Entity<InputState>,
        _subscription: Subscription,
    },
    Delete(String),
}

#[derive(Clone)]
struct EffectDrag {
    owner: String,
    index: usize,
    title: String,
}

enum EffectAction {
    Add(String, Option<usize>),
    Remove(usize),
    Apply(String),
    Clear,
    Unlink,
    Save,
    SaveAs,
    Rename(String),
    Delete(String),
}

const COLOR_FIELDS: [(&str, f64); 4] = [
    ("mediaHue", 0.),
    ("mediaSaturation", 1.),
    ("mediaBrightness", 0.),
    ("mediaContrast", 1.),
];

fn chain(item: &Value) -> &[Value] {
    item["mediaEffects"].as_array().map_or(&[], Vec::as_slice)
}

fn preset_payload(item: &Value) -> Value {
    let effects: serde_json::Map<_, _> = COLOR_FIELDS
        .into_iter()
        .map(|(key, neutral)| {
            (
                key.to_owned(),
                item.get(key).cloned().unwrap_or_else(|| json!(neutral)),
            )
        })
        .collect();
    json!({"effects":effects,"mediaEffects":chain(item)})
}

fn matches_preset(item: &Value, preset: &Value) -> bool {
    chain(item) == chain(preset)
        && COLOR_FIELDS.into_iter().all(|(key, neutral)| {
            item[key].as_f64().unwrap_or(neutral)
                == preset["effects"][key].as_f64().unwrap_or(neutral)
        })
}

fn apply_preset(item: &mut Value, preset: &Value) {
    for (key, neutral) in COLOR_FIELDS {
        item[key] = preset["effects"]
            .get(key)
            .cloned()
            .unwrap_or_else(|| json!(neutral));
    }
    item["mediaEffects"] = json!(chain(preset));
}

impl MediaBin {
    pub(super) fn effect_variable(&mut self, key: &str, value: Value, cx: &mut Context<Self>) {
        let Some((index, name)) = key
            .strip_prefix("effect:")
            .and_then(|key| key.split_once(':'))
        else {
            return;
        };
        let Some(index) = index.parse::<usize>().ok() else {
            return;
        };
        let mut chain = self
            .media
            .iter()
            .find(|item| item["id"] == self.selected)
            .and_then(|item| item["mediaEffects"].as_array())
            .cloned()
            .unwrap_or_default();
        let Some(effect) = chain.get_mut(index).and_then(Value::as_object_mut) else {
            return;
        };
        let variables = effect.entry("variables").or_insert_with(|| json!({}));
        let Some(variables) = variables.as_object_mut() else {
            return;
        };
        variables.insert(name.to_string(), value);
        self.patch_asset("mediaEffects", json!(chain), cx);
    }
    pub(super) fn effects(
        &mut self,
        item: &Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let t = *cx.global::<Translator>();
        let theme = *cx.theme();
        let effects = chain(item);
        let preset = self.effect_preset(item);
        let title = preset.map_or_else(
            || t.t("media.effects.noPreset"),
            |preset| {
                let title = text(preset, "title");
                if matches_preset(item, preset) {
                    title.to_owned()
                } else {
                    format!("{title}*")
                }
            },
        );
        let mut body = div().flex().flex_col().gap(theme.metrics.space_xs).child(
            Field::new(
                t.t("media.effects.preset"),
                ActionButton::new("media-preset", title)
                    .compact()
                    .variant(ButtonVariant::Soft)
                    .disabled(self.edits_blocked())
                    .on_click(cx.listener(|this, event: &gpui::ClickEvent, _, cx| {
                        this.effect_menu = Some((EffectMenu::Presets, event.position()));
                        cx.notify();
                    })),
            )
            .inline()
            .label_width(theme.metrics.space_lg * 7.),
        );
        for (index, effect) in effects.iter().enumerate() {
            let entry = self
                .effect_catalog
                .iter()
                .find(|entry| entry["id"] == effect["effectId"])
                .cloned();
            let title = entry
                .as_ref()
                .map(|entry| text(entry, "name"))
                .unwrap_or_else(|| text(effect, "effectId"))
                .to_string();
            let weak = cx.weak_entity();
            let enabled = effect["enabled"] == true;
            let drag = EffectDrag {
                owner: self.selected.clone(),
                index,
                title: title.clone(),
            };
            let menu = move |this: &mut Self, position, cx: &mut Context<Self>| {
                if !this.edits_blocked() {
                    this.effect_menu = Some((EffectMenu::Row(index), position));
                    cx.notify();
                }
            };
            body = body.child(
                div()
                    .id(("media-effect-row", index))
                    .debug_selector(move || format!("media-effect-row-{index}"))
                    .flex()
                    .items_center()
                    .justify_between()
                    .border_b_1()
                    .border_color(theme.colors.border_soft)
                    .on_mouse_down(
                        gpui::MouseButton::Right,
                        cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                            menu(this, event.position, cx);
                            cx.stop_propagation();
                        }),
                    )
                    .on_drop::<EffectDrag>(cx.listener(move |this, drag: &EffectDrag, _, cx| {
                        if drag.owner != this.selected
                            || drag.index == index
                            || this.edits_blocked()
                        {
                            return;
                        }
                        let mut effects = this
                            .media
                            .iter()
                            .find(|item| item["id"] == drag.owner)
                            .map_or_else(Vec::new, |item| chain(item).to_vec());
                        if index < effects.len() && drag.index < effects.len() {
                            let effect = effects.remove(drag.index);
                            effects.insert(index, effect);
                            this.inputs.retain(|key, _| !key.starts_with("effect:"));
                            this.patch_asset("mediaEffects", json!(effects), cx);
                        }
                    }))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .child(
                                CheckboxField::new(
                                    SharedString::from(format!("effect-enabled-{index}")),
                                    title.clone(),
                                    enabled,
                                )
                                .bare()
                                .disabled(self.edits_blocked())
                                .on_change(
                                    move |enabled, _, cx| {
                                        let _ = weak.update(cx, |this, cx| {
                                            let mut chain = this
                                                .media
                                                .iter()
                                                .find(|item| item["id"] == this.selected)
                                                .and_then(|item| item["mediaEffects"].as_array())
                                                .cloned()
                                                .unwrap_or_default();
                                            if let Some(effect) = chain.get_mut(index) {
                                                effect["enabled"] = json!(enabled);
                                                this.patch_asset("mediaEffects", json!(chain), cx);
                                            }
                                        });
                                    },
                                ),
                            )
                            .child(
                                div()
                                    .id(("media-effect-drag", index))
                                    .debug_selector(move || format!("media-effect-drag-{index}"))
                                    .cursor(gpui::CursorStyle::OpenHand)
                                    .p(theme.metrics.space_xs)
                                    .on_drag(drag, |drag, _, _, cx| {
                                        cx.new(|_| super::tree::DragLabel(drag.title.clone()))
                                    })
                                    .child(title),
                            ),
                    )
                    .child(
                        IconButton::new(
                            ("media-effect-menu", index),
                            "chevron-down",
                            t.t("media.effects.actions"),
                        )
                        .disabled(self.edits_blocked())
                        .on_click(cx.listener(
                            move |this, event: &gpui::ClickEvent, _, cx| {
                                menu(this, event.position(), cx)
                            },
                        )),
                    ),
            );
            if !enabled {
                continue;
            }
            if let Some(variables) = entry
                .as_ref()
                .and_then(|entry| entry["variables"].as_array())
            {
                for variable in variables {
                    let name = text(variable, "name");
                    let key = format!("effect:{index}:{name}");
                    let value = effect["variables"].get(name).unwrap_or(&variable["value"]);
                    if variable["type"] == "color" {
                        if let Some(color) = value.as_str() {
                            let weak = cx.weak_entity();
                            let target = key.clone();
                            body = body.child(
                                Field::new(
                                    text(variable, "label").to_string(),
                                    ColorPicker::new(
                                        SharedString::from(key),
                                        text(variable, "label").to_string(),
                                        color,
                                    )
                                    .disabled(self.edits_blocked())
                                    .on_change(
                                        move |value, _, cx| {
                                            let _ = weak.update(cx, |this, cx| {
                                                this.effect_variable(&target, json!(value), cx)
                                            });
                                        },
                                    ),
                                )
                                .inline(),
                            );
                        }
                    } else if let (Some(value), Some(min), Some(max)) = (
                        value.as_f64(),
                        variable["min"].as_f64(),
                        variable["max"].as_f64(),
                    ) {
                        body = body.child(self.effect_numeric(
                            &key,
                            text(variable, "label"),
                            value,
                            (min, max),
                            window,
                            cx,
                        ));
                    }
                }
            }
        }
        body.child(
            div().flex().justify_center().child(
                IconButton::new("effect-add", "plus", t.t("media.effectChain.add"))
                    .full_height()
                    .large_icon()
                    .disabled(self.edits_blocked() || self.effect_catalog.is_empty())
                    .on_click(cx.listener(|this, event: &gpui::ClickEvent, _, cx| {
                        this.effect_menu = Some((EffectMenu::Add, event.position()));
                        cx.notify();
                    })),
            ),
        )
        .into_any_element()
    }

    fn effect_numeric(
        &mut self,
        key: &str,
        label: &str,
        value: f64,
        (min, max): (f64, f64),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let field = key.to_owned();
        let owner = self.selected.clone();
        let state = window.use_keyed_state(
            SharedString::from(format!("effect-slider-{owner}-{field}")),
            cx,
            |_, cx| {
                let slider = range_state(
                    min as f32,
                    max as f32,
                    ((max - min) / 100.).max(0.001) as f32,
                    value as f32,
                    cx,
                );
                (slider, value, None::<Subscription>)
            },
        );
        let slider = state.read(cx).0.clone();
        if state.read(cx).2.is_none() {
            let subscription = cx.subscribe(&slider, move |this, _, event: &SliderEvent, cx| {
                if this.selected == owner {
                    if let SliderEvent::Release(value) = event {
                        this.inputs.remove(&field);
                        this.effect_variable(
                            &field,
                            json!(f64::from(value.end()).clamp(min, max)),
                            cx,
                        );
                    }
                }
            });
            state.update(cx, |state, _| state.2 = Some(subscription));
        }
        if state.read(cx).1 != value {
            state.update(cx, |state, _| state.1 = value);
            slider.update(cx, |slider, cx| slider.set_value(value as f32, window, cx));
        }
        let width = cx.theme().metrics.control_height * 2.;
        let gap = cx.theme().metrics.space_xs;
        let input = self
            .numeric_input(key, value, (min, max), window, cx)
            .label(label.to_owned());
        Field::new(
            label.to_owned(),
            div()
                .flex()
                .items_center()
                .gap(gap)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .debug_selector({
                            let key = key.to_owned();
                            move || format!("effect-slider-{key}")
                        })
                        .child(
                            RangeInput::new(&slider)
                                .label(label.to_owned())
                                .disabled(self.edits_blocked()),
                        ),
                )
                .child(div().flex_none().w(width).child(input)),
        )
        .inline()
    }

    fn effect_presets(&self) -> &[Value] {
        self.source["mediaEffectPresets"]
            .as_array()
            .map_or(&[], Vec::as_slice)
    }

    fn effect_preset(&self, item: &Value) -> Option<&Value> {
        self.effect_presets()
            .iter()
            .find(|preset| preset["id"] == item["mediaEffectPresetId"])
    }

    pub(super) fn effect_menu(&self, item: &Value, cx: &mut Context<Self>) -> Option<ContextMenu> {
        let (menu, point) = self.effect_menu?;
        let t = *cx.global::<Translator>();
        let owner = cx.weak_entity();
        let action = |label: String, command| {
            let owner = owner.clone();
            MenuItem::new(label).on_click(move |_, window, cx| {
                let _ = owner.update(cx, |this, cx| this.effect_action(&command, window, cx));
            })
        };
        let entries = match menu {
            EffectMenu::Add | EffectMenu::Row(_) => {
                let index = match menu {
                    EffectMenu::Row(index) => Some(index),
                    _ => None,
                };
                let mut entries = Vec::new();
                if let Some(index) = index {
                    entries.push(
                        action(t.t("media.effectChain.remove"), EffectAction::Remove(index)).into(),
                    );
                    entries.push(MenuEntry::Separator);
                }
                entries.extend(self.effect_catalog.iter().map(|entry| {
                    action(
                        text(entry, "name").to_owned(),
                        EffectAction::Add(text(entry, "id").to_owned(), index),
                    )
                    .into()
                }));
                entries
            }
            EffectMenu::Presets => {
                let preset = self.effect_preset(item);
                let has_effects = !matches_preset(item, &Value::Null);
                let mut entries = Vec::new();
                if has_effects || preset.is_some() {
                    entries.push(action(t.t("media.effects.clear"), EffectAction::Clear).into());
                }
                if let Some(preset) = preset {
                    entries.push(
                        action(t.t("media.effects.unlinkPreset"), EffectAction::Unlink).into(),
                    );
                    if !matches_preset(item, preset) {
                        entries.push(
                            action(t.t("common.save"), EffectAction::Save)
                                .disabled(self.busy)
                                .into(),
                        );
                    }
                }
                if has_effects {
                    entries
                        .push(action(t.t("media.effects.savePreset"), EffectAction::SaveAs).into());
                }
                if !entries.is_empty() && !self.effect_presets().is_empty() {
                    entries.push(MenuEntry::Separator);
                }
                entries.extend(self.effect_presets().iter().map(|preset| {
                    let id = text(preset, "id");
                    MenuEntry::SelectableSubmenu {
                        item: action(
                            text(preset, "title").to_owned(),
                            EffectAction::Apply(id.to_owned()),
                        )
                        .checked(item["mediaEffectPresetId"] == id),
                        entries: vec![
                            action(t.t("common.rename"), EffectAction::Rename(id.to_owned()))
                                .into(),
                            action(t.t("common.delete"), EffectAction::Delete(id.to_owned()))
                                .into(),
                        ],
                    }
                }));
                if entries.is_empty() {
                    entries.push(
                        MenuItem::new(t.t("media.effects.noPreset"))
                            .disabled(true)
                            .into(),
                    );
                }
                entries
            }
        };
        Some(ContextMenu::new(
            "media-effect-menu",
            point,
            entries,
            move |_, _, cx| {
                let _ = owner.update(cx, |this, cx| {
                    this.effect_menu = None;
                    cx.notify();
                });
            },
        ))
    }

    fn effect_action(
        &mut self,
        action: &EffectAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.edits_blocked() {
            return;
        }
        match action {
            EffectAction::SaveAs => self.open_preset_name(None, window, cx),
            EffectAction::Rename(id) => self.open_preset_name(Some(id.clone()), window, cx),
            EffectAction::Delete(id) => {
                self.effect_preset_dialog = Some(PresetDialog::Delete(id.clone()));
                cx.notify();
            }
            EffectAction::Save => self.save_effect_preset(cx),
            _ => {
                let mut media = self.media.clone();
                let Some(item) = media.iter_mut().find(|item| item["id"] == self.selected) else {
                    return;
                };
                match action {
                    EffectAction::Add(id, after) => {
                        let mut effects = chain(item).to_vec();
                        let index =
                            after.map_or(effects.len(), |index| (index + 1).min(effects.len()));
                        effects.insert(index, json!({"effectId":id,"enabled":true}));
                        item["mediaEffects"] = json!(effects);
                    }
                    EffectAction::Remove(index) => {
                        let mut effects = chain(item).to_vec();
                        if *index >= effects.len() {
                            return;
                        }
                        effects.remove(*index);
                        item["mediaEffects"] = json!(effects);
                    }
                    EffectAction::Apply(id) => {
                        let Some(preset) = self
                            .effect_presets()
                            .iter()
                            .find(|preset| preset["id"] == *id)
                        else {
                            return;
                        };
                        apply_preset(item, preset);
                        item["mediaEffectPresetId"] = json!(id);
                    }
                    EffectAction::Clear | EffectAction::Unlink => {
                        if matches!(action, EffectAction::Clear) {
                            apply_preset(item, &Value::Null);
                        }
                        if let Some(item) = item.as_object_mut() {
                            item.remove("mediaEffectPresetId");
                        }
                    }
                    _ => unreachable!(),
                }
                self.inputs.retain(|key, _| !key.starts_with("effect:"));
                self.mutate("saveMediaSettings", vec![json!(media)], cx);
            }
        }
    }

    fn open_preset_name(
        &mut self,
        id: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let title = id
            .as_ref()
            .and_then(|id| {
                self.effect_presets()
                    .iter()
                    .find(|preset| preset["id"] == *id)
            })
            .map_or("", |preset| text(preset, "title"));
        let input = input_state(title.to_owned(), window, cx);
        let subscription =
            cx.subscribe_in(
                &input,
                window,
                |this, _, event: &InputEvent, _, cx| match event {
                    InputEvent::PressEnter { .. } => this.confirm_preset_dialog(cx),
                    InputEvent::Change => cx.notify(),
                    _ => {}
                },
            );
        self.effect_preset_dialog = Some(PresetDialog::Name {
            id,
            input,
            _subscription: subscription,
        });
        cx.notify();
    }

    fn preset_name_taken(&self, id: Option<&str>, title: &str) -> bool {
        self.effect_presets().iter().any(|preset| {
            preset["id"].as_str() != id
                && text(preset, "title").trim().to_lowercase() == title.to_lowercase()
        })
    }

    fn confirm_preset_dialog(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let mut presets = self.effect_presets().to_vec();
        let mut patch = json!({});
        match self.effect_preset_dialog.as_ref() {
            Some(PresetDialog::Name { id, input, .. }) => {
                let value = input.read(cx).value();
                let title = value.trim();
                if title.is_empty() || self.preset_name_taken(id.as_deref(), title) {
                    return;
                }
                if let Some(id) = id {
                    let Some(preset) = presets.iter_mut().find(|preset| preset["id"] == *id) else {
                        return;
                    };
                    preset["title"] = json!(title);
                } else {
                    let mut media = self.media.clone();
                    let Some(item) = media.iter_mut().find(|item| item["id"] == self.selected)
                    else {
                        return;
                    };
                    let id = uuid::Uuid::new_v4().to_string();
                    let mut preset = preset_payload(item);
                    preset["id"] = json!(id);
                    preset["title"] = json!(title);
                    presets.push(preset);
                    item["mediaEffectPresetId"] = json!(id);
                    patch["media"] = json!(media);
                }
            }
            Some(PresetDialog::Delete(id)) => {
                presets.retain(|preset| preset["id"] != *id);
                let mut media = self.media.clone();
                for item in media
                    .iter_mut()
                    .filter(|item| item["mediaEffectPresetId"] == *id)
                {
                    if let Some(item) = item.as_object_mut() {
                        item.remove("mediaEffectPresetId");
                    }
                }
                if media != self.media {
                    patch["media"] = json!(media);
                }
            }
            None => return,
        }
        patch["mediaEffectPresets"] = json!(presets);
        self.effect_preset_dialog = None;
        self.mutate("saveWorkspaceContent", vec![patch], cx);
    }

    fn save_effect_preset(&mut self, cx: &mut Context<Self>) {
        let Some(item) = self.media.iter().find(|item| item["id"] == self.selected) else {
            return;
        };
        let Some(preset) = self.effect_preset(item) else {
            return;
        };
        let next = preset_payload(item);
        let mut presets = self.effect_presets().to_vec();
        if let Some(saved) = presets.iter_mut().find(|saved| saved["id"] == preset["id"]) {
            saved["effects"] = next["effects"].clone();
            saved["mediaEffects"] = next["mediaEffects"].clone();
        }
        let mut media = self.media.clone();
        for linked in &mut media {
            if linked["mediaEffectPresetId"] == preset["id"] && matches_preset(linked, preset) {
                apply_preset(linked, &next);
            }
        }
        let mut patch = json!({"mediaEffectPresets":presets});
        if media != self.media {
            patch["media"] = json!(media);
        }
        self.mutate("saveWorkspaceContent", vec![patch], cx);
    }

    pub(super) fn effect_preset_dialog(&self, cx: &mut Context<Self>) -> Option<Dialog> {
        let t = *cx.global::<Translator>();
        let owner = cx.weak_entity();
        let close = move |_: &mut Window, cx: &mut gpui::App| {
            let _ = owner.update(cx, |this, cx| {
                this.effect_preset_dialog = None;
                cx.notify();
            });
        };
        let (title, confirm, disabled, danger) = match self.effect_preset_dialog.as_ref()? {
            PresetDialog::Name { id, input, .. } => {
                let value = input.read(cx).value();
                let title = value.trim();
                let action = t.t(if id.is_some() {
                    "common.rename"
                } else {
                    "media.effects.savePreset"
                });
                (
                    action.clone(),
                    action,
                    title.is_empty() || self.preset_name_taken(id.as_deref(), title),
                    false,
                )
            }
            PresetDialog::Delete(_) => (t.t("common.delete"), t.t("common.delete"), false, true),
        };
        let mut dialog = Dialog::new("media-effect-preset-dialog", title, t.t("common.cancel"), {
            let close = close.clone();
            move |_, window, cx| close(window, cx)
        })
        .width(cx.theme().metrics.control_height * 12.)
        .close_on_enter(false)
        .footer(
            ActionButton::new("effect-preset-cancel", t.t("common.cancel"))
                .variant(ButtonVariant::Secondary)
                .on_click(move |_, window, cx| close(window, cx)),
        )
        .footer(
            ActionButton::new("effect-preset-confirm", confirm)
                .when(danger, |button| button.variant(ButtonVariant::Danger))
                .disabled(self.busy || disabled)
                .on_click(cx.listener(|this, _, _, cx| this.confirm_preset_dialog(cx))),
        );
        match self.effect_preset_dialog.as_ref()? {
            PresetDialog::Name { id, input, .. } => {
                dialog = dialog
                    .initial_focus(input.read(cx).focus_handle(cx))
                    .child(Field::new(
                        t.t("media.effects.presetName"),
                        div()
                            .debug_selector(|| "effect-preset-name".into())
                            .child(TextInput::new("effect-preset-name", input)),
                    ));
                if self.preset_name_taken(id.as_deref(), input.read(cx).value().trim()) {
                    dialog = dialog.child(t.t("media.effects.duplicatePresetName"));
                }
            }
            PresetDialog::Delete(id) => {
                let preset = self
                    .effect_presets()
                    .iter()
                    .find(|preset| preset["id"] == *id)?;
                dialog = dialog.child(t.format(
                    "media.effects.deletePresetConfirm",
                    &[("title", text(preset, "title"))],
                ));
            }
        }
        Some(dialog)
    }
}

#[derive(Clone)]
pub(super) struct MediaPreviewFrame {
    pub key: Value,
    pub size: orbit_ui::FrameSize,
    pub image: std::sync::Arc<gpui::RenderImage>,
}

pub(super) fn visual_asset(item: &Value) -> Value {
    let Some(object) = item.as_object() else {
        return Value::Null;
    };
    Value::Object(
        object
            .iter()
            .filter(|(key, _)| {
                key.starts_with("media") && key.as_str() != "mediaEffectPresetId"
                    || matches!(
                        key.as_str(),
                        "id" | "type" | "sourcePath" | "sourceUrl" | "assetFile"
                    )
            })
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
    )
}

pub(super) struct ThumbnailCapture {
    pub id: String,
    pub frame: Option<MediaPreviewFrame>,
    seconds: f64,
}

impl MediaBin {
    pub(super) fn capture_thumbnail(&mut self, cx: &mut Context<Self>) {
        let seconds = self.preview_frame_time();
        let frame = self
            .rendered_media_preview
            .as_ref()
            .filter(|frame| {
                frame.key[0] == self.workspace
                    && (self.preview_playing || frame.key[2].as_f64() == Some(seconds))
                    && self
                        .media
                        .iter()
                        .find(|asset| asset["id"] == self.selected)
                        .is_some_and(|asset| frame.key[1] == visual_asset(asset))
            })
            .cloned();
        let seconds = frame
            .as_ref()
            .and_then(|frame| frame.key[2].as_f64())
            .unwrap_or(seconds);
        self.queued_thumbnail = Some(ThumbnailCapture {
            id: self.selected.clone(),
            frame,
            seconds,
        });
        self.flush_thumbnail(cx);
        cx.notify();
    }

    pub(super) fn flush_thumbnail(&mut self, cx: &mut Context<Self>) {
        if !self.busy {
            if let Some(capture) = self.queued_thumbnail.as_ref() {
                self.mutate_saved(
                    "setVideoThumbnail",
                    vec![json!(capture.id), json!(capture.seconds)],
                    cx,
                );
            }
        }
    }
}

impl MediaBin {
    pub(super) fn request_effect_preview(&mut self, item: &Value, cx: &mut Context<Self>) {
        let seconds = self.preview_frame_time();
        let item = visual_asset(item);
        let clock_revision = self
            .preview_playing
            .then(|| self.preview_generation.load(Ordering::Acquire));
        let key = json!([self.workspace, item, seconds, clock_revision]);
        if self.asset_preview_busy || self.effect_preview_key == key {
            return;
        }
        self.asset_preview_busy = true;
        self.effect_preview_key = key.clone();
        let host = self.runtime.host.clone();
        let workspace = self.workspace.clone();
        let stream = self.preview_stream.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let mut stream = stream
                        .lock()
                        .map_err(|_| "Inspector video decoder is unavailable.")?;
                    let image = host.render_media_asset_preview(
                        &workspace,
                        item,
                        seconds,
                        &mut stream,
                        clock_revision,
                    )?;
                    let size = orbit_ui::FrameSize {
                        width: image.width as u32,
                        height: image.height as u32,
                    };
                    let bytes = image.pixels;
                    orbit_ui::bgra_image(size, bytes).map(|image| (size, image))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.asset_preview_busy = false;
                if this.effect_preview_key != key {
                    cx.notify();
                    return;
                }
                match result {
                    Ok((size, image)) => {
                        this.rendered_media_preview = Some(MediaPreviewFrame {
                            key,
                            size,
                            image: image.clone(),
                        });
                        this.effect_preview.update(cx, |frame, cx| {
                            frame.set_source(Some(size), cx);
                            frame.set_image(image, cx);
                        });
                        cx.notify();
                    }
                    Err(error) => this.runtime.status.update(cx, |status, cx| {
                        status.message = error;
                        cx.notify();
                    }),
                }
            });
        })
        .detach();
    }
}
