use super::*;
use orbit_ui::controls::{ContextMenu, MenuEntry, MenuItem};

impl MediaBin {
    /// Nothing is copied, or the copied media has no playlist to be pasted into.
    pub(super) fn paste_blocked(&self) -> bool {
        self.clipboard.as_ref().is_none_or(|item| {
            matches!(item["type"].as_str(), Some("image" | "video" | "audio"))
                && !model::has_playlist(&self.libraries, item["type"] == "audio")
        })
    }
    pub(super) fn menu_action(&mut self, action: &str, cx: &mut Context<Self>) {
        self.menu_position = None;
        let item = self
            .media
            .iter()
            .chain(self.rows.iter())
            .find(|item| item["id"] == self.selected)
            .cloned();
        if action == "paste" && self.paste_blocked() {
            return;
        }
        if action == "copy" || action == "cut" {
            self.clipboard = item.clone();
        }
        if action == "copy" {
            cx.notify();
            return;
        }
        if item
            .as_ref()
            .is_some_and(|item| item["type"] == "background")
            || action == "paste"
                && self
                    .clipboard
                    .as_ref()
                    .is_some_and(|item| item["type"] == "background")
        {
            let mut backgrounds = self.backgrounds.clone();
            match action {
                "cut" | "delete" => backgrounds.retain(|item| item["id"] != self.selected),
                "duplicate" | "paste" => {
                    let Some(mut source) = (if action == "paste" {
                        self.clipboard.clone()
                    } else {
                        item
                    }) else {
                        return;
                    };
                    source["id"] = json!(uuid::Uuid::new_v4().to_string());
                    source["name"] = json!(crate::collections::model::copy_name(
                        text(&source, "name"),
                        &backgrounds
                            .iter()
                            .map(|item| text(item, "name"))
                            .collect::<Vec<_>>()
                    ));
                    if let Some(object) = source.as_object_mut() {
                        object.remove("type");
                        object.remove("title");
                    }
                    backgrounds.push(source);
                }
                _ => return,
            }
            self.runtime.output.update(cx, |output, cx| {
                output.edit_live(|live| live["backgrounds"] = json!(backgrounds));
                cx.notify();
            });
            return;
        }
        let mut media = self.media.clone();
        match action {
            "cut" | "delete" => media.retain(|item| item["id"] != self.selected),
            "paste" | "duplicate" => {
                let source = if action == "paste" {
                    self.clipboard.clone()
                } else {
                    item
                };
                let Some(mut source) = source else {
                    return;
                };
                source["title"] = json!(crate::collections::model::copy_name(
                    text(&source, "title"),
                    &media
                        .iter()
                        .map(|item| text(item, "title"))
                        .collect::<Vec<_>>()
                ));
                source["id"] = json!(uuid::Uuid::new_v4().to_string());
                media.push(source);
            }
            "open-location" => {
                let Some(path) =
                    item.and_then(|item| item["sourcePath"].as_str().map(str::to_string))
                else {
                    return;
                };
                let host = self.runtime.host.clone();
                cx.spawn(async move |this, cx| {
                    let result = cx
                        .background_spawn(async move {
                            host.invoke("showItemInFolder", vec![json!(path)])
                        })
                        .await;
                    let _ = this.update(cx, |this, cx| {
                        if let Err(error) = result {
                            this.runtime.status.update(cx, |status, cx| {
                                status.message = error;
                                cx.notify();
                            });
                        }
                    });
                })
                .detach();
                return;
            }
            _ => return,
        }
        self.mutate("saveMediaSettings", vec![json!(media)], cx);
    }

