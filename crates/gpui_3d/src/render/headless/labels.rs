use super::{ReadFrame, RenderObject};
use crate::Camera;
use std::{fmt, sync::Arc};

/// Invalid input or allocation admission for a CPU label image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LabelError {
    MissingObjectIds,
    InvalidSize,
    PixelCount { expected: u64, actual: usize },
    PixelLimit { required: usize, limit: usize },
    UnknownObjectId { output_id: u32, pixel: [u32; 2] },
    AllocationFailed,
}

impl fmt::Display for LabelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingObjectIds => f.write_str("frame has no object-ID channel"),
            Self::InvalidSize => f.write_str("label images require nonzero frame dimensions"),
            Self::PixelCount { expected, actual } => write!(
                f,
                "object-ID channel has {actual} pixels; expected {expected}"
            ),
            Self::PixelLimit { required, limit } => write!(
                f,
                "label image requires {required} pixels; limit is {limit}"
            ),
            Self::UnknownObjectId { output_id, pixel } => {
                write!(f, "unknown frame object ID {output_id} at pixel {pixel:?}")
            }
            Self::AllocationFailed => f.write_str("could not allocate label image data"),
        }
    }
}
impl std::error::Error for LabelError {}

/// Owned, tightly packed top-left-origin u32 labels and their source identities.
/// Zero includes background and explicitly excluded objects. No GPU resources,
/// original pixel buffers, or scene geometry are retained.
#[derive(Debug)]
pub struct FrameLabels {
    layout: crate::FrameReadbackLayout,
    frame_id: crate::Scene3dFrameId,
    size: [u32; 2],
    camera: Camera,
    pixels: Vec<u32>,
    labels: Vec<u32>,
    objects: Arc<[RenderObject]>,
}

impl ReadFrame {
    /// Remaps rendered object IDs to caller-defined labels without rendering or
    /// GPU work. `assign` runs once per mapped object in frame order, including
    /// zero-coverage objects. Repeated labels merge objects; zero excludes them.
    /// Background remains zero. Labels retain all 32 bits and do not index storage.
    /// The pixel limit is checked before allocation or callbacks. Failure returns
    /// no partial image; callback side effects are not rolled back.
    pub fn label_image(
        &self,
        pixel_limit: usize,
        assign: impl FnMut(&RenderObject) -> u32,
    ) -> Result<FrameLabels, LabelError> {
        let ids = self
            .pixels
            .object_ids
            .as_ref()
            .ok_or(LabelError::MissingObjectIds)?;
        let [width, height] = self.pixels.size;
        if width == 0 || height == 0 {
            return Err(LabelError::InvalidSize);
        }
        let expected = u64::from(width) * u64::from(height);
        if ids.len() as u64 != expected {
            return Err(LabelError::PixelCount {
                expected,
                actual: ids.len(),
            });
        }
        if !self.layout.valid(self.pixels.size) {
            return Err(LabelError::InvalidSize);
        }
        if ids.len() > pixel_limit {
            return Err(LabelError::PixelLimit {
                required: ids.len(),
                limit: pixel_limit,
            });
        }
        let mut pixels = Vec::new();
        pixels
            .try_reserve_exact(ids.len())
            .map_err(|_| LabelError::AllocationFailed)?;
        let labels = assign_labels(&self.objects, assign)?;
        for (index, &output_id) in ids.iter().enumerate() {
            let label = if output_id == 0 {
                0
            } else {
                *labels.get((output_id - 1) as usize).ok_or_else(|| {
                    LabelError::UnknownObjectId {
                        output_id,
                        pixel: [
                            (index as u64 % u64::from(width)) as u32,
                            (index as u64 / u64::from(width)) as u32,
                        ],
                    }
                })?
            };
            pixels.push(label);
        }
        Ok(FrameLabels {
            layout: self.layout,
            frame_id: self.frame_id().clone(),
            size: self.pixels.size,
            camera: self.camera,
            pixels,
            labels,
            objects: self.objects.clone(),
        })
    }
}

