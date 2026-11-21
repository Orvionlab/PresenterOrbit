use super::*;

#[derive(Default)]
pub(super) struct Preparation {
    pub hovered: String,
    pub key: Value,
    pub revision: u64,
    busy: bool,
}

/// Both the UI previews and the detached program process need their own process-local pixels.
fn prepare(host: &orbit_host::Host, requests: Value) -> Result<(), String> {
    host.invoke("prepareMedia", vec![requests.clone()])?;
    if let Some(route) = host
        .try_state::<crate::output_process::OutputRoute>()
        .and_then(|route| route.0.get())
    {
        if route.attached() {
            crate::state::runtime::output_call(host, "prepareMedia", vec![requests])?;
        }
    }
    Ok(())
}

impl MediaBin {
    pub(super) fn prepare_media(&mut self, cx: &mut Context<Self>) {
        if self.preparation.busy {
            return;
        }
        let key = json!([
            self.workspace,
            self.preparation.revision,
            self.library,
            self.preparation.hovered,
            self.selected,
            self.runtime.output.read(cx).media_preparation_epoch
        ]);
        if self.preparation.key == key {
            return;
        }
        let rows: Vec<_> = self
            .rows
            .iter()
            .filter(|item| {
                matches!(item["type"].as_str(), Some("image" | "video")) && item["missing"] != true
            })
            .collect();
        let start = rows
            .iter()
            .position(|item| item["id"] == self.preparation.hovered)
            .or_else(|| rows.iter().position(|item| item["id"] == self.selected))
            .unwrap_or(0);
        let requests: Vec<_> = (start..rows.len())
            .chain(0..start)
            .map(|index| {
                let item = rows[index];
                let start = if item["type"] == "video" {
                    item.get("mediaTransport")
                        .map_or(json!(0), |transport| transport["inSeconds"].clone())
                } else {
                    json!(0)
                };
                json!({"path":item["sourcePath"],"kind":item["type"],"startSeconds":start})
            })
            .collect();
        self.preparation.key = key;
        self.preparation.busy = true;
        let host = self.runtime.host.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { prepare(&host, json!(requests)) })
                .await;
            this.update(cx, |this, cx| {
                this.preparation.busy = false;
                if let Err(error) = result {
                    this.runtime.status.update(cx, |status, cx| {
                        status.message = error;
                        cx.notify();
                    });
                }
                this.prepare_media(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}
