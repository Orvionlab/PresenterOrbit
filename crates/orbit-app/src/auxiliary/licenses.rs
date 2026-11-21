use std::collections::HashMap;
use std::sync::Arc;

use gpui::{div, prelude::*, Context, Entity, Render, SharedString, Subscription, Window};
use orbit_host::Host;
use orbit_ui::controls::{
    filter_state, list_row, ActionButton, ButtonVariant, EmptyState, FilterInput, InputEvent,
    InputState, NativeButton,
};
use orbit_ui::{ActiveTheme, Translator};
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
struct File {
    name: String,
    #[serde(rename = "textId")]
    text_id: String,
}

#[derive(Deserialize)]
struct Dependency {
    name: String,
    version: String,
    license: Option<String>,
    repository: Option<String>,
    files: Vec<File>,
}

#[derive(Deserialize)]
struct Catalog {
    packages: Vec<Dependency>,
    texts: HashMap<String, String>,
}

fn catalog() -> Result<Catalog, String> {
    let mut catalog: Catalog = serde_json::from_str(include_str!(concat!(
        env!("OUT_DIR"),
        "/dependency-notices.json"
    )))
    .map_err(|error| format!("Invalid bundled dependency notices: {error}"))?;
    for package in &catalog.packages {
        if package
            .files
            .iter()
            .any(|file| !catalog.texts.contains_key(&file.text_id))
        {
            return Err(format!(
                "Missing bundled license text for {} {}",
                package.name, package.version
            ));
        }
    }
    catalog.texts.insert(
        "orbit".into(),
        format!(
            "Copyright (C) 2026 Orvion Lab\n\n{}",
            include_str!("../../../../LICENSE")
        ),
    );
    catalog.packages.insert(
        0,
        Dependency {
            name: "Presenter Orbit".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            license: Some("GPL-3.0-only".into()),
            repository: None,
            files: vec![File {
                name: "LICENSE".into(),
                text_id: "orbit".into(),
            }],
        },
    );
    Ok(catalog)
}

fn declaration(license: Option<&str>, has_text: bool, t: Translator) -> SharedString {
    match license {
        Some(license) => license.to_string().into(),
        None if has_text => t.t("about.licenseSeeText").into(),
        None => t.t("about.licenseNotDeclared").into(),
    }
}

struct Detail {
    title: SharedString,
    license: Option<SharedString>,
    repository: Option<String>,
    texts: Vec<(SharedString, SharedString)>,
}

pub(super) struct Licenses {
    host: Host,
    catalog: Option<Arc<Catalog>>,
    loading: bool,
    error: Option<String>,
    source_error: Option<String>,
    filter: Entity<InputState>,
    query: String,
    visible: Vec<usize>,
    detail: Option<Detail>,
    _filter_subscription: Subscription,
}

