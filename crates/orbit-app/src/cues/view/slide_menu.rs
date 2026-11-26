use super::*;

pub enum SlideMenuRequest {
    Actions {
        item: String,
        slide: String,
        section: Option<&'static str>,
    },
    InspectMedia(String),
}

impl gpui::EventEmitter<SlideMenuRequest> for CueView {}

const BEHAVIORS: [(&str, &str); 2] = [
    ("background", "media.behavior.background"),
    ("foreground", "media.behavior.foreground"),
];
const SCALES: [(&str, &str); 4] = [
    ("fit", "media.scale.fit"),
    ("fill", "media.scale.fill"),
    ("stretch", "media.scale.stretch"),
    ("blur", "media.scale.blur"),
];

#[derive(Clone)]
struct SlideMedia {
    asset: Value,
    action_id: Option<String>,
}

impl SlideMedia {
    fn details<'a>(&self, slide: &'a Value) -> Option<&'a Value> {
        let id = self.action_id.as_ref()?;
        slide["actions"]
            .as_array()?
            .iter()
            .find(|action| action["id"] == *id)
            .map(|action| &action["details"])
    }

    fn remove(&self, slide: &mut Value) {
        if let Some(id) = &self.action_id {
            if let Some(actions) = slide["actions"].as_array_mut() {
                actions.retain(|action| action["id"] != *id);
            }
        } else if let Some(slide) = slide.as_object_mut() {
            slide.remove("mediaBackgroundId");
            slide.remove("mediaBackgroundOnly");
            slide.remove("mediaBackgroundBehavior");
            slide.remove("mediaBackgroundScale");
        }
    }

    fn set_option(&self, slide: &mut Value, field: &str, value: &str) {
        if let Some(id) = &self.action_id {
            if let Some(action) = slide["actions"]
                .as_array_mut()
                .and_then(|actions| actions.iter_mut().find(|action| action["id"] == *id))
            {
                if action["details"].is_null() {
                    action["details"] = json!({});
                }
                if let Some(details) = action["details"].as_object_mut() {
                    details.insert(field.into(), json!(value));
                }
            }
        } else if slide["mediaBackgroundId"] == self.asset["id"] {
            if field == "backgroundBehavior" {
                slide["mediaBackgroundOnly"] = json!(value == "background");
                slide["mediaBackgroundBehavior"] = json!(value);
            } else {
                slide["mediaBackgroundScale"] = json!(value);
            }
        }
    }

    fn scale<'a>(&'a self, slide: &'a Value) -> &'a str {
        let value = if self.action_id.is_some() {
            self.details(slide)
                .and_then(|details| details["backgroundScale"].as_str())
        } else {
            slide["mediaBackgroundScale"].as_str()
        };
        value
            .or(self.asset["backgroundScale"].as_str())
            .unwrap_or("fit")
    }

    fn behavior<'a>(&'a self, slide: &'a Value) -> &'a str {
        let value = if self.action_id.is_some() {
            self.details(slide)
                .and_then(|details| details["backgroundBehavior"].as_str())
        } else if slide["mediaBackgroundOnly"] == true {
            Some("background")
        } else {
            slide["mediaBackgroundBehavior"].as_str()
        };
        value
            .or(self.asset["backgroundBehavior"].as_str())
            .unwrap_or("foreground")
    }

    fn convert(&self, slide: &mut Value) {
        let Some(source) = self.asset["sourcePath"].as_str() else {
            return;
        };
        let size = crate::canvas::slide_size(slide);
        let media_scale = match self.scale(slide) {
            "fill" => "Scale to Fill",
            "stretch" => "Stretch to Fill",
            _ => "Scale to Fit",
        };
        let layer = json!({
            "id": uuid::Uuid::new_v4().to_string(),
            "type":"Shape", "shape":"rectangle", "fillType":"media", "fill":"Media",
            "mediaType":self.asset["type"], "mediaTitle":self.asset["title"],
            "sourcePath":source, "mediaUrl":self.asset["sourceUrl"], "mediaScale":media_scale,
            "bounds":{"x":0,"y":0,"width":size.x,"height":size.y},
        });
        if slide["layers"].is_null() {
            slide["layers"] = json!([]);
        }
        if let Some(layers) = slide["layers"].as_array_mut() {
            layers.insert(0, layer);
            self.remove(slide);
        }
    }
}

