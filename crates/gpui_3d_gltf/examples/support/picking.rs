use gpui::{Pixels, Point, Window};
use gpui_3d::{NodeHandle, ViewportPickCapture, ViewportPickFrame, ViewportPickReadback};

pub(super) struct Picking {
    pub capture: ViewportPickCapture,
    pub error: Option<String>,
    pending: Option<ViewportPickReadback>,
    queued: Option<(ViewportPickFrame, Point<Pixels>)>,
}

impl Picking {
    pub fn new() -> Self {
        Self {
            capture: ViewportPickCapture::new(256 * 1024 * 1024),
            error: None,
            pending: None,
            queued: None,
        }
    }

    pub fn clear(&mut self) {
        self.pending = None;
        self.queued = None;
        self.error = None;
        self.capture.clear();
    }

    pub fn click(&mut self, position: Point<Pixels>) {
        self.queued = None;
        self.error = None;
        match self.capture.frame() {
            Ok(Some(frame)) => self.queued = Some((frame, position)),
            Ok(None) => {
                self.pending = None;
                self.error = Some("Picking is waiting for a submitted viewport frame".into());
            }
            Err(error) => {
                self.pending = None;
                self.error = Some(error.to_string());
            }
        }
    }

    pub fn poll(&mut self, window: &Window) -> Option<NodeHandle> {
        let mut selected = None;
        if let Some(pending) = &mut self.pending {
            match pending.try_read() {
                Ok(Some(result)) => {
                    if self.queued.is_none() {
                        selected = result.frame.hit.and_then(|hit| hit.object.node);
                    }
                    self.pending = None;
                }
                Ok(None) => {}
                Err(error) => {
                    self.error = Some(error.to_string());
                    self.pending = None;
                }
            }
        }
        if self.pending.is_none()
            && let Some((frame, position)) = self.queued.take()
        {
            match frame.pick(position) {
                Ok(request) => {
                    self.pending = request;
                    self.error = None;
                }
                Err(error) => self.error = Some(error.to_string()),
            }
        }
        if self.pending.is_some() {
            window.request_animation_frame();
        }
        selected
    }
}
