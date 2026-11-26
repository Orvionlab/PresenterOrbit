//! Native frames, latest-wins delivery, first-frame readiness, and bounded still caching.

use std::collections::VecDeque;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};

use gpui::{div, prelude::*, Context, Entity, Render, RenderImage, Subscription, Window};
use orbit_host::{native_compositor::NativeFrame, Host};
use orbit_ui::controls::EmptyState;
use orbit_ui::{bgra_image, ActiveTheme, FrameSize, FrameView, Presented, Translator};
use serde_json::Value;

use crate::collections::model::Item;
use crate::startup::first_page::{self, Asset};

const CACHE_BYTES: usize = 64 * 1024 * 1024;
const CACHE_ENTRIES: usize = 128;
const FRAME_INTERVAL: Duration = Duration::from_nanos(1_000_000_000 / 30);

#[derive(Clone)]
pub enum Source {
    Still {
        key: String,
        slide: Arc<Value>,
        transparent: bool,
    },
    /// A stage layout over sample current and next slide text.
    StageLayout {
        key: String,
        slide: Arc<Value>,
    },
    Mask {
        key: String,
        slide: Arc<Value>,
    },
    Output(String),
    Cue {
        key: String,
        snapshot: Arc<Value>,
        item: Item,
        index: usize,
        playlist_id: String,
        background: Option<String>,
    },
}

#[derive(Clone, PartialEq, Eq)]
struct CacheKey {
    source: String,
    width: u32,
    height: u32,
}

struct Cached {
    key: CacheKey,
    image: Arc<RenderImage>,
    visual: u64,
    bytes: usize,
}

#[derive(Default)]
struct Cache {
    entries: VecDeque<Cached>,
    bytes: usize,
}

impl Cache {
    fn get(&mut self, key: &CacheKey) -> Option<(Arc<RenderImage>, u64)> {
        let index = self.entries.iter().position(|entry| &entry.key == key)?;
        let entry = self.entries.remove(index)?;
        let answer = (entry.image.clone(), entry.visual);
        self.entries.push_back(entry);
        Some(answer)
    }

    fn insert(&mut self, entry: Cached) {
        if entry.bytes > CACHE_BYTES {
            return;
        }
        if let Some(index) = self
            .entries
            .iter()
            .position(|cached| cached.key == entry.key)
        {
            self.bytes -= self
                .entries
                .remove(index)
                .expect("cache entry exists")
                .bytes;
        }
        while self.bytes + entry.bytes > CACHE_BYTES || self.entries.len() >= CACHE_ENTRIES {
            let Some(old) = self.entries.pop_front() else {
                break;
            };
            self.bytes -= old.bytes;
        }
        self.bytes += entry.bytes;
        self.entries.push_back(entry);
    }
}

#[derive(Clone, Default)]
pub struct Frames {
    cache: Arc<Mutex<Cache>>,
    still_lane: Arc<futures::lock::Mutex<()>>,
}

struct Rendered {
    image: Option<Arc<RenderImage>>,
    visual: u64,
    dynamic: bool,
}

pub struct Preview {
    host: Host,
    frames: Frames,
    pub frame: Entity<FrameView>,
    source: Option<Source>,
    source_size: Option<FrameSize>,
    size: Option<FrameSize>,
    generation: Arc<AtomicU64>,
    revision: u64,
    busy: bool,
    visual: Option<u64>,
    pub ready: bool,
    error: Option<String>,
    failures: usize,
    dynamic: bool,
    _frame_subscription: Subscription,
}