fn media_assets(workspace: &Value) -> impl Iterator<Item = &Value> {
    workspace["media"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|asset| matches!(asset["type"].as_str(), Some("image" | "video" | "audio")))
}

fn edit_slides(runtime: &Runtime, item_id: &str, cx: &mut App, edit: impl FnOnce(&mut [Value])) {
    let Some(item) = runtime.content.read(cx).items.get(item_id) else {
        return;
    };
    let Some(before) = item.value["slides"].as_array() else {
        return;
    };
    let mut slides = before.clone();
    edit(&mut slides);
    if slides == *before {
        return;
    }
    let id = item_id.to_owned();
    let mutation = json!({"type":"mutateItem","itemId":id,"operation":{
        "type":"setSlideDocument","slides":slides,"expectedSlides":before
    }});
    runtime
        .save_preview(cx, Kind::Library, mutation, move |content| {
            crate::collections::arrangements::edit_item(content, &id, |item| {
                item["slides"] = json!(slides)
            });
        })
        .detach();
}

fn edit_slide(
    runtime: &Runtime,
    item_id: &str,
    slide_id: &str,
    cx: &mut App,
    edit: impl FnOnce(&mut Value),
) {
    edit_slides(runtime, item_id, cx, |slides| {
        if let Some(slide) = slides.iter_mut().find(|slide| slide["id"] == slide_id) {
            edit(slide);
        }
    });
}

impl CueView {
    pub(super) fn open_slide_transition(
        &mut self,
        item: String,
        slide: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let runtime = self.runtime.clone();
        self.slide_transition = Some(cx.new(|cx| {
            let mut picker = crate::transitions::TransitionDefaults::new(
                crate::transitions::Kind::Slide,
                runtime,
                cx,
            );
            picker.open_slide(item, slide, window, cx);
            picker
        }));
    }

    pub(super) fn slide_transition_element(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let picker = self.slide_transition.clone()?;
        let t = *cx.global::<Translator>();
        let owner = cx.weak_entity();
        Some(
            Dialog::new(
                "slide-transition-dialog",
                t.t("transition.title"),
                t.t("common.close"),
                move |_, _, cx| {
                    let _ = owner.update(cx, |view, cx| {
                        view.slide_transition = None;
                        cx.notify();
                    });
                },
            )
            .close_on_enter(false)
            .flush()
            .width(cx.theme().metrics.control_height * 30.)
            .child(picker)
            .into_any_element(),
        )
    }

    pub(super) fn copy_slide_style(&mut self, item: &Item, id: &str) {
        let Some(slide) = item.value["slides"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|slide| slide["id"] == id)
        else {
            return;
        };
        let layer = slide["layers"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|layer| crate::canvas::editing::is_text(layer));
        let mut style = crate::canvas::editing::style(slide, layer);
        if let Some(style) = style.as_object_mut() {
            style.retain(|key, _| !key.starts_with("linkedText"));
        }
        self.text_style_clipboard = Some(style);
    }

    pub(super) fn paste_slide_style(&self, item_id: &str, ids: &[String], cx: &mut App) {
        let Some(style) = &self.text_style_clipboard else {
            return;
        };
        edit_slides(&self.runtime, item_id, cx, |slides| {
            for slide in slides
                .iter_mut()
                .filter(|slide| ids.iter().any(|id| slide["id"] == *id))
            {
                let target = slide["layers"]
                    .as_array()
                    .and_then(|layers| layers.iter().position(crate::canvas::editing::is_text));
                let text_style = match target {
                    Some(index) => &mut slide["layers"][index]["textStyle"],
                    None => &mut slide["textStyle"],
                };
                let mut updated = style.clone();
                if let Some(existing) = text_style.as_object() {
                    for (key, value) in existing
                        .iter()
                        .filter(|(key, _)| key.starts_with("linkedText"))
                    {
                        updated[key] = value.clone();
                    }
                }
                *text_style = updated;
            }
        });
    }

