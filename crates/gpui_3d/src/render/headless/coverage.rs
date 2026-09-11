use super::{ReadFrame, RenderObject};
use crate::Camera;
use gpui::{Bounds, point, size};
use std::{fmt, sync::Arc};

/// Invalid or unavailable frame data for an object-ID coverage query.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CoverageError {
    MissingObjectIds,
    InvalidSize,
    PixelCount { expected: u64, actual: usize },
    UnknownObjectId { output_id: u32, pixel: [u32; 2] },
    AllocationFailed,
}

impl fmt::Display for CoverageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingObjectIds => f.write_str("frame has no object-ID channel"),
            Self::InvalidSize => f.write_str("coverage requires nonzero frame dimensions"),
            Self::PixelCount { expected, actual } => {
                write!(
                    f,
                    "object-ID channel has {actual} pixels; expected {expected}"
                )
            }
            Self::UnknownObjectId { output_id, pixel } => {
                write!(f, "unknown frame object ID {output_id} at pixel {pixel:?}")
            }
            Self::AllocationFailed => f.write_str("could not allocate object coverage records"),
        }
    }
}
impl std::error::Error for CoverageError {}

#[derive(Clone, Copy, Debug, Default)]
struct Coverage {
    pixels: u64,
    bounds: Option<Bounds<u32>>,
}

/// Coverage of one frame-local object at the output's physical pixel centers.
#[derive(Clone, Copy, Debug)]
pub struct ObjectCoverage<'a> {
    pub object: &'a RenderObject,
    pub pixels: u64,
    /// Sampled pixels divided by the full source output area, not the object's projection.
    pub screen_fraction: f64,
    /// Smallest region-local pixel rectangle enclosing all matching samples.
    /// Right and bottom are exclusive. `None` means there are no matching pixels.
    pub bounds: Option<Bounds<u32>>,
}

/// Owned ID-channel summary retaining the camera and object identities of its frame.
/// No pixel buffers or GPU resources are retained. Zero counts do not distinguish
/// occlusion, clipping, discarded alpha, or subpixel geometry.
#[derive(Clone, Debug)]
pub struct FrameCoverage {
    layout: crate::FrameReadbackLayout,
    frame_id: crate::Scene3dFrameId,
    size: [u32; 2],
    background_pixels: u64,
    camera: Camera,
    objects: Arc<[RenderObject]>,
    coverage: Vec<Coverage>,
}

impl ReadFrame {
    /// Counts the current ID pixels in one CPU pass, without rendering or readback.
    /// Requires a complete object-ID channel matching this frame's identity map.
    /// Retains zero-count objects and reports missing/malformed data as an error.
    /// IDs follow pixel-center nearest-surface coverage, including surviving Blend
    /// fragments; counts are not alpha-weighted color contributions or MSAA coverage.
    pub fn coverage(&self) -> Result<FrameCoverage, CoverageError> {
        let ids = self
            .pixels
            .object_ids
            .as_ref()
            .ok_or(CoverageError::MissingObjectIds)?;
        let [width, height] = self.pixels.size;
        if width == 0 || height == 0 {
            return Err(CoverageError::InvalidSize);
        }
        let expected = u64::from(width) * u64::from(height);
        if ids.len() as u64 != expected {
            return Err(CoverageError::PixelCount {
                expected,
                actual: ids.len(),
            });
        }
        if !self.layout.valid(self.pixels.size) {
            return Err(CoverageError::InvalidSize);
        }
        let mut coverage = Vec::new();
        coverage
            .try_reserve_exact(self.objects.len())
            .map_err(|_| CoverageError::AllocationFailed)?;
        coverage.resize(self.objects.len(), Coverage::default());
        let mut background_pixels = 0;
        for (index, &output_id) in ids.iter().enumerate() {
            if output_id == 0 {
                background_pixels += 1;
                continue;
            }
            let x = (index as u64 % u64::from(width)) as u32;
            let y = (index as u64 / u64::from(width)) as u32;
            let entry = coverage.get_mut((output_id - 1) as usize).ok_or(
                CoverageError::UnknownObjectId {
                    output_id,
                    pixel: [x, y],
                },
            )?;
            entry.pixels += 1;
            entry.bounds = Some(match entry.bounds {
                None => Bounds::new(point(x, y), size(1, 1)),
                Some(bounds) => {
                    let left = bounds.origin.x.min(x);
                    let top = bounds.origin.y.min(y);
                    let right = bounds.right().max(x + 1);
                    let bottom = bounds.bottom().max(y + 1);
                    Bounds::new(point(left, top), size(right - left, bottom - top))
                }
            });
        }
        Ok(FrameCoverage {
            layout: self.layout,
            frame_id: self.frame_id().clone(),
            size: self.pixels.size,
            background_pixels,
            camera: self.camera,
            objects: self.objects.clone(),
            coverage,
        })
    }
}

impl FrameCoverage {
    pub fn layout(&self) -> crate::FrameReadbackLayout {
        self.layout
    }
    pub fn frame_id(&self) -> &crate::Scene3dFrameId {
        &self.frame_id
    }
    pub fn size(&self) -> [u32; 2] {
        self.size
    }