impl Preview {
    pub fn new(host: Host, frames: Frames, cx: &mut Context<Self>) -> Self {
        let owner = cx.weak_entity();
        let frame = cx.new(|cx| {
            FrameView::new(
                move |size, cx| {
                    let _ = owner.update(cx, |view: &mut Preview, cx| {
                        let resized = view.size != Some(size);
                        if resized {
                            view.generation.fetch_add(1, Ordering::Relaxed);
                            view.ready = false;
                            first_page::changed(Asset::Preview(cx.entity_id()), false, cx);
                        }
                        view.size = Some(size);
                        // Visibility can expire during a slow render. A painted still does not need
                        // another frame merely because its next paint arrives after that interval.
                        if resized || (!view.ready && view.failures < 3) || view.dynamic {
                            view.start_request(cx);
                        }
                    });
                },
                cx,
            )
        });
        let generation = Arc::new(AtomicU64::new(0));
        let subscription = cx.subscribe(&frame, |view, _, _: &Presented, cx| {
            if !view.ready {
                view.ready = true;
                first_page::changed(Asset::Preview(cx.entity_id()), true, cx);
                cx.notify();
            }
        });
        let release = generation.clone();
        let startup_asset = Asset::Preview(cx.entity_id());
        cx.on_release(move |_, cx| {
            first_page::forget(startup_asset, cx);
            release.fetch_add(1, Ordering::Relaxed);
        })
        .detach();
        Self {
            host,
            frames,
            frame,
            source: None,
            source_size: None,
            size: None,
            generation,
            revision: 0,
            busy: false,
            visual: None,
            ready: false,
            error: None,
            failures: 0,
            dynamic: false,
            _frame_subscription: subscription,
        }
    }

    pub fn suspend(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_source(None, None, self.revision, cx);
        self.frame
            .update(cx, |frame, cx| frame.release_after_present(window, cx));
    }

    pub fn show_checkerboard(&mut self, cx: &mut Context<Self>) {
        self.frame.update(cx, |frame, _| frame.show_checkerboard());
    }

    pub fn set_source(
        &mut self,
        source: Option<Source>,
        resolution: Option<FrameSize>,
        revision: u64,
        cx: &mut Context<Self>,
    ) {
        // The owner increments revision for content changes; unchanged views keep their image and request state.
        if self.revision == revision
            && source_id(self.source.as_ref()) == source_id(source.as_ref())
            && self.source_size == resolution
        {
            return;
        }
        if self.source.is_none() || source.is_none() {
            self.visual = None;
        }
        self.source = source;
        self.source_size = resolution;
        self.revision = revision;
        self.generation.fetch_add(1, Ordering::Relaxed);
        self.ready = false;
        first_page::changed(Asset::Preview(cx.entity_id()), self.source.is_none(), cx);
        self.error = None;
        self.failures = 0;
        self.dynamic = false;
        self.frame.update(cx, |frame, cx| {
            frame.reset_presentation(cx);
            frame.set_source(resolution, cx);
            if self.source.is_none() {
                frame.clear(cx);
            }
        });
        self.request(cx);
        cx.notify();
    }

    fn request(&mut self, cx: &mut Context<Self>) {
        if self.frame.read(cx).is_visible() {
            self.start_request(cx);
        }
    }