    pub(super) fn slide_action_entries(
        &self,
        item: &Item,
        cue: &Value,
        cx: &Context<Self>,
    ) -> Vec<MenuEntry> {
        let Some(slide_id) = cue["id"].as_str() else {
            return Vec::new();
        };
        let slide_id = slide_id.to_owned();
        let slide = item.value["slides"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|slide| slide["id"] == slide_id)
            .unwrap_or(cue);
        let t = *cx.global::<Translator>();
        let workspace = &self.runtime.content.read(cx).workspace;
        let action_entry = |label, section| {
            let (owner, item, slide) = (cx.weak_entity(), item.id.clone(), slide_id.clone());
            MenuItem::new(label)
                .on_click(move |_, _, cx| {
                    let _ = owner.update(cx, |_, cx| {
                        cx.emit(SlideMenuRequest::Actions {
                            item: item.clone(),
                            slide: slide.clone(),
                            section,
                        })
                    });
                })
                .into()
        };
        let categories = [
            ("clear", "automation.macros.addActionCategory.clear"),
            (
                "audience-look",
                "automation.macros.addActionCategory.audience-look",
            ),
            ("stage", "automation.macros.addActionCategory.stage"),
            ("timer", "automation.macros.addActionCategory.timer"),
            ("prop", "automation.macros.addActionCategory.prop"),
            ("message", "automation.macros.addActionCategory.message"),
            ("playlist", "cue.menu.playlistAction"),
            (
                "communication",
                "automation.macros.addActionCategory.communication",
            ),
            (
                "audio-input",
                "automation.macros.addActionCategory.audio-input",
            ),
            (
                "video-input",
                "automation.macros.addActionCategory.video-input",
            ),
            ("macro", "automation.macros.addActionCategory.macro"),
            ("capture", "automation.macros.addActionCategory.capture"),
        ];
        let mut categories: Vec<_> = categories
            .into_iter()
            .map(|(kind, label)| action_entry(t.t(label), Some(kind)))
            .collect();
        categories.push(MenuEntry::Separator);
        categories.push(action_entry(t.t("cue.menu.actionPalette"), None));
        let media_choices: Vec<_> = media_assets(workspace)
            .filter_map(|asset| {
                let id = asset["id"].as_str()?.to_owned();
                let name = asset["title"].as_str().unwrap_or(&id).to_owned();
                let (runtime, item, slide) =
                    (self.runtime.clone(), item.id.clone(), slide_id.clone());
                Some(
                    MenuItem::new(name)
                        .on_click(move |_, _, cx| {
                            let id = id.clone();
                            edit_slide(&runtime, &item, &slide, cx, |slide| {
                                if slide["actions"].is_null() {
                                    slide["actions"] = json!([]);
                                }
                                if let Some(actions) = slide["actions"].as_array_mut() {
                                    actions.push(json!({
                            "id":uuid::Uuid::new_v4().to_string(),"type":"media","targetId":id,
                            "enabled":true,"delaySeconds":0
                        }));
                                }
                            });
                        })
                        .into(),
                )
            })
            .collect();
        let mut entries = vec![
            MenuEntry::Submenu {
                label: t.t("automation.macros.addActionMenu").into(),
                disabled: false,
                entries: categories,
            },
            MenuEntry::Submenu {
                label: t.t("cue.menu.addMediaAction").into(),
                disabled: media_choices.is_empty(),
                entries: media_choices,
            },
        ];
        let background = slide["mediaBackgroundId"].as_str().map(|id| (id, None));
        let actions = slide["actions"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|action| action["type"] == "media")
            .filter_map(|action| {
                Some((
                    action["targetId"].as_str()?,
                    Some(action["id"].as_str()?.to_owned()),
                ))
            });
        for (id, action_id) in background.into_iter().chain(actions) {
            if let Some(asset) =
                media_assets(workspace).find(|asset| asset["id"] == id || asset["sourceId"] == id)
            {
                let media = SlideMedia {
                    asset: asset.clone(),
                    action_id,
                };
                self.add_media_entries(&mut entries, item, slide, media, cx);
            }
        }
        entries
    }