    pub(super) fn menu(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let position = self.menu_position?;
        let t = *cx.global::<Translator>();
        let item = self
            .media
            .iter()
            .chain(self.rows.iter())
            .find(|item| item["id"] == self.selected)?;
        let mut entries: Vec<MenuEntry> = Vec::new();
        let weak = cx.weak_entity();
        entries.push(
            MenuItem::new(t.t("common.rename"))
                .disabled(self.busy)
                .on_click(move |_, window, cx| {
                    let _ = weak.update(cx, |this, cx| {
                        if let Some(item) = this
                            .media
                            .iter()
                            .chain(this.rows.iter())
                            .find(|item| item["id"] == this.selected)
                        {
                            this.rename =
                                Some(input_state(text(item, "title").to_string(), window, cx));
                        }
                        this.menu_position = None;
                        cx.notify();
                    });
                })
                .into(),
        );
        let weak = cx.weak_entity();
        entries.push(
            MenuItem::new(t.t("media.action.inspector"))
                .disabled(item["type"] == "background")
                .on_click(move |_, _, cx| {
                    let _ = weak.update(cx, |this, cx| {
                        this.menu_position = None;
                        this.inspector_open = true;
                        cx.notify();
                    });
                })
                .into(),
        );
        for (action, key) in [
            ("cut", "common.cut"),
            ("copy", "common.copy"),
            ("paste", "common.paste"),
            ("duplicate", "common.duplicate"),
            ("delete", "common.delete"),
            ("open-location", "media.action.openFileLocation"),
        ] {
            if item["type"] == "audio" && !matches!(action, "delete" | "open-location") {
                continue;
            }
            let weak = cx.weak_entity();
            let mut entry = MenuItem::new(t.t(key));
            if action == "delete" {
                entry = entry.meta("Delete");
            }
            entries.push(
                entry
                    .disabled(
                        self.busy
                            || action == "paste" && self.paste_blocked()
                            || action == "open-location" && item["sourcePath"].is_null(),
                    )
                    .on_click(move |_, _, cx| {
                        let _ = weak.update(cx, |this, cx| this.menu_action(action, cx));
                    })
                    .into(),
            );
        }
        let live = self.runtime.output.read(cx).live.clone();
        if item["type"] == "image" || item["type"] == "video" {
            for (field, label, options) in [
                (
                    "backgroundScale",
                    "media.scaling",
                    vec![
                        ("fit", "media.scale.fit"),
                        ("fill", "media.scale.fill"),
                        ("stretch", "media.scale.stretch"),
                        ("blur", "media.scale.blur"),
                    ],
                ),
                (
                    "backgroundBehavior",
                    "media.behavior",
                    vec![
                        ("background", "media.behavior.background"),
                        ("foreground", "media.behavior.foreground"),
                    ],
                ),
                (
                    "backgroundTransition",
                    "media.transitions",
                    vec![
                        ("none", "media.transition.layerDefault"),
                        ("dissolve", "media.transition.dissolve"),
                        ("cut", "media.transition.cut"),
                    ],
                ),
                (
                    "backgroundPlayback",
                    "media.playbackBehavior",
                    vec![
                        ("stop", "media.playback.stop"),
                        ("loop", "media.playback.loop"),
                    ],
                ),
                (
                    "backgroundEndBehavior",
                    "media.endBehavior",
                    vec![
                        ("hold", "media.end.hold"),
                        ("black", "media.end.black"),
                        ("clear", "media.end.clear"),
                        ("fade-black", "media.end.fadeBlack"),
                        ("fade-clear", "media.end.fadeClear"),
                    ],
                ),
            ] {
                if item["type"] != "video"
                    && matches!(field, "backgroundPlayback" | "backgroundEndBehavior")
                {
                    continue;
                }
                let current = option_value(item, &live, field);
                let choices = options
                    .into_iter()
                    .map(|(value, label)| {
                        let weak = cx.weak_entity();
                        MenuItem::new(t.t(label))
                            .checked(current == value)
                            .disabled(self.busy)
                            .on_click(move |_, _, cx| {
                                let _ = weak.update(cx, |this, cx| {
                                    this.menu_position = None;
                                    let id = this.selected.clone();
                                    this.take(&id, cx);
                                    this.runtime.output.update(cx, |output, cx| {
                                        output.edit_live(|live| live[field] = json!(value));
                                        cx.notify();
                                    });
                                    this.patch_asset(field, json!(value), cx);
                                });
                            })
                            .into()
                    })
                    .collect();
                entries.push(MenuEntry::Submenu {
                    label: t.t(label).into(),
                    disabled: self.busy,
                    entries: choices,
                });
            }
        }
        if item["type"] == "audio" {
            let current = model::audio_behavior(item, &self.libraries, &self.audio_library);
            let choices = [
                ("stop", "media.playback.stop"),
                ("loop", "media.playback.loop"),
                ("next", "media.playback.next"),
            ]
            .into_iter()
            .map(|(value, key)| {
                let weak = cx.weak_entity();
                MenuItem::new(t.t(key))
                    .checked(current == value)
                    .disabled(self.busy)
                    .on_click(move |_, _, cx| {
                        let _ = weak.update(cx, |this, cx| {
                            this.menu_position = None;
                            this.patch_asset("audioPlayback", json!(value), cx);
                        });
                    })
                    .into()
            })
            .collect();
            let mut existing = entries.into_iter();
            let rename = existing.next();
            let inspector = existing.next();
            let delete = existing.next();
            let location = existing.next();
            entries = location
                .into_iter()
                .chain([MenuEntry::Submenu {
                    label: t.t("media.playbackBehavior").into(),
                    disabled: self.busy,
                    entries: choices,
                }])
                .chain(inspector)
                .chain(rename)
                .chain(delete)
                .collect();
        }
        let weak = cx.weak_entity();
        Some(
            ContextMenu::new("media-context", position, entries, move |_, _, cx| {
                let _ = weak.update(cx, |this, cx| {
                    this.menu_position = None;
                    cx.notify();
                });
            })
            .into_any_element(),
        )
    }
}

/// Mirrors React's menu defaults: saved asset value, else the live value while this asset is on air, else the default.
pub(super) fn option_value(item: &Value, live: &Value, field: &str) -> String {
    let default = match field {
        "backgroundScale" => "fit",
        "backgroundBehavior" => "foreground",
        "backgroundTransition" => "none",
        "backgroundPlayback" => "stop",
        _ => "hold",
    };
    let value = if field == "backgroundBehavior" {
        // React only distinguishes "background"; any other saved value reads as foreground.
        return if item[field] == "background" {
            "background"
        } else {
            default
        }
        .into();
    } else if let Some(value) = item[field].as_str() {
        value
    } else if live["backgroundId"] == item["id"] {
        live[field].as_str().unwrap_or(default)
    } else {
        default
    };
    value.into()
}
