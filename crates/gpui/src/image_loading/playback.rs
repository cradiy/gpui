use crate::{BackgroundExecutor, ImageAnimation, ImageCacheError, ImageId, RenderImage, Task};
use futures::FutureExt;
use scheduler::Instant;
use std::{sync::Arc, time::Duration};

pub(crate) struct AnimationPlayback {
    pub image_id: ImageId,
    source: Arc<ImageAnimation>,
    current: Arc<RenderImage>,
    index: usize,
    started_at: Option<Instant>,
    pending: Option<Task<Result<Arc<RenderImage>, ImageCacheError>>>,
    ready: Option<Arc<RenderImage>>,
    error: Option<ImageCacheError>,
}

impl AnimationPlayback {
    pub fn new(poster: Arc<RenderImage>, source: Arc<ImageAnimation>) -> Self {
        Self {
            image_id: poster.id,
            source,
            current: poster,
            index: 0,
            started_at: None,
            pending: None,
            ready: None,
            error: None,
        }
    }

    pub fn update(
        &mut self,
        now: Instant,
        active: bool,
        executor: &BackgroundExecutor,
    ) -> Result<Arc<RenderImage>, ImageCacheError> {
        if let Some(result) = self.pending.as_mut().and_then(|task| task.now_or_never()) {
            self.pending = None;
            match result {
                Ok(image) => self.ready = Some(image),
                Err(error) => self.error = Some(error),
            }
        }
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        if !active {
            self.started_at = None;
            return Ok(self.current.clone());
        }
        let started_at = *self.started_at.get_or_insert(now);
        let delay = Duration::from(self.current.delay(0)).max(Duration::from_millis(10));
        if now - started_at >= delay
            && let Some(image) = self.ready.take()
        {
            self.current = image;
            self.index = (self.index + 1) % self.source.frame_count();
            self.started_at = Some(now);
        }
        if self.pending.is_none() && self.ready.is_none() {
            self.pending = Some(
                self.source
                    .frame((self.index + 1) % self.source.frame_count(), executor),
            );
        }
        Ok(self.current.clone())
    }
}