pub(super) fn assign_labels(
    objects: &[RenderObject],
    mut assign: impl FnMut(&RenderObject) -> u32,
) -> Result<Vec<u32>, LabelError> {
    let mut labels = Vec::new();
    labels
        .try_reserve_exact(objects.len())
        .map_err(|_| LabelError::AllocationFailed)?;
    for object in objects {
        labels.push(assign(object));
    }
    Ok(labels)
}

impl FrameLabels {
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
    /// One u32 label per physical pixel, with no row padding.
    pub fn pixels(&self) -> &[u32] {
        &self.pixels
    }
    /// Moves out the label buffer without copying it.
    pub fn into_pixels(self) -> Vec<u32> {
        self.pixels
    }
    /// Returns zero for background/excluded samples and None outside the image.
    pub fn label_at(&self, x: u32, y: u32) -> Option<u32> {
        if x >= self.size[0] || y >= self.size[1] {
            return None;
        }
        self.pixels
            .get((u64::from(y) * u64::from(self.size[0]) + u64::from(x)) as usize)
            .copied()
    }
    /// Maps an original frame output ID to its assigned label. Zero and unknown
    /// source IDs return None; excluded objects return Some(0).
    pub fn label_for_object(&self, output_id: u32) -> Option<u32> {
        self.labels.get(output_id.checked_sub(1)? as usize).copied()
    }
    /// Source objects assigned this label, in original frame order, including
    /// objects with no pixels. Label zero lists excluded objects, not background.
    pub fn objects(&self, label: u32) -> impl Iterator<Item = &RenderObject> {
        self.objects
            .iter()
            .zip(&self.labels)
            .filter_map(move |(object, &assigned)| (assigned == label).then_some(object))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Node, Scene3dPixels, SceneGraph};

    fn frame() -> ReadFrame {
        let mut graph = SceneGraph::new();
        let nodes = std::array::from_fn::<_, 4, _>(|_| graph.insert(None, Node::new()).unwrap());
        ReadFrame {
            layout: crate::FrameReadbackLayout::full([4, 2]),
            frame_id: Default::default(),
            pixels: Scene3dPixels {
                depth_background: Default::default(),
                size: [4, 2],
                object_ids: Some(vec![0, 1, 2, 3, 2, 0, 1, 3]),
                rgba: None,
                linear_rgba: None,
                linear_depth: None,
                world_normals: None,
            },
            camera: Camera::orbit(0.4, 0.2, 8.),
            objects: nodes
                .into_iter()
                .enumerate()
                .map(|(index, node)| RenderObject {
                    output_id: index as u32 + 1,
                    object_index: index,
                    node: Some(node),
                    id: Some(format!("part-{index}").into()),
                })
                .collect::<Vec<_>>()
                .into(),
        }
    }

