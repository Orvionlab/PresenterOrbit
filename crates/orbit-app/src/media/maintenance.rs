use super::*;
use orbit_ui::controls::Dialog;

impl MediaBin {
    pub(super) fn scan_maintenance(&mut self, cleanup: bool, cx: &mut Context<Self>) {
        if self.busy || self.maintenance_busy {
            return;
        }
        self.maintenance_busy = true;
        self.maintenance_error.clear();
        self.maintenance = Some((cleanup, json!([])));
        self.maintenance_request += 1;
        let request = self.maintenance_request;
        let workspace = self.workspace.clone();
        let host = self.runtime.host.clone();
        let media = self.media.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    if cleanup {
                        host.invoke("reviewMediaCleanup", vec![json!(workspace)])
                    } else {
                        host.invoke("listMissingMedia", vec![json!(workspace), json!(media)])
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.maintenance_request != request {
                    return;
                }
                this.maintenance_busy = false;
                match result {
                    Ok(result) => this.maintenance = Some((cleanup, result)),
                    Err(error) => this.maintenance_error = error,
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub(super) fn close_maintenance(&mut self, cx: &mut Context<Self>) {
        self.maintenance = None;
        self.maintenance_busy = false;
        self.maintenance_request += 1;
        cx.notify();
    }
    pub(super) fn maintenance_dialog(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let (cleanup, data) = self.maintenance.as_ref()?;
        let cleanup = *cleanup;
        let t = *cx.global::<Translator>();
        let weak = cx.weak_entity();
        let mut dialog = Dialog::new(
            "media-maintenance",
            t.t(if cleanup {
                "media.cleanup.title"
            } else {
                "media.missing.title"
            }),
            t.t("common.close"),
            move |_, _, cx| {
                let _ = weak.update(cx, |this, cx| this.close_maintenance(cx));
            },
        );
        let files = if cleanup {
            data["files"].as_array()
        } else {
            data.as_array()
        };
        let mut body = div()
            .id("media-maintenance-files")
            .flex()
            .flex_col()
            .gap(cx.theme().metrics.space_xs)
            .max_h(cx.theme().metrics.row_height * 15.)
            .overflow_y_scroll();
        if !cleanup {
            body = body
                .child(t.t("media.missing.help"))
                .child(t.t("media.missing.derivedHelp"));
        }
        if self.maintenance_busy || self.busy {
            body = body.child(
                div()
                    .id("missing-busy")
                    .role(gpui::Role::Status)
                    .child(t.t("media.missing.busy")),
            );
        }
        if !self.maintenance_error.is_empty() {
            body = body.child(
                div()
                    .id("missing-error")
                    .role(gpui::Role::Alert)
                    .child(self.maintenance_error.clone()),
            );
        }
        if let Some(files) = files {
            if files.is_empty() && !self.maintenance_busy && self.maintenance_error.is_empty() {
                body = body.child(t.t(if cleanup {
                    "media.cleanup.empty"
                } else {
                    "media.missing.none"
                }));
            }
            for (index, file) in files.iter().enumerate() {
                let mut row = div()
                    .flex()
                    .flex_col()
                    .child(text(file, if cleanup { "relativePath" } else { "path" }).to_string());
                if !cleanup {
                    row = row
                        .child(text(&file["media"], "title").to_string())
                        .child(t.t(if file["kind"] == "source" {
                            "media.missing.source"
                        } else {
                            "media.missing.thumbnail"
                        }));
                    for (method, key) in [
                        ("relinkMissingMedia", "media.missing.relink"),
                        ("forgetMissingMedia", "media.missing.forget"),
                    ] {
                        if method == "forgetMissingMedia"
                            && (file["kind"] != "source"
                                || text(&file["media"], "libraryId").is_empty())
                        {
                            continue;
                        }
                        let file = file.clone();
                        row = row.child(
                            ActionButton::new(
                                SharedString::from(format!("{method}-{index}")),
                                t.t(key),
                            )
                            .disabled(self.busy || self.maintenance_busy)
                            .on_click(cx.listener(
                                move |this, _, _, cx| this.mutate(method, vec![file.clone()], cx),
                            )),
                        );
                    }
                }
                body = body.child(row);
            }
        }
        if let Some(path) = data["recoveryPath"].as_str() {
            body = body
                .child(t.t("media.cleanup.recovery"))
                .child(path.to_string());
        }
        dialog = dialog.child(body);
        if cleanup {
            let review = data.clone();
            dialog = dialog.child(
                ActionButton::new("media-clean-confirm", t.t("media.cleanup.confirm"))
                    .disabled(self.busy || files.is_none_or(Vec::is_empty))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.mutate("cleanUnusedMedia", vec![review.clone()], cx)
                    })),
            );
        } else {
            dialog = dialog.child(
                ActionButton::new("media-missing-refresh", t.t("media.missing.refresh"))
                    .disabled(self.busy || self.maintenance_busy)
                    .on_click(cx.listener(|this, _, _, cx| this.scan_maintenance(false, cx))),
            );
        }
        Some(dialog.into_any_element())
    }
}