    pub fn camera(&self) -> Camera {
        self.camera
    }

    pub fn pixel_count(&self) -> u64 {
        u64::from(self.size[0]) * u64::from(self.size[1])
    }

    /// Samples whose ID is zero, including uncovered environment background.
    pub fn background_pixels(&self) -> u64 {
        self.background_pixels
    }

    /// Zero and IDs absent from this frame's mapping return `None`. A mapped object
    /// without matching pixels returns a record with zero count and no bounds.
    pub fn object(&self, output_id: u32) -> Option<ObjectCoverage<'_>> {
        let index = output_id.checked_sub(1)? as usize;
        Some(self.record(self.objects.get(index)?, self.coverage.get(index)?))
    }

    /// All mapped objects in frame order, including objects with zero samples.
    pub fn objects(&self) -> impl ExactSizeIterator<Item = ObjectCoverage<'_>> {
        self.objects
            .iter()
            .zip(&self.coverage)
            .map(|(object, coverage)| self.record(object, coverage))
    }

    fn record<'a>(&self, object: &'a RenderObject, coverage: &Coverage) -> ObjectCoverage<'a> {
        ObjectCoverage {
            object,
            pixels: coverage.pixels,
            screen_fraction: coverage.pixels as f64
                / (u64::from(self.layout.output_size[0]) * u64::from(self.layout.output_size[1]))
                    as f64,
            bounds: coverage.bounds,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Node, Scene3dPixels, SceneGraph};

    fn frame(size: [u32; 2], ids: Vec<u32>) -> ReadFrame {
        let mut graph = SceneGraph::new();
        let node = graph.insert(None, Node::new()).unwrap();
        ReadFrame {
            layout: crate::FrameReadbackLayout::full(size),
            frame_id: Default::default(),
            pixels: Scene3dPixels {
                depth_background: Default::default(),
                size,
                object_ids: Some(ids),
                rgba: None,
                linear_rgba: None,
                linear_depth: None,
                world_normals: None,
            },
            camera: Camera::orbit(0.4, 0.2, 8.),
            objects: (0..4)
                .map(|index| RenderObject {
                    output_id: index + 1,
                    object_index: index as usize,
                    node: Some(node),
                    id: (index != 2).then(|| format!("object-{index}").into()),
                })
                .collect::<Vec<_>>()
                .into(),
        }
    }

    #[test]
    fn counts_sparse_samples_and_zero_coverage_objects_with_exclusive_bounds() {
        let frame = frame([4, 3], vec![0, 1, 1, 0, 2, 1, 0, 0, 2, 2, 0, 3]);
        let summary = frame.coverage().unwrap();
        assert_eq!(summary.size(), [4, 3]);
        assert_eq!(summary.pixel_count(), 12);
        assert_eq!(summary.background_pixels(), 5);
        for (id, pixels, bounds) in [
            (1, 3, Some(Bounds::new(point(1, 0), size(2, 2)))),
            (2, 3, Some(Bounds::new(point(0, 1), size(2, 2)))),
            (3, 1, Some(Bounds::new(point(3, 2), size(1, 1)))),
            (4, 0, None),
        ] {
            let entry = summary.object(id).unwrap();
            assert_eq!(entry.pixels, pixels);
            assert_eq!(entry.bounds, bounds);
            assert_eq!(entry.screen_fraction, pixels as f64 / 12.);
            assert_eq!(entry.object.output_id, id);
            assert_eq!(entry.object.node, frame.object(id).unwrap().node);
            assert_eq!(entry.object.id, frame.object(id).unwrap().id);
        }
        assert_eq!(summary.objects().len(), 4);
        assert_eq!(
            summary.objects().map(|entry| entry.pixels).sum::<u64>() + summary.background_pixels(),
            summary.pixel_count()
        );
        assert!(summary.object(0).is_none());
        assert!(summary.object(5).is_none());
        assert!(summary.object(u32::MAX).is_none());
    }

    #[test]
    fn coverage_snapshots_retain_camera_and_ids_without_retaining_pixel_buffers() {
        let mut frame = frame([2, 2], vec![1, 0, 2, 2]);
        let summary = frame.coverage().unwrap();
        let original_camera = frame.camera();
        let original_id = frame.object(1).unwrap().id.clone();
        frame.pixels.object_ids = Some(vec![3; 4]);
        frame.camera = Camera::default();
        frame.objects = (0..3)
            .map(|index| RenderObject {
                output_id: index + 1,
                object_index: index as usize,
                node: None,
                id: Some(format!("replacement-{index}").into()),
            })
            .collect::<Vec<_>>()
            .into();
        let later = frame.coverage().unwrap();
        drop(frame);
        assert_eq!(summary.size(), [2, 2]);
        assert_eq!(
            summary.camera().view_projection(1.).unwrap(),
            original_camera.view_projection(1.).unwrap()
        );
        assert_eq!(summary.object(1).unwrap().object.id, original_id);
        assert_eq!(summary.object(2).unwrap().pixels, 2);
        assert_eq!(later.size(), [2, 2]);
        assert_eq!(later.object(3).unwrap().pixels, 4);
        assert_eq!(
            later.object(3).unwrap().object.id,
            Some("replacement-2".into())
        );
        assert_eq!(later.camera().eye, Camera::default().eye);
    }

    #[test]
    fn derived_results_keep_frame_identity_without_conflating_equal_contents() {
        let first = frame([2, 2], vec![0, 1, 2, 1]);
        let second = frame([2, 2], vec![0, 1, 2, 1]);
        let identity = first.frame_id().clone();
        let coverage = first.coverage().unwrap();
        let labels = first.label_image(4, |object| object.output_id).unwrap();
        assert_eq!(coverage.frame_id(), &identity);
        assert_eq!(labels.frame_id(), &identity);
        assert_ne!(second.coverage().unwrap().frame_id(), &identity);
        drop(first);
        let mut identities = std::collections::HashSet::new();
        identities.insert(identity);
        identities.insert(coverage.frame_id().clone());
        identities.insert(second.frame_id().clone());
        assert_eq!(identities.len(), 2);
        assert!(identities.contains(labels.frame_id()));
    }

    #[test]
    fn mirrored_and_rescaled_id_images_preserve_coverage_fractions() {
        let ids: Vec<_> = (0..5)
            .flat_map(|y| (0..7).map(move |x| (x + 3 * y) % 4))
            .collect();
        let original = frame([7, 5], ids.clone()).coverage().unwrap();
        let mirrored = frame(
            [7, 5],
            ids.chunks_exact(7)
                .flat_map(|row| row.iter().rev().copied())
                .collect(),
        )
        .coverage()
        .unwrap();
        let enlarged_ids = (0..15)
            .flat_map(|y| {
                let ids = &ids;
                (0..21).map(move |x| ids[(y / 3) * 7 + x / 3])
            })
            .collect();
        let enlarged = frame([21, 15], enlarged_ids).coverage().unwrap();
        for entry in original.objects() {
            let mirror = mirrored.object(entry.object.output_id).unwrap();
            let large = enlarged.object(entry.object.output_id).unwrap();
            assert_eq!(mirror.pixels, entry.pixels);
            assert_eq!(large.pixels, entry.pixels * 9);
            assert_eq!(mirror.screen_fraction, entry.screen_fraction);
            assert_eq!(large.screen_fraction, entry.screen_fraction);
            assert_eq!(
                mirror.bounds,
                entry.bounds.map(|bounds| Bounds::new(
                    point(7 - bounds.right(), bounds.origin.y),
                    bounds.size
                ))
            );
            assert_eq!(
                large.bounds,
                entry.bounds.map(|bounds| Bounds::new(
                    point(bounds.origin.x * 3, bounds.origin.y * 3),
                    size(bounds.size.width * 3, bounds.size.height * 3)
                ))
            );
        }
        assert_eq!(
            enlarged.background_pixels(),
            original.background_pixels() * 9
        );
    }

    #[test]
    fn background_only_frame_is_valid_with_or_without_mapped_objects() {
        let mut frame = frame([3, 2], vec![0; 6]);
        let summary = frame.coverage().unwrap();
        assert_eq!(summary.background_pixels(), 6);
        assert!(
            summary
                .objects()
                .all(|entry| entry.pixels == 0 && entry.bounds.is_none())
        );
        frame.objects = Arc::from([]);
        let summary = frame.coverage().unwrap();
        assert_eq!(summary.background_pixels(), 6);
        assert_eq!(summary.objects().len(), 0);
    }

    #[test]
    fn malformed_id_channels_return_errors_instead_of_partial_statistics() {
        let mut frame = frame([2, 2], vec![1, 0, 2, u32::MAX]);
        assert_eq!(
            frame.coverage().unwrap_err(),
            CoverageError::UnknownObjectId {
                output_id: u32::MAX,
                pixel: [1, 1]
            }
        );
        frame.pixels.object_ids = Some(vec![0; 3]);
        assert_eq!(
            frame.coverage().unwrap_err(),
            CoverageError::PixelCount {
                expected: 4,
                actual: 3
            }
        );
        frame.pixels.object_ids = Some(vec![0; 5]);
        assert_eq!(
            frame.coverage().unwrap_err(),
            CoverageError::PixelCount {
                expected: 4,
                actual: 5
            }
        );
        frame.pixels.size = [u32::MAX; 2];
        assert_eq!(
            frame.coverage().unwrap_err(),
            CoverageError::PixelCount {
                expected: u64::from(u32::MAX).pow(2),
                actual: 5
            }
        );
        frame.pixels.size = [0, 2];
        assert_eq!(frame.coverage().unwrap_err(), CoverageError::InvalidSize);
        frame.pixels.object_ids = None;
        assert_eq!(
            frame.coverage().unwrap_err(),
            CoverageError::MissingObjectIds
        );
    }
}
