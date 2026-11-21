use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::ActiveTheme;
use gpui::{canvas, prelude::*, Context, Corners, Entity, Pixels, RenderImage, Size, Window};
use image::{Frame, RgbaImage};

/// A bitmap size in device pixels. Slide units and logical `Pixels` are other spaces, converted only here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameSize {
    pub width: u32,
    pub height: u32,
}

/// The bitmap to request for a view: `bounds x scale_factor`, shrunk uniformly so that neither side exceeds
/// the source resolution. Upscaling a 1080p output adds nothing.
pub fn device_size(
    bounds: Size<Pixels>,
    scale_factor: f32,
    source: Option<FrameSize>,
) -> Option<FrameSize> {
    let (width, height) = (
        f32::from(bounds.width) * scale_factor,
        f32::from(bounds.height) * scale_factor,
    );
    if !width.is_finite() || !height.is_finite() || width < 0.5 || height < 0.5 {
        return None;
    }
    let shrink = source
        .map_or(1.0, |source| {
            (source.width as f32 / width)
                .min(source.height as f32 / height)
                .min(1.0)
        })
        .min(8192. / width.max(height))
        .min((33_554_432. / (width * height)).sqrt());
    let mut size = FrameSize {
        width: ((width * shrink).round() as u32).max(1),
        height: ((height * shrink).round() as u32).max(1),
    };
    if u64::from(size.width) * u64::from(size.height) > 33_554_432 {
        size.height = 33_554_432 / size.width;
    }
    Some(size)
}

/// Wraps compositor output. The compositor emits straight-alpha BGRA, which is what GPUI's sprite atlas holds
/// and what its blend state expects (spike S2), so the bytes go in unchanged. `image` only carries them.
pub fn bgra_image(size: FrameSize, bytes: Vec<u8>) -> Result<Arc<RenderImage>, String> {
    let buffer = RgbaImage::from_raw(size.width, size.height, bytes).ok_or_else(|| {
        format!(
            "A {}x{} frame needs {} bytes.",
            size.width,
            size.height,
            size.width as usize * size.height as usize * 4
        )
    })?;
    Ok(Arc::new(RenderImage::new(smallvec::smallvec![Frame::new(
        buffer
    )])))
}

/// Emitted once a delivered bitmap has been painted. A notify cannot report this: while a window renders the
/// view, a notify only invalidates it and never reaches observers.
pub struct Presented;

impl gpui::EventEmitter<Presented> for FrameView {}

type RequestHandler = Box<dyn Fn(FrameSize, &mut Context<FrameView>)>;

/// Shows the latest bitmap its owner delivered. Whenever the painted size or the window's scale factor
/// changes, it asks the owner for a bitmap at the new device size and keeps showing the old one until that
/// arrives.
pub struct FrameView {
    image: Option<Arc<RenderImage>>,
    painted_image: Option<Arc<RenderImage>>,
    retired: Vec<Arc<RenderImage>>,
    source: Option<FrameSize>,
    stretch: bool,
    checkerboard: bool,
    requested: Option<FrameSize>,
    last_visible: Option<Instant>,
    presented: bool,
    pending_presentation: bool,
    on_request: RequestHandler,
}

impl FrameView {
    pub fn new(
        on_request: impl Fn(FrameSize, &mut Context<FrameView>) + 'static,
        cx: &mut Context<Self>,
    ) -> Self {
        // Dropping the view must free its atlas entries, or the atlas keeps them for the window's life.
        cx.on_release(|view, cx| {
            for image in view.image.take().into_iter().chain(view.retired.drain(..)) {
                cx.drop_image(image, None);
            }
        })
        .detach();
        Self {
            image: None,
            painted_image: None,
            retired: Vec::new(),
            source: None,
            stretch: false,
            checkerboard: false,
            requested: None,
            last_visible: None,
            presented: false,
            pending_presentation: false,
            on_request: Box::new(on_request),
        }
    }

    /// The native resolution of what is shown, when it has one. It caps the requested size.
    pub fn set_source(&mut self, source: Option<FrameSize>, cx: &mut Context<Self>) {
        if self.source != source {
            self.source = source;
            self.requested = None;
            cx.notify();
        }
    }

    pub fn requested(&self) -> Option<FrameSize> {
        self.requested
    }

    pub fn is_visible(&self) -> bool {
        self.last_visible
            .is_some_and(|time| time.elapsed() < Duration::from_millis(200))
    }

    /// Canvas gesture planes may stretch independently along each slide axis.
    pub fn set_stretch(&mut self, stretch: bool) {
        self.stretch = stretch;
    }

    pub fn show_checkerboard(&mut self) {
        self.checkerboard = true;
    }