    #[test]
    fn merges_object_labels_preserving_full_integer_precision_and_source_ownership() {
        let mut frame = frame();
        let selected = [
            frame.objects[0].node.unwrap(),
            frame.objects[1].node.unwrap(),
        ];
        let mut calls = Vec::new();
        let labels = frame
            .label_image(8, |object| {
                calls.push(object.output_id);
                if object.node.is_some_and(|node| selected.contains(&node)) {
                    u32::MAX
                } else {
                    0
                }
            })
            .unwrap();
        assert_eq!(calls, [1, 2, 3, 4]);
        assert_eq!(
            labels.pixels(),
            &[0, u32::MAX, u32::MAX, 0, u32::MAX, 0, u32::MAX, 0]
        );
        assert_eq!(
            labels
                .objects(u32::MAX)
                .map(|object| object.output_id)
                .collect::<Vec<_>>(),
            [1, 2]
        );
        assert_eq!(
            labels
                .objects(0)
                .map(|object| object.output_id)
                .collect::<Vec<_>>(),
            [3, 4]
        );
        assert_eq!(labels.label_for_object(4), Some(0));
        for id in [0, 5, u32::MAX] {
            assert_eq!(labels.label_for_object(id), None);
        }
        assert_eq!(labels.label_at(3, 0), Some(0));
        assert_eq!(labels.label_at(2, 0), Some(u32::MAX));
        for pixel in [[4, 0], [0, 2], [u32::MAX, u32::MAX]] {
            assert_eq!(labels.label_at(pixel[0], pixel[1]), None);
        }
        let coverage = frame.coverage().unwrap();
        let selected_pixels: u64 = coverage
            .objects()
            .filter(|entry| labels.label_for_object(entry.object.output_id) == Some(u32::MAX))
            .map(|entry| entry.pixels)
            .sum();
        assert_eq!(selected_pixels, 4);
        let retained_camera = frame.camera;
        frame.pixels.object_ids.as_mut().unwrap().fill(4);
        frame.objects = Arc::from([]);
        frame.camera = Camera::default();
        drop(frame);
        let labels = std::thread::spawn(move || labels).join().unwrap();
        assert_eq!(labels.camera(), retained_camera);
        assert_eq!(labels.size(), [4, 2]);
        assert_eq!(
            labels.objects(u32::MAX).next().unwrap().node,
            Some(selected[0])
        );
        let pointer = labels.pixels().as_ptr();
        let pixels = labels.into_pixels();
        assert_eq!(pixels.as_ptr(), pointer);
        assert_eq!(pixels.iter().filter(|&&id| id == u32::MAX).count(), 4);
    }

    #[test]
    fn remapping_preserves_samples_without_revealing_excluded_occluders() {
        let mut frame = frame();
        let original = frame.pixels.object_ids.clone();
        let labels = frame
            .label_image(
                8,
                |object| if object.output_id == 3 { 0 } else { 16_777_217 },
            )
            .unwrap();
        assert_eq!(
            labels.pixels(),
            &[0, 16_777_217, 16_777_217, 0, 16_777_217, 0, 16_777_217, 0]
        );
        assert_eq!(frame.pixels.object_ids, original);
        frame.pixels.object_ids.as_mut().unwrap().fill(0);
        frame.objects = Arc::from([]);
        let empty = frame
            .label_image(8, |_| panic!("no mapped objects"))
            .unwrap();
        assert_eq!(empty.pixels(), &[0; 8]);
    }

    #[test]
    fn validates_frame_layout_and_budget_before_callbacks_and_reports_unknown_ids() {
        let mut frame = frame();
        assert_eq!(
            frame.label_image(7, |_| panic!("over budget")).unwrap_err(),
            LabelError::PixelLimit {
                required: 8,
                limit: 7
            }
        );
        frame.pixels.object_ids.as_mut().unwrap()[7] = 99;
        assert_eq!(
            frame.label_image(8, |_| 1).unwrap_err(),
            LabelError::UnknownObjectId {
                output_id: 99,
                pixel: [3, 1]
            }
        );
        frame.pixels.object_ids.as_mut().unwrap().pop();
        assert_eq!(
            frame
                .label_image(8, |_| panic!("malformed layout"))
                .unwrap_err(),
            LabelError::PixelCount {
                expected: 8,
                actual: 7
            }
        );
        frame.pixels.size = [u32::MAX; 2];
        assert_eq!(
            frame
                .label_image(8, |_| panic!("malformed layout"))
                .unwrap_err(),
            LabelError::PixelCount {
                expected: u64::from(u32::MAX).pow(2),
                actual: 7
            }
        );
        frame.pixels.size = [0, 2];
        assert_eq!(
            frame.label_image(8, |_| panic!("zero size")).unwrap_err(),
            LabelError::InvalidSize
        );
        frame.pixels.object_ids = None;
        assert_eq!(
            frame
                .label_image(8, |_| panic!("missing channel"))
                .unwrap_err(),
            LabelError::MissingObjectIds
        );
    }
}