    fn start_request(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let (Some(source), Some(size)) = (self.source.clone(), self.size) else {
            return;
        };
        let version = self.generation.load(Ordering::Relaxed);
        let cancellation = self.generation.clone();
        let (host, frames, previous) = (self.host.clone(), self.frames.clone(), self.visual);
        let started = Instant::now();
        self.busy = true;
        cx.spawn(async move |owner, cx| {
            let rendered = cx
                .background_spawn(async move {
                    let _still_guard = if matches!(
                        &source,
                        Source::Cue { .. }
                            | Source::Still { .. }
                            | Source::StageLayout { .. }
                            | Source::Mask { .. }
                    ) {
                        Some(frames.still_lane.lock().await)
                    } else {
                        None
                    };
                    if cancellation.load(Ordering::Relaxed) != version {
                        return Ok(None);
                    }
                    render(&host, &frames, &source, size, previous).map(Some)
                })
                .await;
            let _ = owner.update(cx, |view, cx| {
                view.busy = false;
                if view.generation.load(Ordering::Relaxed) != version {
                    view.request(cx);
                    return;
                }
                match rendered {
                    Ok(Some(rendered)) => {
                        view.visual = Some(rendered.visual);
                        view.dynamic = rendered.dynamic;
                        view.failures = 0;
                        if let Some(image) = rendered.image {
                            view.frame
                                .update(cx, |frame, cx| frame.set_image(image, cx));
                        } else {
                            view.frame
                                .update(cx, |frame, cx| frame.present_current_image(cx));
                        }
                        view.error = None;
                        cx.notify();
                        if rendered.dynamic {
                            view.schedule(started + FRAME_INTERVAL, version, cx);
                        }
                    }
                    Ok(None) => {}
                    Err(error) => {
                        view.failures += 1;
                        view.error = Some(error);
                        if view.failures >= 3 {
                            first_page::changed(Asset::Preview(cx.entity_id()), true, cx);
                        }
                        if view.dynamic || (!view.ready && view.failures < 3) {
                            view.schedule(
                                Instant::now()
                                    + if view.dynamic {
                                        FRAME_INTERVAL
                                    } else {
                                        Duration::from_millis(100)
                                    },
                                version,
                                cx,
                            );
                        }
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    fn schedule(&self, deadline: Instant, version: u64, cx: &mut Context<Self>) {
        cx.spawn(async move |owner, cx| {
            // Short waits avoid Windows' 33 ms timer rounding up to roughly 50 ms.
            while Instant::now() < deadline {
                cx.background_executor()
                    .timer(
                        Duration::from_millis(4)
                            .min(deadline.saturating_duration_since(Instant::now())),
                    )
                    .await;
            }
            let _ = owner.update(cx, |view, cx| {
                if view.generation.load(Ordering::Relaxed) == version {
                    view.frame.update(cx, |_, cx| cx.notify());
                    view.request(cx);
                }
            });
        })
        .detach();
    }
}

fn source_id(source: Option<&Source>) -> Option<(u8, &str)> {
    match source? {
        Source::Output(id) => Some((0, id)),
        Source::Cue { key, .. } => Some((2, key)),
        Source::Still { key, .. } => Some((3, key)),
        Source::StageLayout { key, .. } => Some((4, key)),
        Source::Mask { key, .. } => Some((5, key)),
    }
}

fn render(
    host: &Host,
    frames: &Frames,
    source: &Source,
    size: FrameSize,
    previous: Option<u64>,
) -> Result<Rendered, String> {
    let cache_key = match source {
        Source::Cue { key, .. }
        | Source::Still { key, .. }
        | Source::StageLayout { key, .. }
        | Source::Mask { key, .. } => Some(CacheKey {
            source: key.clone(),
            width: size.width,
            height: size.height,
        }),
        _ => None,
    };
    if let Some(key) = &cache_key {
        if let Some((image, visual)) = frames
            .cache
            .lock()
            .map_err(|_| "The thumbnail cache is unavailable.")?
            .get(key)
        {
            return Ok(Rendered {
                image: (previous != Some(visual)).then_some(image),
                visual,
                dynamic: false,
            });
        }
    }
    let frame: NativeFrame = match source {
        Source::Still {
            slide,
            transparent: true,
            ..
        } => orbit_host::native_compositor::render_slide_still_over(
            slide,
            &serde_json::json!({"backgroundCss":"transparent"}),
            size.width as usize,
            size.height as usize,
        )?,
        Source::Still {
            slide,
            transparent: false,
            ..
        } => orbit_host::native_compositor::render_slide_still(
            slide,
            size.width as usize,
            size.height as usize,
        )?,
        Source::StageLayout { slide, .. } => {
            orbit_host::native_compositor::render_stage_layout_still(
                slide,
                size.width as usize,
                size.height as usize,
            )?
        }
        Source::Mask { slide, .. } => orbit_host::native_compositor::render_mask_still(
            slide,
            size.width as usize,
            size.height as usize,
        )?,
        Source::Output(id) => {
            let frame = host.render_output_bgra_frame(id, size.width, size.height)?;
            Arc::try_unwrap(frame).unwrap_or_else(|frame| (*frame).clone())
        }
        Source::Cue {
            snapshot,
            item,
            index,
            playlist_id,
            background,
            ..
        } => {
            let slide = item.value["slides"]
                .as_array()
                .and_then(|slides| slides.get(*index))
                .ok_or("The cue thumbnail slide is no longer present.")?;
            host.render_cue_still_with_background(
                snapshot,
                &item.value,
                slide,
                playlist_id,
                (size.width, size.height),
                background.as_deref(),
            )?
        }
    };
    let mut hasher = DefaultHasher::new();
    frame.width.hash(&mut hasher);
    frame.height.hash(&mut hasher);
    frame.pixels.hash(&mut hasher);
    let visual = hasher.finish();
    let dynamic = frame.dynamic_media && cache_key.is_none();
    let bytes = frame.pixels.len();
    let image = if previous == Some(visual) {
        None
    } else {
        Some(bgra_image(size, frame.pixels)?)
    };
    if let (Some(key), Some(image)) = (cache_key, image.as_ref()) {
        frames
            .cache
            .lock()
            .map_err(|_| "The thumbnail cache is unavailable.")?
            .insert(Cached {
                key,
                image: image.clone(),
                visual,
                bytes,
            });
    }
    Ok(Rendered {
        image,
        visual,
        dynamic,
    })
}

impl Render for Preview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let program = matches!(self.source, None | Some(Source::Output(_)));
        let transparent = matches!(
            self.source,
            Some(
                Source::Mask { .. }
                    | Source::Still {
                        transparent: true,
                        ..
                    }
            )
        );
        let screen_size = self.source_size;
        let asset = Asset::Preview(cx.entity_id());
        let ready = self.source.is_none() || self.ready || self.failures >= 3;
        div()
            .when(program && self.ready, |element| {
                element.debug_selector(|| "program-preview-ready".into())
            })
            .relative()
            .size_full()
            .when(!transparent, |element| {
                element.bg(if program {
                    theme.colors.canvas_bg
                } else {
                    theme.colors.preview_matte_bg
                })
            })
            .when(program, |element| {
                element.child(
                    gpui::canvas(
                        move |mut bounds, _, _| {
                            let source = screen_size?;
                            let scale = (f32::from(bounds.size.width) / source.width as f32)
                                .min(f32::from(bounds.size.height) / source.height as f32);
                            let size = gpui::size(
                                gpui::px(source.width as f32 * scale),
                                gpui::px(source.height as f32 * scale),
                            );
                            bounds.origin += ((bounds.size - size) / 2.).into();
                            bounds.size = size;
                            Some(bounds)
                        },
                        |_, bounds, window, _| {
                            if let Some(bounds) = bounds {
                                window.paint_quad(gpui::fill(
                                    bounds,
                                    orbit_ui::theme::PROGRAM_PREVIEW_BG,
                                ));
                            }
                        },
                    )
                    .absolute()
                    .size_full(),
                )
            })
            .child(div().absolute().size_full().child(self.frame.clone()))
            .when(cx.has_global::<first_page::FirstPage>(), |element| {
                element.child(
                    gpui::canvas(
                        move |bounds, window, cx| {
                            if !bounds.intersect(&window.content_mask().bounds).is_empty() {
                                first_page::visible(asset.clone(), ready, window, cx);
                            }
                        },
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .size_full(),
                )
            })
            .when(
                self.error.is_some() && !self.ready && self.failures >= 3,
                |element| {
                    element.child(
                        div()
                            .absolute()
                            .bottom_0()
                            .left_0()
                            .right_0()
                            .bg(theme.colors.panel_bg)
                            .child(EmptyState::new(
                                self.error.clone().expect("preview error exists"),
                            )),
                    )
                },
            )
            .when(
                !program
                    && !transparent
                    && !matches!(
                        self.source,
                        Some(Source::Cue { .. } | Source::StageLayout { .. })
                    )
                    && !self.ready
                    && self.failures < 3,
                |element| {
                    element.child(
                        div()
                            .debug_selector(|| "preview-placeholder".into())
                            .absolute()
                            .inset_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(EmptyState::new(
                                cx.global::<Translator>().t("search.preview.empty"),
                            )),
                    )
                },
            )
    }
}
