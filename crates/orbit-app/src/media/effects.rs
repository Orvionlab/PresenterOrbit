use super::*;
use orbit_ui::controls::{CheckboxField, ColorPicker, Field};

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
        let chain = item["mediaEffects"].as_array().cloned().unwrap_or_default();
        let mut body = div()
            .flex()
            .flex_col()
            .gap(theme.metrics.space_xs)
            .child(PanelHeader::new(t.t("media.effectChain.title")));
        for (index, effect) in chain.iter().enumerate() {
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
            body = body.child(
                CheckboxField::new(
                    SharedString::from(format!("effect-enabled-{index}")),
                    title,
                    enabled,
                )
                .disabled(self.edits_blocked())
                .on_change(move |enabled, _, cx| {
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
                }),
            );
            let mut actions = div().flex().gap(theme.metrics.space_xs);
            for (offset, key) in [
                (-1, "media.effectChain.up"),
                (1, "media.effectChain.down"),
                (0, "media.effectChain.remove"),
            ] {
                actions = actions.child(
                    ActionButton::new(SharedString::from(format!("{key}-{index}")), t.t(key))
                        .variant(ButtonVariant::Ghost)
                        .disabled(
                            self.busy
                                || offset < 0 && index == 0
                                || offset > 0 && index + 1 == chain.len(),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let mut chain = this
                                .media
                                .iter()
                                .find(|item| item["id"] == this.selected)
                                .and_then(|item| item["mediaEffects"].as_array())
                                .cloned()
                                .unwrap_or_default();
                            if index >= chain.len() {
                                return;
                            }
                            if offset == 0 {
                                chain.remove(index);
                            } else if let Some(target) = index.checked_add_signed(offset) {
                                if target >= chain.len() {
                                    return;
                                }
                                chain.swap(index, target);
                            }
                            this.inputs.clear();
                            this.patch_asset("mediaEffects", json!(chain), cx);
                        })),
                );
            }
            body = body.child(actions);
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
                        body = body.child(self.numeric(
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
        let weak = cx.weak_entity();
        let options = std::iter::once(SelectOption::new(
            String::new(),
            t.t("media.effectChain.choose"),
        ))
        .chain(self.effect_catalog.iter().map(|entry| {
            SelectOption::new(
                text(entry, "id").to_string(),
                text(entry, "name").to_string(),
            )
        }))
        .collect();
        body = body.child(self.presets(item, window, cx));
        body.child(
            Select::new("effect-add", options, String::new())
                .compact()
                .disabled(self.edits_blocked())
                .on_change(move |id, _, cx| {
                    if id.is_empty() {
                        return;
                    }
                    let _ = weak.update(cx, |this, cx| {
                        let mut chain = this
                            .media
                            .iter()
                            .find(|item| item["id"] == this.selected)
                            .and_then(|item| item["mediaEffects"].as_array())
                            .cloned()
                            .unwrap_or_default();
                        chain.push(json!({"effectId":id,"enabled":true}));
                        this.patch_asset("mediaEffects", json!(chain), cx);
                    });
                }),
        )
        .into_any_element()
    }
}

impl MediaBin {
    fn presets(
        &mut self,
        item: &Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let t = *cx.global::<Translator>();
        let title = self
            .preset_title
            .get_or_insert_with(|| input_state(String::new(), window, cx))
            .clone();
        let presets = self.source["mediaEffectPresets"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let options = std::iter::once(SelectOption::new(
            String::new(),
            t.t("media.effects.noPreset"),
        ))
        .chain(presets.iter().map(|preset| {
            SelectOption::new(
                text(preset, "id").to_string(),
                text(preset, "title").to_string(),
            )
        }))
        .collect();
        let weak = cx.weak_entity();
        let mut body = div()
            .flex()
            .flex_col()
            .gap(cx.theme().metrics.space_xs)
            .child(Field::new(
                t.t("media.effects.preset"),
                Select::new("media-preset", options, self.preset_id.clone())
                    .disabled(self.edits_blocked())
                    .on_change(move |id, window, cx| {
                        let _ = weak.update(cx, |this, cx| {
                            this.preset_id = id.clone();
                            let preset = this.source["mediaEffectPresets"]
                                .as_array()
                                .and_then(|presets| {
                                    presets.iter().find(|preset| preset["id"] == id)
                                })
                                .cloned();
                            if let Some(preset) = preset {
                                this.preset_title = Some(input_state(
                                    text(&preset, "title").to_string(),
                                    window,
                                    cx,
                                ));
                                let mut media = this.media.clone();
                                if let Some(item) =
                                    media.iter_mut().find(|item| item["id"] == this.selected)
                                {
                                    if let Some(effects) = preset["effects"].as_object() {
                                        for (key, value) in effects {
                                            item[key] = value.clone();
                                        }
                                    }
                                    item["mediaEffects"] = preset
                                        .get("mediaEffects")
                                        .cloned()
                                        .unwrap_or_else(|| json!([]));
                                    this.inputs.clear();
                                    this.mutate("saveMediaSettings", vec![json!(media)], cx);
                                }
                            }
                            cx.notify();
                        });
                    }),
            ))
            .child(Field::new(
                t.t("media.effects.presetName"),
                TextInput::new("media-preset-title", &title).disabled(self.edits_blocked()),
            ));
        for (action, key) in [
            ("create", "media.effects.savePreset"),
            ("update", "media.effects.updatePreset"),
            ("delete", "common.delete"),
        ] {
            let item = item.clone();
            let title = title.clone();
            body = body.child(ActionButton::new(key,t.t(key)).disabled(self.busy || action != "create" && self.preset_id.is_empty()).on_click(cx.listener(move |this,_,_,cx| {
                let name = title.read(cx).value().to_string();
                if action != "delete" && name.trim().is_empty() { return; }
                let mut presets = this.source["mediaEffectPresets"].as_array().cloned().unwrap_or_default();
                let id = if action == "create" {uuid::Uuid::new_v4().to_string()} else {this.preset_id.clone()};
                let mut effects = json!({});
                for (key, neutral) in [("mediaHue",0.),("mediaSaturation",1.),("mediaBrightness",0.),("mediaContrast",1.)] {
                    effects[key] = item.get(key).cloned().unwrap_or_else(||json!(neutral));
                }
                if action == "delete" { presets.retain(|preset|preset["id"]!=id); }
                else {
                    let next = json!({"id":id,"title":name.trim(),"effects":effects,"mediaEffects":item.get("mediaEffects").cloned().unwrap_or_else(||json!([]))});
                    if action == "create" {presets.push(next);}
                    else if let Some(preset)=presets.iter_mut().find(|preset|preset["id"]==id) {*preset=next;} else {return;}
                }
                this.mutate("saveWorkspaceContent",vec![json!({"mediaEffectPresets":presets})],cx);
            })));
        }
        body.into_any_element()
    }
}

impl MediaBin {
    pub(super) fn request_effect_preview(&mut self, item: &Value, cx: &mut Context<Self>) {
        let seconds = self.preview_position;
        let key = json!([self.workspace, item, seconds]);
        if self.asset_preview_busy || self.effect_preview_key == key {
            return;
        }
        self.asset_preview_busy = true;
        self.effect_preview_key = key.clone();
        let host = self.runtime.host.clone();
        let workspace = self.workspace.clone();
        let id = self.selected.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let image = host.render_media_asset_preview(&workspace, &id, seconds)?;
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
                    Ok((size, image)) => this.effect_preview.update(cx, |frame, cx| {
                        frame.set_source(Some(size), cx);
                        frame.set_image(image, cx);
                    }),
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