    /// Retire an unmounted canvas's atlas images after the replacing window frame is presented.
    pub fn release_after_present(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.reset_presentation(cx);
        self.painted_image = None;
        let images = self
            .retired
            .drain(..)
            .chain(self.image.take())
            .collect::<Vec<_>>();
        window.on_next_frame(move |window, cx| {
            for image in images {
                cx.drop_image(image, Some(window));
            }
        });
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.reset_presentation(cx);
        self.retired.extend(self.image.take());
        self.painted_image = None;
        cx.notify();
    }

    pub fn has_presented(&self) -> bool {
        self.presented
    }

    pub fn reset_presentation(&mut self, cx: &mut Context<Self>) {
        self.presented = false;
        self.pending_presentation = false;
        cx.notify();
    }

    /// The accepted scene produced identical pixels, so acknowledge the existing bitmap on its next paint.
    pub fn present_current_image(&mut self, cx: &mut Context<Self>) {
        self.presented = false;
        self.pending_presentation = self.image.is_some();
        cx.notify();
    }

    /// Replaces the shown bitmap. The old one stays in the atlas until the frame that shows the new one has
    /// been presented.
    pub fn set_image(&mut self, image: Arc<RenderImage>, cx: &mut Context<Self>) {
        self.presented = false;
        self.pending_presentation = true;
        if let Some(old) = self.image.replace(image) {
            if self
                .painted_image
                .as_ref()
                .is_some_and(|painted| Arc::ptr_eq(painted, &old))
            {
                self.retired.push(old);
            } else {
                // This view never painted the superseded bitmap, so it owns no atlas entry for it.
                drop(old);
            }
        }
        cx.notify();
    }

    fn observe_bounds(&mut self, bounds: Size<Pixels>, scale_factor: f32, cx: &mut Context<Self>) {
        let Some(size) = device_size(bounds, scale_factor, self.source) else {
            return;
        };
        let was_visible = self.is_visible();
        self.last_visible = Some(Instant::now());
        if self.requested != Some(size) || !was_visible {
            self.requested = Some(size);
            (self.on_request)(size, cx);
        }
    }
}

impl Render for FrameView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (image, view): (_, Entity<Self>) = (self.image.clone(), cx.entity());
        let sizing = view.clone();
        let source = self.source;
        let stretch = self.stretch;
        let checkerboard = self.checkerboard.then(|| {
            gpui::checkerboard(
                crate::theme::PROGRAM_PREVIEW_BG,
                f32::from(cx.theme().metrics.space_sm),
            )
        });
        canvas(
            move |mut bounds, window, cx| {
                if let Some(source) = source.filter(|_| !stretch) {
                    let shrink = (f32::from(bounds.size.width) / source.width as f32)
                        .min(f32::from(bounds.size.height) / source.height as f32);
                    let fitted = gpui::size(
                        gpui::px(source.width as f32 * shrink),
                        gpui::px(source.height as f32 * shrink),
                    );
                    bounds.origin.x += (bounds.size.width - fitted.width) / 2.;
                    bounds.origin.y += (bounds.size.height - fitted.height) / 2.;
                    bounds.size = fitted;
                }
                let scale_factor = window.scale_factor();
                if !bounds.intersect(&window.content_mask().bounds).is_empty() {
                    sizing.update(cx, |view, cx| {
                        view.observe_bounds(bounds.size, scale_factor, cx)
                    });
                }
                bounds
            },
            move |_, bounds, window, cx| {
                if let Some(pattern) = checkerboard {
                    window.paint_quad(gpui::fill(bounds, crate::theme::MEDIA_TRANSPARENCY_BG));
                    window.paint_quad(gpui::fill(bounds, pattern));
                }
                let Some(image) = image else {
                    let retired = view.update(cx, |view, _| std::mem::take(&mut view.retired));
                    if !retired.is_empty() {
                        window.on_next_frame(move |window, cx| {
                            for image in retired {
                                cx.drop_image(image, Some(window));
                            }
                        });
                    }
                    return;
                };
                // The image already has the device size, so it maps one to one onto the snapped bounds.
                if let Err(error) =
                    window.paint_image(bounds, bounds, Corners::default(), image.clone(), 0, false)
                {
                    eprintln!("Presenter Orbit could not paint a frame: {error}");
                    return;
                }
                view.update(cx, |view, cx| {
                    view.painted_image = Some(image);
                    if view.pending_presentation {
                        view.pending_presentation = false;
                        view.presented = true;
                        cx.emit(Presented);
                        cx.notify();
                    }
                });
                let retired = view.update(cx, |view, _| std::mem::take(&mut view.retired));
                if !retired.is_empty() {
                    window.on_next_frame(move |window, cx| {
                        for image in retired {
                            cx.drop_image(image, Some(window));
                        }
                    });
                }
            },
        )
        .size_full()
    }
}
