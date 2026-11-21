use super::*;
use orbit_ui::controls::CheckboxField;

pub(super) fn asset_route(item: &Value) -> Value {
    item.get("audioRouting").cloned().unwrap_or_else(||json!({"gainDb":0,"channels":[{"sourceChannel":1,"outputs":[1],"muted":false},{"sourceChannel":2,"outputs":[2],"muted":false}]}))
}
impl MediaBin {
    pub(super) fn route_field(&mut self, key: &str, value: Value, cx: &mut Context<Self>) {
        let Some(item) = self.media.iter().find(|item| item["id"] == self.selected) else {
            return;
        };
        let mut route = asset_route(item);
        if key == "route:gainDb" {
            route["gainDb"] = value;
        } else {
            let Some((index, field)) = key
                .strip_prefix("route:")
                .and_then(|key| key.split_once(':'))
            else {
                return;
            };
            let Some(channel) = index.parse::<usize>().ok().and_then(|index| {
                route["channels"]
                    .as_array_mut()
                    .and_then(|channels| channels.get_mut(index))
            }) else {
                return;
            };
            channel[field] = value;
        }
        self.patch_asset("audioRouting", route, cx);
    }
    pub(super) fn routing(
        &mut self,
        item: &Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let t = *cx.global::<Translator>();
        let theme = *cx.theme();
        let route = asset_route(item);
        let mut body = div()
            .flex()
            .flex_col()
            .gap(theme.metrics.space_xs)
            .child(PanelHeader::new(t.t("media.audioRouting.title")));
        body = body.child(self.numeric(
            "route:gainDb",
            "media.audioRouting.gain",
            route["gainDb"].as_f64().unwrap_or(0.),
            (-60., 12.),
            window,
            cx,
        ));
        let Some(channels) = route["channels"].as_array() else {
            return body.into_any_element();
        };
        for (index, channel) in channels.iter().enumerate() {
            body = body.child(div().child(format!(
                "{} {}",
                t.t("media.audioRouting.input"),
                channel["sourceChannel"]
            )));
            for output in [1, 2] {
                let checked = channel["outputs"]
                    .as_array()
                    .is_some_and(|outputs| outputs.contains(&json!(output)));
                let weak = cx.weak_entity();
                body = body.child(
                    CheckboxField::new(
                        SharedString::from(format!("route-{index}-output-{output}")),
                        format!("{} {output}", t.t("media.audioRouting.output")),
                        checked,
                    )
                    .disabled(self.edits_blocked())
                    .on_change(move |checked, _, cx| {
                        let _ = weak.update(cx, |this, cx| {
                            let Some(item) =
                                this.media.iter().find(|item| item["id"] == this.selected)
                            else {
                                return;
                            };
                            let route = asset_route(item);
                            let Some(channel) = route["channels"].get(index) else {
                                return;
                            };
                            let Some(mut outputs) = channel["outputs"].as_array().cloned() else {
                                return;
                            };
                            outputs.retain(|value| *value != json!(output));
                            if checked {
                                outputs.push(json!(output));
                            }
                            outputs.sort_by_key(|value| value.as_u64());
                            this.route_field(&format!("route:{index}:outputs"), json!(outputs), cx);
                        });
                    }),
                );
            }
            body = body.child(self.numeric(
                &format!("route:{index}:gainDb"),
                "media.audioRouting.gain",
                channel["gainDb"].as_f64().unwrap_or(0.),
                (-60., 12.),
                window,
                cx,
            ));
            for (flag, label) in [
                ("muted", "media.audioRouting.mute"),
                ("solo", "theme.media.solo"),
                ("compressLimit", "media.audioRouting.compressLimit"),
            ] {
                let weak = cx.weak_entity();
                body = body.child(
                    CheckboxField::new(
                        SharedString::from(format!("route-{index}-{flag}")),
                        t.t(label),
                        channel[flag] == true,
                    )
                    .disabled(self.edits_blocked())
                    .on_change(move |checked, _, cx| {
                        let _ = weak.update(cx, |this, cx| {
                            this.route_field(&format!("route:{index}:{flag}"), json!(checked), cx)
                        });
                    }),
                );
            }
            if channels.len() > 1 {
                body = body.child(
                    ActionButton::new(
                        SharedString::from(format!("route-remove-{index}")),
                        t.t("media.audioRouting.removeChannel"),
                    )
                    .disabled(self.edits_blocked())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let Some(item) = this.media.iter().find(|item| item["id"] == this.selected)
                        else {
                            return;
                        };
                        let mut route = asset_route(item);
                        let Some(channels) = route["channels"].as_array_mut() else {
                            return;
                        };
                        if channels.len() <= 1 || index >= channels.len() {
                            return;
                        }
                        channels.remove(index);
                        this.inputs.clear();
                        this.patch_asset("audioRouting", route, cx);
                    })),
                );
            }
        }
        let next = (1..=8).find(|source| {
            !channels
                .iter()
                .any(|channel| channel["sourceChannel"] == *source)
        });
        if let Some(source) = next {
            body = body.child(
                ActionButton::new("route-add", t.t("media.audioRouting.addChannel"))
                    .disabled(self.edits_blocked())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let Some(item) = this.media.iter().find(|item| item["id"] == this.selected)
                        else {
                            return;
                        };
                        let mut route = asset_route(item);
                        let Some(channels) = route["channels"].as_array_mut() else {
                            return;
                        };
                        if channels.len() >= 8 {
                            return;
                        }
                        channels.push(json!({"sourceChannel":source,"outputs":[],"muted":false}));
                        this.patch_asset("audioRouting", route, cx);
                    })),
            );
        }
        body.into_any_element()
    }
}
