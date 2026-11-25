use super::*;

#[derive(Default)]
pub(super) struct AssetDetails {
    pub key: Value,
    pub pending: bool,
    pub metadata: Value,
    pub error: String,
    pub image: Option<Arc<gpui::RenderImage>>,
    pub transparent: bool,
}

pub(super) fn details_key(workspace: &str, item: &Value) -> Value {
    json!([
        workspace,
        item["sourcePath"],
        item["thumbnailFile"],
        item["thumbnailTimeSeconds"],
        item["thumbnailMissing"],
        item["css"]
    ])
}

impl MediaBin {
    pub(super) fn request_details(&mut self, item: &Value, cx: &mut Context<Self>) {
        let id = text(item, "id").to_string();
        let key = details_key(&self.workspace, item);
        if self
            .details
            .get(&id)
            .is_some_and(|details| details.key == key)
            || (item["type"] != "background" && item["sourcePath"].as_str().is_none())
        {
            return;
        }
        let details = self.details.entry(id.clone()).or_default();
        if details.key[0] != key[0] || details.key[1] != key[1] || details.key[5] != key[5] {
            if let Some(image) = details.image.take() {
                cx.drop_image(image, None);
            }
            *details = AssetDetails::default();
        }
        details.key = key.clone();
        details.error.clear();
        details.pending = true;
        crate::startup::first_page::changed(
            crate::startup::first_page::Asset::Media(cx.entity_id(), id.clone()),
            false,
            cx,
        );
        let item = item.clone();
        let host = self.runtime.host.clone();
        let workspace = self.workspace.clone();
        cx.spawn(async move |this, cx| {
            let (metadata, image) = cx
                .background_spawn(async move {
                    let metadata = if item["type"] == "audio" || item["type"] == "video" {
                        host.invoke("probeMediaMetadata", vec![item["sourcePath"].clone()])
                    } else if item["type"] == "background" {
                        Ok(Value::Null)
                    } else if std::path::Path::new(text(&item, "sourcePath"))
                        .extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("svg"))
                    {
                        host.invoke("probeMediaMetadata", vec![item["sourcePath"].clone()])
                    } else {
                        image::image_dimensions(std::path::Path::new(text(&item, "sourcePath")))
                            .map(|(width, height)| json!({"width":width,"height":height}))
                            .map_err(|error| error.to_string())
                    };
                    let image = if item["type"] == "background" {
                        host.render_background_preview(text(&item, "css"))
                            .and_then(|frame| {
                                orbit_ui::bgra_image(
                                    orbit_ui::FrameSize {
                                        width: frame.width as u32,
                                        height: frame.height as u32,
                                    },
                                    frame.pixels,
                                )
                                .map(|image| Some((image, false)))
                            })
                    } else if item["type"] == "image" || item["type"] == "video" {
                        host.render_media_thumbnail(&workspace, &item)
                            .and_then(|image| {
                                let size = orbit_ui::FrameSize {
                                    width: image.width(),
                                    height: image.height(),
                                };
                                let transparent = image.pixels().any(|pixel| pixel[3] < 255);
                                let mut bytes = image.into_raw();
                                for pixel in bytes.as_chunks_mut::<4>().0 {
                                    pixel.swap(0, 2);
                                }
                                orbit_ui::bgra_image(size, bytes)
                                    .map(|image| Some((image, transparent)))
                            })
                    } else {
                        Ok(None)
                    };
                    (metadata, image)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                let Some(details) = this
                    .details
                    .get_mut(&id)
                    .filter(|details| details.key == key)
                else {
                    return;
                };
                match metadata {
                    Ok(value) => details.metadata = value,
                    Err(error) => details.error = error,
                }
                match image {
                    Ok(Some((image, transparent))) => {
                        if let Some(old) = details.image.replace(image) {
                            cx.drop_image(old, None);
                        }
                        details.transparent = transparent;
                    }
                    Ok(None) => {}
                    Err(error) => details.error = error,
                }
                details.pending = false;
                crate::startup::first_page::changed(
                    crate::startup::first_page::Asset::Media(cx.entity_id(), id.clone()),
                    true,
                    cx,
                );
                cx.notify();
            });
        })
        .detach();
    }
}

pub(crate) fn transparency_background(id: &str, cx: &gpui::App) -> gpui::Div {
    let selector = format!("media-transparency-{id}");
    div()
        .debug_selector(move || selector.clone())
        .absolute()
        .inset_0()
        .bg(orbit_ui::theme::MEDIA_TRANSPARENCY_BG)
        .child(div().size_full().bg(gpui::checkerboard(
            orbit_ui::theme::PROGRAM_PREVIEW_BG,
            f32::from(cx.theme().metrics.space_sm),
        )))
}