    fn add_media_entries(
        &self,
        entries: &mut Vec<MenuEntry>,
        item: &Item,
        slide: &Value,
        media: SlideMedia,
        cx: &Context<Self>,
    ) {
        let Some(slide_id) = slide["id"].as_str() else {
            return;
        };
        let Some(asset_id) = media.asset["id"].as_str() else {
            return;
        };
        let t = *cx.global::<Translator>();
        let path = media.asset["sourcePath"].as_str();
        let name = path
            .and_then(|path| std::path::Path::new(path).file_name())
            .and_then(|name| name.to_str())
            .or_else(|| media.asset["title"].as_str())
            .unwrap_or(asset_id);
        let asset_id = asset_id.to_owned();
        let edit = |label, change: fn(&SlideMedia, &mut Value), disabled| {
            let (runtime, item, slide, media) = (
                self.runtime.clone(),
                item.id.clone(),
                slide_id.to_owned(),
                media.clone(),
            );
            MenuItem::new(label)
                .disabled(disabled)
                .on_click(move |_, _, cx| {
                    edit_slide(&runtime, &item, &slide, cx, |slide| change(&media, slide));
                })
                .into()
        };
        let remove = || {
            edit(
                t.format("cue.menu.removeAction", &[("name", name)]),
                SlideMedia::remove,
                false,
            )
        };
        let inspect = || {
            let (owner, id) = (cx.weak_entity(), asset_id.clone());
            MenuItem::new(t.t("media.action.inspector"))
                .on_click(move |_, _, cx| {
                    let _ = owner.update(cx, |_, cx| {
                        cx.emit(SlideMenuRequest::InspectMedia(id.clone()))
                    });
                })
                .into()
        };
        let options =
            |label, field: &'static str, current: &str, choices: &[(&'static str, &str)]| {
                let entries = choices
                    .iter()
                    .map(|&(value, label)| {
                        let (runtime, item, slide, media) = (
                            self.runtime.clone(),
                            item.id.clone(),
                            slide_id.to_owned(),
                            media.clone(),
                        );
                        MenuItem::new(t.t(label))
                            .checked(value == current)
                            .on_click(move |_, _, cx| {
                                edit_slide(&runtime, &item, &slide, cx, |slide| {
                                    media.set_option(slide, field, value)
                                });
                            })
                            .into()
                    })
                    .collect();
                MenuEntry::Submenu {
                    label: t.t(label).into(),
                    disabled: false,
                    entries,
                }
            };
        let mut submenu = vec![inspect(), MenuEntry::Separator];
        if media.asset["type"] != "audio" {
            submenu.extend([
                options(
                    "media.behavior",
                    "backgroundBehavior",
                    media.behavior(slide),
                    &BEHAVIORS,
                ),
                options(
                    "media.scaling",
                    "backgroundScale",
                    media.scale(slide),
                    &SCALES,
                ),
                MenuEntry::Separator,
                edit(
                    t.t("cue.menu.convertMediaAction"),
                    SlideMedia::convert,
                    path.is_none() || media.scale(slide) == "blur",
                ),
            ]);
        }
        let runtime = self.runtime.clone();
        let path = path.map(str::to_owned);
        submenu.push(
            MenuItem::new(t.t("media.action.openFileLocation"))
                .disabled(path.is_none())
                .on_click(move |_, _, cx| {
                    if let Some(path) = &path {
                        crate::media::reveal_asset(&runtime, path.clone(), cx);
                    }
                })
                .into(),
        );
        submenu.extend([MenuEntry::Separator, remove()]);
        entries.extend([
            MenuEntry::Separator,
            MenuEntry::Submenu {
                label: t.format("cue.menu.editAction", &[("name", name)]).into(),
                disabled: false,
                entries: submenu,
            },
            remove(),
            MenuEntry::Separator,
            inspect(),
        ]);
    }
}
