use std::{any::Any, sync::Arc};

use parking_lot::Mutex;

use crate::SharedString;

type CaptureResult = Result<Arc<dyn Any + Send + Sync>, SharedString>;

/// Opt-in publication of a viewport's submitted ID/depth output by its renderer.
/// Use a separate capture for each simultaneous viewport. Consumers must match
/// backend output metadata to the scene frame and layout they intend to query.
#[derive(Clone)]
pub struct Scene3dPickCapture {
    max_bytes: u64,
    output: Arc<Mutex<Option<CaptureResult>>>,
}

impl Scene3dPickCapture {
    /// Maximum output and render-attachment payload for one captured viewport.
    /// Zero rejects allocation. This is not a quota on retained older outputs.
    pub fn new(max_bytes: u64) -> Self {
        Self {
            max_bytes,
            output: Arc::new(Mutex::new(None)),
        }
    }

    /// Configured per-viewport target payload limit.
    pub fn max_bytes(&self) -> u64 {
        self.max_bytes
    }

    /// Returns the latest published output or error. `None` means no publication.
    /// A successful downcast does not establish source-frame or device identity.
    pub fn read<T: Any + Send + Sync>(&self) -> Option<Result<Arc<T>, SharedString>> {
        self.output.lock().clone().map(|result| {
            result.and_then(|output| {
                output
                    .downcast()
                    .map_err(|_| "unsupported 3D pick output backend".into())
            })
        })
    }

    /// Backend publication after successful queue submission, or explicit failure.
    /// Replacing a result does not revoke references retained by consumers.
    pub fn publish<T: Any + Send + Sync>(&self, result: Result<Arc<T>, SharedString>) {
        *self.output.lock() = Some(result.map(|output| output as Arc<dyn Any + Send + Sync>));
    }
}

impl std::fmt::Debug for Scene3dPickCapture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Scene3dPickCapture")
            .field("max_bytes", &self.max_bytes)
            .field("published", &self.output.lock().is_some())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publication_replacement_preserves_retained_outputs() {
        let capture = Scene3dPickCapture::new(1024);
        let consumer = capture.clone();
        assert!(consumer.read::<Vec<u32>>().is_none());
        capture.publish(Ok(Arc::new(vec![3_u32, 7])));
        let retained = consumer.read::<Vec<u32>>().unwrap().unwrap();
        assert!(consumer.read::<String>().unwrap().is_err());
        assert!(Arc::ptr_eq(
            &retained,
            &consumer.read::<Vec<u32>>().unwrap().unwrap()
        ));

        capture.publish::<Vec<u32>>(Err("capture unavailable".into()));
        assert_eq!(
            consumer.read::<Vec<u32>>().unwrap().unwrap_err(),
            SharedString::from("capture unavailable")
        );
        capture.publish(Ok(Arc::new(vec![11_u32])));
        assert_eq!(*consumer.read::<Vec<u32>>().unwrap().unwrap(), vec![11]);
        drop(capture);
        drop(consumer);
        assert_eq!(*retained, vec![3, 7]);
        assert_eq!(Arc::strong_count(&retained), 1);
    }
}
