use crate::locked::Locked;
use futures::channel::oneshot;
use gpui::RenderImage;
use orbit_host::native_compositor::{CanvasRenderKind, NativeCompositor};
use orbit_ui::{bgra_image, FrameSize};
use serde_json::Value;
use std::sync::{mpsc, Arc, Mutex};

pub struct Raster {
    pub images: Vec<Arc<RenderImage>>,
    pub dependent: bool,
}
struct Request {
    slide: Arc<Value>,
    context: Option<Arc<Value>>,
    size: FrameSize,
    active: Option<usize>,
    kind: CanvasRenderKind,
    reply: oneshot::Sender<Result<Raster, String>>,
}
/// One compositor and at most one waiting request. A superseded receiver is intentionally cancelled.
pub struct Worker {
    pending: Arc<Mutex<Option<Request>>>,
    wake: mpsc::SyncSender<()>,
}
impl Worker {
    pub fn new() -> Self {
        let pending: Arc<Mutex<Option<Request>>> = Arc::default();
        let slot = pending.clone();
        let (wake, receive) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let mut compositor = NativeCompositor::new_static_preview();
            while receive.recv().is_ok() {
                let request = slot.locked().take();
                let Some(request) = request else {
                    continue;
                };
                let result = compositor
                    .render_canvas(
                        &request.slide,
                        request.context.as_deref(),
                        request.size.width as usize,
                        request.size.height as usize,
                        request.active,
                        request.kind,
                    )
                    .and_then(|(frames, dependent)| {
                        Ok(Raster {
                            images: frames
                                .into_iter()
                                .map(|frame| bgra_image(request.size, frame.pixels))
                                .collect::<Result<_, _>>()?,
                            dependent,
                        })
                    });
                let _ = request.reply.send(result);
            }
        });
        Self { pending, wake }
    }
    pub fn render(
        &self,
        slide: Arc<Value>,
        context: Option<Arc<Value>>,
        size: FrameSize,
        active: Option<usize>,
        kind: CanvasRenderKind,
    ) -> oneshot::Receiver<Result<Raster, String>> {
        let (reply, receive) = oneshot::channel();
        *self.pending.locked() = Some(Request {
            slide,
            context,
            size,
            active,
            kind,
            reply,
        });
        if matches!(
            self.wake.try_send(()),
            Err(mpsc::TrySendError::Disconnected(_))
        ) {
            if let Some(request) = self.pending.locked().take() {
                let _ = request
                    .reply
                    .send(Err("The canvas raster worker stopped.".into()));
            }
        }
        receive
    }
}
