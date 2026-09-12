use gpui::{Pixels, Point, Window};
use gpui_3d::{RenderObject, ViewportPickCapture, ViewportPickFrame, ViewportPickReadback};

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

    pub fn cancel(&mut self) {
        self.pending = None;
        self.queued = None;
        self.error = None;
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

    pub fn poll(&mut self, window: &Window) -> Option<RenderObject> {
        let mut hit = None;
        if let Some(pending) = &mut self.pending {
            match pending.try_read() {
                Ok(Some(result)) => {
                    if self.queued.is_none() {
                        hit = result.frame.hit.map(|hit| hit.object);
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
                Ok(pending) => {
                    self.pending = pending;
                    self.error = None;
                }
                Err(error) => self.error = Some(error.to_string()),
            }
        }
        if self.pending.is_some() || self.queued.is_some() {
            window.request_animation_frame();
        }
        hit
    }
}