impl Licenses {
    pub(super) fn new(host: Host, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filter = filter_state("", window, cx);
        let subscription = cx.subscribe(&filter, |view, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                view.query = input.read(cx).value().to_lowercase();
                view.refilter();
                cx.notify();
            }
        });
        Self {
            host,
            catalog: None,
            loading: false,
            error: None,
            source_error: None,
            filter,
            query: String::new(),
            visible: Vec::new(),
            detail: None,
            _filter_subscription: subscription,
        }
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        if self.catalog.is_some() || self.loading || self.error.is_some() {
            return;
        }
        self.loading = true;
        cx.spawn(async |owner, cx| {
            let result = cx.background_spawn(async { catalog() }).await;
            let _ = owner.update(cx, |view, cx| {
                view.loading = false;
                match result {
                    Ok(catalog) => {
                        view.catalog = Some(Arc::new(catalog));
                        view.refilter();
                    }
                    Err(error) => view.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn refilter(&mut self) {
        self.visible = self.catalog.as_ref().map_or_else(Vec::new, |catalog| {
            catalog
                .packages
                .iter()
                .enumerate()
                .filter_map(|(index, item)| {
                    let text = format!(
                        "{} {} {}",
                        item.name,
                        item.version,
                        item.license.as_deref().unwrap_or("")
                    )
                    .to_lowercase();
                    text.contains(&self.query).then_some(index)
                })
                .collect()
        });
    }

    pub(super) fn reset(&mut self, cx: &mut Context<Self>) {
        self.load(cx);
        self.detail = None;
        self.source_error = None;
        cx.notify();
    }

    pub(super) fn back(&mut self, cx: &mut Context<Self>) -> bool {
        let detail = self.detail.take().is_some();
        self.source_error = None;
        cx.notify();
        detail
    }

    fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(catalog) = &self.catalog else { return };
        let Some(item) = catalog.packages.get(index) else {
            return;
        };
        self.detail = Some(Detail {
            title: format!("{} {}", item.name, item.version).into(),
            license: item.license.clone().map(Into::into),
            repository: item.repository.clone(),
            texts: item
                .files
                .iter()
                .map(|file| {
                    (
                        file.name.clone().into(),
                        catalog.texts[&file.text_id].clone().into(),
                    )
                })
                .collect(),
        });
        cx.notify();
    }

    fn open_source(&mut self, url: String, cx: &mut Context<Self>) {
        let host = self.host.clone();
        cx.spawn(async move |owner, cx| {
            let result = cx
                .background_spawn(async move { host.invoke("openExternalUrl", vec![json!(url)]) })
                .await;
            let _ = owner.update(cx, |view, cx| {
                view.source_error = result.err();
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for Licenses {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let t = *cx.global::<Translator>();
        let root = div()
            .debug_selector(|| "licenses-page".into())
            .flex()
            .flex_col()
            .size_full()
            .p(theme.metrics.panel_padding)
            .min_h_0()
            .gap(theme.metrics.control_gap);
        if let Some(detail) = &self.detail {
            let url = detail.repository.clone();
            return root
                .child(
                    div()
                        .debug_selector(|| "license-detail-title".into())
                        .text_size(theme.metrics.font_lg)
                        .child(detail.title.clone()),
                )
                .child(
                    div()
                        .debug_selector(|| "license-declaration".into())
                        .child(declaration(
                            detail.license.as_deref(),
                            !detail.texts.is_empty(),
                            t,
                        )),
                )
                .when_some(url, |root, url| {
                    root.child(
                        ActionButton::new("license-source", t.t("about.licenseSource"))
                            .variant(ButtonVariant::Secondary)
                            .on_click(
                                cx.listener(move |view, _, _, cx| {
                                    view.open_source(url.clone(), cx)
                                }),
                            ),
                    )
                })
                .children(self.source_error.clone().map(EmptyState::new))
                .child(
                    div()
                        .id("license-detail")
                        .debug_selector(|| "license-detail".into())
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .when(detail.texts.is_empty(), |body| {
                            body.child(
                                div()
                                    .debug_selector(|| "license-text-unavailable".into())
                                    .child(EmptyState::new(t.t("about.licenseTextNotBundled"))),
                            )
                        })
                        .children(detail.texts.iter().map(|(name, text)| {
                            div()
                                .debug_selector(|| "license-original-text".into())
                                .mb(theme.metrics.space_lg)
                                .child(
                                    div()
                                        .text_color(theme.colors.text_muted)
                                        .mb(theme.metrics.space_sm)
                                        .child(name.clone()),
                                )
                                .child(div().text_size(theme.metrics.font_sm).child(text.clone()))
                        })),
                )
                .into_any_element();
        }
        if let Some(error) = &self.error {
            return root
                .child(EmptyState::new(error.clone()))
                .into_any_element();
        }
        let Some(catalog) = self.catalog.clone() else {
            return root
                .child(EmptyState::new(t.t("status.workspace.loading")))
                .into_any_element();
        };
        let visible = self.visible.clone();
        let owner = cx.weak_entity();
        root.child(
            div()
                .debug_selector(|| "licenses-filter".into())
                .child(FilterInput::new("licenses-filter", &self.filter)),
        )
        .child(
            gpui::uniform_list("licenses-list", visible.len(), move |range, _, cx| {
                range
                    .map(|row| {
                        let index = visible[row];
                        let package = &catalog.packages[index];
                        let owner = owner.clone();
                        NativeButton::new(("license-open", index))
                            .accessibility_label(format!("{} {}", package.name, package.version))
                            .w_full()
                            .h(theme.metrics.row_height)
                            .focus_visible(|style| style.bg(theme.colors.control_bg_hover))
                            .child(
                                list_row(("license", index), false, cx)
                                    .debug_selector(move || {
                                        format!(
                                            "license-package-{}-{}",
                                            package.name, package.version
                                        )
                                    })
                                    .cursor_pointer()
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .truncate()
                                            .child(format!("{} {}", package.name, package.version)),
                                    )
                                    .child(div().text_color(theme.colors.text_muted).child(
                                        declaration(
                                            package.license.as_deref(),
                                            !package.files.is_empty(),
                                            t,
                                        ),
                                    )),
                            )
                            .on_click(move |_, _, cx| {
                                let _ = owner.update(cx, |view, cx| view.select(index, cx));
                            })
                    })
                    .collect()
            })
            .flex_1()
            .min_h_0(),
        )
        .when(self.visible.is_empty(), |root| {
            root.child(EmptyState::new(t.t("search.empty.noResults")))
        })
        .into_any_element()
    }
}
