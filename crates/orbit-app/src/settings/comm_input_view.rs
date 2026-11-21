//! `CommunicationInputFields.tsx` and `ArtNetInputFields.tsx`: direction, deck settings, and Art-Net settings of an
//! input device. The device edits go through `devices_edit`.

use super::ui::Field;
use gpui::{prelude::*, AnyElement, Context, SharedString, Window};
use orbit_ui::controls::{CheckboxField, Select, SelectOption};
use orbit_ui::Translator;
use serde_json::{json, Value};

use super::protocols::with_input_protocol;
use super::ui::stack;
use super::view::SettingsView;

/// The four frame rate choices of a deck input: numerator, denominator, drop frame.
pub const RATES: [(&str, u64, u64, bool); 4] = [
    ("rate25", 25, 1, false),
    ("rate30", 30, 1, false),
    ("rate2997df", 30000, 1001, true),
    ("rate2997ndf", 30000, 1001, false),
];

impl SettingsView {
    pub(super) fn input_fields(
        &mut self,
        draft: &Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = *cx.global::<Translator>();
        let weak = cx.weak_entity();
        let id = draft["id"].as_str().unwrap_or("").to_string();
        let input = draft["input"].clone();
        let protocol = input["protocol"].as_str().unwrap_or("output").to_string();
        let direction_weak = weak.clone();
        let mut body = stack(cx).child(Field::new(
            t.t("settings.devices.direction"),
            Select::new(
                "device-direction",
                ["output", "rosstalk", "sony-bvw", "amp", "vdcp", "artnet"]
                    .iter()
                    .map(|p| {
                        SelectOption::new(
                            p.to_string(),
                            t.t(&format!("settings.devices.direction.{p}")),
                        )
                    })
                    .collect(),
                protocol.clone(),
            )
            .on_change(move |value, _, cx| {
                direction_weak
                    .update(cx, |v, cx| {
                        v.devices_edit(|d| *d = with_input_protocol(d, &value), cx)
                    })
                    .ok();
            }),
        ));
        if protocol == "artnet" {
            let fixture = !input["baseAddress"].is_null();
            let (map_weak, discovery_weak, reconnect_weak) =
                (weak.clone(), weak.clone(), weak.clone());
            body = body.child(self.number_field(
                &format!("device-artnet-universe-{id}"),
                t.t("settings.devices.artnetUniverse"),
                &input["universe"].as_u64().unwrap_or(0).to_string(),
                (0., 32767.),
                1.,
                window,
                cx,
                |v, text, cx| {
                    if let Ok(n) = text.trim().parse::<u64>() {
                        v.devices_edit(|d| d["input"]["universe"] = json!(n), cx)
                    }
                },
            ));
            body = body.child(Field::new(
                t.t("settings.devices.artnetMode"),
                Select::new(
                    "device-artnet-map",
                    vec![
                        SelectOption::new(
                            "control".to_string(),
                            t.t("settings.devices.artnetControlMode"),
                        ),
                        SelectOption::new(
                            "fixture".to_string(),
                            t.t("settings.devices.artnetFixtureMode"),
                        ),
                    ],
                    if fixture { "fixture" } else { "control" }.to_string(),
                )
                .on_change(move |value, _, cx| {
                    map_weak
                        .update(cx, |v, cx| {
                            v.devices_edit(
                                |d| {
                                    d["input"] = super::protocols::with_artnet_map(
                                        &d["input"],
                                        value == "fixture",
                                    )
                                },
                                cx,
                            )
                        })
                        .ok();
                }),
            ));
            let (key, label, max) = if fixture {
                ("baseAddress", "settings.devices.artnetBase", 496.)
            } else {
                ("controlChannel", "settings.devices.artnetChannel", 512.)
            };
            body = body.child(self.number_field(
                &format!("device-artnet-{key}-{id}"),
                t.t(label),
                &input[key].as_u64().unwrap_or(0).to_string(),
                (1., max),
                1.,
                window,
                cx,
                move |v, text, cx| {
                    if let Ok(n) = text.trim().parse::<u64>() {
                        v.devices_edit(|d| d["input"][key] = json!(n), cx)
                    }
                },
            ));
            body = body.child(
                CheckboxField::new(
                    SharedString::from(format!("device-artnet-reconnect-{id}")),
                    t.t("settings.devices.artnetReconnect"),
                    input["autoReconnect"] == json!(true),
                )
                .on_change(move |on, _, cx| {
                    reconnect_weak
                        .update(cx, |v, cx| {
                            v.devices_edit(|d| d["input"]["autoReconnect"] = json!(on), cx)
                        })
                        .ok();
                }),
            );
            let discovery = input.as_object().is_some_and(|o| o.contains_key("oemCode"));
            body = body.child(
                CheckboxField::new(
                    SharedString::from(format!("device-artnet-discovery-{id}")),
                    t.t("settings.devices.artnetDiscovery"),
                    discovery,
                )
                .on_change(move |on, _, cx| {
                    discovery_weak
                        .update(cx, |v, cx| {
                            v.devices_edit(
                                |d| {
                                    d["input"] =
                                        super::protocols::with_artnet_discovery(&d["input"], on)
                                },
                                cx,
                            )
                        })
                        .ok();
                }),
            );
            if discovery {
                body = body.child(
                    self.number_field(
                        &format!("device-artnet-oem-{id}"),
                        t.t("settings.devices.artnetOem"),
                        &input["oemCode"]
                            .as_u64()
                            .map(|n| n.to_string())
                            .unwrap_or_default(),
                        (0., 65535.),
                        1.,
                        window,
                        cx,
                        |v, text, cx| {
                            if let Ok(n) = text.trim().parse::<u64>() {
                                v.devices_edit(|d| d["input"]["oemCode"] = json!(n), cx)
                            }
                        },
                    ),
                );
            }
        } else if !input.is_null() && protocol != "rosstalk" {
            let current = format!(
                "{}/{}/{}",
                input["frameRate"]["numerator"],
                input["frameRate"]["denominator"],
                input["dropFrame"]
            );
            let rate_weak = weak.clone();
            body = body.child(Field::new(
                t.t("settings.devices.frameRate"),
                Select::new(
                    "device-frame-rate",
                    RATES
                        .iter()
                        .map(|(key, n, d, drop)| {
                            SelectOption::new(
                                format!("{n}/{d}/{drop}"),
                                t.t(&format!("settings.devices.{key}")),
                            )
                        })
                        .collect(),
                    current,
                )
                .on_change(move |value, _, cx| {
                    let parts: Vec<&str> = value.split('/').collect();
                    if let [n, d, drop] = parts[..] {
                        let (n, d, drop) = (
                            n.parse::<u64>().unwrap_or(25),
                            d.parse::<u64>().unwrap_or(1),
                            drop == "true",
                        );
                        rate_weak
                            .update(cx, |v, cx| {
                                v.devices_edit(
                                    |dev| {
                                        dev["input"]["frameRate"] =
                                            json!({"numerator": n, "denominator": d});
                                        dev["input"]["dropFrame"] = json!(drop);
                                    },
                                    cx,
                                )
                            })
                            .ok();
                    }
                }),
            ));
            if protocol == "vdcp" {
                for (key, label, min, max) in [
                    ("virtualPortId", "settings.devices.virtualPort", 1., 127.),
                    ("unitAddress", "settings.devices.unitAddress", 0., 15.),
                ] {
                    body = body.child(self.number_field(
                        &format!("device-vdcp-{key}-{id}"),
                        t.t(label),
                        &input[key].as_u64().unwrap_or(0).to_string(),
                        (min, max),
                        1.,
                        window,
                        cx,
                        move |v, text, cx| {
                            if let Ok(n) = text.trim().parse::<u64>() {
                                v.devices_edit(|d| d["input"][key] = json!(n), cx)
                            }
                        },
                    ));
                }
            }
            if protocol == "vdcp" || protocol == "amp" {
                let bins: Vec<(String, String)> = self.runtime.content.read(cx).workspace
                    ["mediaLibraries"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|b| {
                        (
                            b["id"].as_str().unwrap_or("").to_string(),
                            b["title"].as_str().unwrap_or("").to_string(),
                        )
                    })
                    .collect();
                let mut options = vec![SelectOption::new(
                    String::new(),
                    t.t("settings.devices.unfiled"),
                )];
                options.extend(bins.into_iter().map(|(i, l)| SelectOption::new(i, l)));
                let bin_weak = weak.clone();
                body = body.child(Field::new(
                    t.t("settings.devices.inputBin"),
                    Select::new(
                        "device-input-bin",
                        options,
                        input["mediaLibraryId"].as_str().unwrap_or("").to_string(),
                    )
                    .on_change(move |value, _, cx| {
                        bin_weak
                            .update(cx, |v, cx| {
                                v.devices_edit(|d| d["input"]["mediaLibraryId"] = json!(value), cx)
                            })
                            .ok();
                    }),
                ));
                let names_weak = weak.clone();
                body = body.child(
                    CheckboxField::new(
                        SharedString::from(format!("device-short-names-{id}")),
                        t.t("settings.devices.clipNames"),
                        input["shortNames"] == json!(true),
                    )
                    .on_change(move |on, _, cx| {
                        names_weak
                            .update(cx, |v, cx| {
                                v.devices_edit(|d| d["input"]["shortNames"] = json!(on), cx)
                            })
                            .ok();
                    }),
                );
            }
        }
        body.into_any_element()
    }
}
