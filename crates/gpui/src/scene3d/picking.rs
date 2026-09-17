use std::{
    any::Any,
    sync::{Arc, Weak},
};

use parking_lot::Mutex;

use crate::{Scene3dFrame, SharedString};

type CaptureResult = Result<Arc<dyn Any + Send + Sync>, SharedString>;

struct Publication {
    frame: Weak<Scene3dFrame>,
    result: CaptureResult,
}

/// Opt-in publication of a viewport's submitted ID/depth output by its renderer.
/// Use a separate capture for each simultaneous viewport. Consumers must match
/// backend output metadata to the scene frame and layout they intend to query.
#[derive(Clone)]
pub struct Scene3dPickCapture {
    max_bytes: u64,
    output: Arc<Mutex<Option<Publication>>>,
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
        self.read_matching(None)
    }

    /// Returns the latest publication only if it belongs to this exact source frame.
    /// Both outputs and errors from other frames return `None`.
    pub fn read_for_frame<T: Any + Send + Sync>(
        &self,
        frame: &Arc<Scene3dFrame>,
    ) -> Option<Result<Arc<T>, SharedString>> {
        self.read_matching(Some(frame))
    }

    fn read_matching<T: Any + Send + Sync>(
        &self,
        frame: Option<&Arc<Scene3dFrame>>,
    ) -> Option<Result<Arc<T>, SharedString>> {
        let publication = self.output.lock();
        let publication = publication.as_ref()?;
        if frame.is_some_and(|frame| !publication.frame.ptr_eq(&Arc::downgrade(frame))) {
            return None;
        }
        Some(publication.result.clone().and_then(|output| {
            output
                .downcast()
                .map_err(|_| "unsupported 3D pick output backend".into())
        }))
    }

    /// Backend publication after successful queue submission, or explicit failure.
    /// The source frame is held weakly. Replacing a result does not revoke references
    /// retained by consumers.
    pub fn publish<T: Any + Send + Sync>(
        &self,
        frame: &Arc<Scene3dFrame>,
        result: Result<Arc<T>, SharedString>,
    ) {
        *self.output.lock() = Some(Publication {
            frame: Arc::downgrade(frame),
            result: result.map(|output| output as Arc<dyn Any + Send + Sync>),
        });
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

    fn frame(capture: &Scene3dPickCapture) -> Arc<Scene3dFrame> {
        let identity = [
            [1., 0., 0., 0.],
            [0., 1., 0., 0.],
            [0., 0., 1., 0.],
            [0., 0., 0., 1.],
        ];
        Arc::new(Scene3dFrame {
            pick_capture: Some(capture.clone()),
            occlusion_groups: Default::default(),
            depth_background: Default::default(),
            viewport_quality: Default::default(),
            ui_texture: None,
            view_projection: identity,
            world_to_view: identity,
            camera_position: [0., 0., 3.],
            orthographic_view_direction: None,
            light_direction: [0., 0., 1.],
            light: [1.; 4],
            lights: None,
            directional_shadow: None,
            ambient: 0.3,
            diffuse_environment: None,
            background: None,
            specular_environment: None,
            color_output: Default::default(),
            objects: Arc::default(),
        })
    }

    #[test]
    fn publication_replacement_preserves_retained_outputs() {
        let capture = Scene3dPickCapture::new(1024);
        let consumer = capture.clone();
        let frame = frame(&capture);
        let source = Arc::downgrade(&frame);
        assert!(consumer.read::<Vec<u32>>().is_none());
        capture.publish(&frame, Ok(Arc::new(vec![3_u32, 7])));
        let retained = consumer.read::<Vec<u32>>().unwrap().unwrap();
        assert!(consumer.read::<String>().unwrap().is_err());
        assert!(Arc::ptr_eq(
            &retained,
            &consumer.read::<Vec<u32>>().unwrap().unwrap()
        ));

        capture.publish::<Vec<u32>>(&frame, Err("capture unavailable".into()));
        assert_eq!(
            consumer.read::<Vec<u32>>().unwrap().unwrap_err(),
            SharedString::from("capture unavailable")
        );
        capture.publish(&frame, Ok(Arc::new(vec![11_u32])));
        assert_eq!(*consumer.read::<Vec<u32>>().unwrap().unwrap(), vec![11]);
        drop(frame);
        assert!(source.upgrade().is_none());
        drop(capture);
        drop(consumer);
        assert_eq!(*retained, vec![3, 7]);
        assert_eq!(Arc::strong_count(&retained), 1);
    }

    #[test]
    fn frame_matching_filters_errors_and_outputs_before_backend_downcast() {
        let capture = Scene3dPickCapture::new(1024);
        let first = frame(&capture);
        let second = Arc::new((*first).clone());
        capture.publish(&first, Ok(Arc::new(vec![7_u32])));
        let retained = capture.read_for_frame::<Vec<u32>>(&first).unwrap().unwrap();
        assert!(capture.read_for_frame::<String>(&second).is_none());
        capture.publish::<Vec<u32>>(&first, Err("first frame failed".into()));
        assert!(capture.read_for_frame::<Vec<u32>>(&first).unwrap().is_err());
        assert!(capture.read_for_frame::<Vec<u32>>(&second).is_none());
        capture.publish::<Vec<u32>>(&second, Err("second frame failed".into()));
        assert!(capture.read_for_frame::<Vec<u32>>(&first).is_none());
        assert_eq!(
            capture
                .read_for_frame::<Vec<u32>>(&second)
                .unwrap()
                .unwrap_err(),
            SharedString::from("second frame failed"),
        );
        capture.publish(&second, Ok(Arc::new(vec![11_u32])));
        assert_eq!(
            *capture
                .read_for_frame::<Vec<u32>>(&second)
                .unwrap()
                .unwrap(),
            vec![11]
        );
        assert!(capture.read_for_frame::<Vec<u32>>(&first).is_none());
        assert_eq!(*retained, vec![7]);
    }
}
