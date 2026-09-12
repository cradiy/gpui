mod weights;
pub use weights::{WeightPose, WeightPoseError};

use super::{AnimationError, TransformPose, normalize, slerp};
use crate::{AffineTransform, NodeHandle};
use std::{collections::HashMap, fmt, sync::Arc};

/// Invalid node-pose data or blend configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PoseError {
    DuplicateNode(NodeHandle),
    MissingNode(NodeHandle),
    MissingReference(NodeHandle),
    /// `None` identifies a global or default weight; `Some` identifies a mask entry.
    InvalidWeight {
        node: Option<NodeHandle>,
    },
    InvalidPose {
        node: NodeHandle,
        source: AnimationError,
    },
}
impl fmt::Display for PoseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateNode(node) => write!(f, "duplicate pose node {node:?}"),
            Self::MissingNode(node) => write!(f, "node {node:?} has no base pose"),
            Self::MissingReference(node) => write!(f, "node {node:?} has no reference pose"),
            Self::InvalidWeight { node } => write!(
                f,
                "pose weight for {node:?} must be finite and between zero and one"
            ),
            Self::InvalidPose { node, source } => write!(f, "pose node {node:?}: {source}"),
        }
    }
}
impl std::error::Error for PoseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidPose { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl TransformPose {
    /// Blends local TRS toward `target` at a finite weight in [0, 1]. Translation
    /// and signed scale interpolate componentwise; rotation takes the shortest arc.
    /// Both inputs and the result must be invertible. Endpoints retain their exact
    /// input representation; interior quaternion samples are normalized.
    pub fn blend(self, target: Self, weight: f32) -> Result<Self, AnimationError> {
        if !valid_weight(weight) {
            return Err(AnimationError::InvalidBlendWeight);
        }
        self.blend_at(target, f64::from(weight))
    }

    fn blend_at(self, target: Self, weight: f64) -> Result<Self, AnimationError> {
        self.affine().map_err(AnimationError::InvalidTransform)?;
        target.affine().map_err(AnimationError::InvalidTransform)?;
        if weight == 0. {
            return Ok(self);
        }
        if weight == 1. {
            return Ok(target);
        }
        let lerp = |a: [f32; 3], b: [f32; 3]| {
            std::array::from_fn(|i| {
                ((1. - weight) * f64::from(a[i]) + weight * f64::from(b[i])) as f32
            })
        };
        let result = Self {
            translation: lerp(self.translation, target.translation),
            rotation: normalize(slerp(
                self.rotation.map(f64::from),
                target.rotation.map(f64::from),
                weight,
            )?)?
            .map(|v| v as f32),
            scale: lerp(self.scale, target.scale),
        };
        result.affine().map_err(AnimationError::InvalidTransform)?;
        Ok(result)
    }

    /// Applies the change from `reference` to `sample` at a finite weight in [0, 1].
    /// Translation differences use parent coordinates; the shortest-arc relative
    /// rotation is right-multiplied onto this rotation. Scale multiplies by the
    /// weighted componentwise sample/reference ratio. All inputs and the result
    /// must be invertible, including at zero weight.
    pub fn additive(
        self,
        sample: Self,
        reference: Self,
        weight: f32,
    ) -> Result<Self, AnimationError> {
        if !valid_weight(weight) {
            return Err(AnimationError::InvalidBlendWeight);
        }
        self.additive_at(sample, reference, f64::from(weight))
    }

    fn additive_at(
        self,
        sample: Self,
        reference: Self,
        weight: f64,
    ) -> Result<Self, AnimationError> {
        for pose in [self, sample, reference] {
            pose.affine().map_err(AnimationError::InvalidTransform)?;
        }
        if weight == 0. || sample == reference {
            return Ok(self);
        }
        if weight == 1. && self == reference {
            return Ok(sample);
        }
        let reference_rotation = normalize(reference.rotation.map(f64::from))?;
        let inverse_reference = [
            -reference_rotation[0],
            -reference_rotation[1],
            -reference_rotation[2],
            reference_rotation[3],
        ];
        let delta = multiply_rotation(
            inverse_reference,
            normalize(sample.rotation.map(f64::from))?,
        );
        let rotation = multiply_rotation(
            normalize(self.rotation.map(f64::from))?,
            slerp([0., 0., 0., 1.], delta, weight)?,
        );
        let result = Self {
            translation: std::array::from_fn(|i| {
                let mut terms = [
                    f64::from(self.translation[i]),
                    -weight * f64::from(reference.translation[i]),
                    weight * f64::from(sample.translation[i]),
                ];
                terms.sort_by(|a, b| b.abs().total_cmp(&a.abs()));
                terms.into_iter().sum::<f64>() as f32
            }),
            rotation: normalize(rotation)?.map(|v| v as f32),
            scale: std::array::from_fn(|i| {
                (f64::from(self.scale[i])
                    * ((1. - weight)
                        + weight * (f64::from(sample.scale[i]) / f64::from(reference.scale[i]))))
                    as f32
            }),
        };
        result.affine().map_err(AnimationError::InvalidTransform)?;
        Ok(result)
    }
}

fn multiply_rotation(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    let [x, y, z, w] = a;
    let [a, b, c, d] = b;
    [
        w * a + x * d + y * c - z * b,
        w * b - x * c + y * d + z * a,
        w * c + x * b - y * a + z * d,
        w * d - x * a - y * b - z * c,
    ]
}

#[derive(Clone, Copy, Debug)]
struct Entry {
    node: NodeHandle,
    pose: TransformPose,
    affine: AffineTransform,
}
#[derive(Debug, Default)]
struct PoseData {
    entries: Vec<Entry>,
    index: HashMap<NodeHandle, usize>,
}

/// Immutable local TRS poses indexed by stable graph node handles. Clones share
/// storage. Node liveness and graph membership are checked during scene evaluation,
/// not by this collection. No affine decomposition or hierarchy evaluation occurs.
#[derive(Clone, Debug, Default)]
pub struct Pose(Arc<PoseData>);

impl Pose {
    pub fn new(
        poses: impl IntoIterator<Item = (NodeHandle, TransformPose)>,
    ) -> Result<Self, PoseError> {
        let mut data = PoseData::default();
        for (node, pose) in poses {
            if data.index.insert(node, data.entries.len()).is_some() {
                return Err(PoseError::DuplicateNode(node));
            }
            let affine = pose.affine().map_err(|source| PoseError::InvalidPose {
                node,
                source: AnimationError::InvalidTransform(source),
            })?;
            data.entries.push(Entry { node, pose, affine });
        }
        Ok(Self(Arc::new(data)))
    }

    pub fn len(&self) -> usize {
        self.0.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.entries.is_empty()
    }
    pub fn get(&self, node: NodeHandle) -> Option<TransformPose> {
        self.0
            .index
            .get(&node)
            .map(|&index| self.0.entries[index].pose)
    }
    pub fn poses(&self) -> impl ExactSizeIterator<Item = (NodeHandle, TransformPose)> + '_ {
        self.0.entries.iter().map(|entry| (entry.node, entry.pose))
    }
    /// Validated local overrides for `SceneGraph::evaluate_with_transforms` or
    /// `evaluate_with_constraints`, in original insertion order.
    pub fn transforms(&self) -> impl ExactSizeIterator<Item = (NodeHandle, AffineTransform)> + '_ {
        self.0
            .entries
            .iter()
            .map(|entry| (entry.node, entry.affine))
    }

    /// Blends a sparse target into this base. Target and explicit mask nodes must
    /// exist in the base, including at zero weight. Missing target nodes retain
    /// their base pose. Effective weight is `weight * mask.weight(node)`; without
    /// a mask every node uses `weight`. Inputs are never modified on success or error.
    /// Repeated calls form ordered override layers, not a normalized multi-way mean.
    pub fn blend(
        &self,
        target: &Self,
        weight: f32,
        mask: Option<&PoseMask>,
    ) -> Result<Self, PoseError> {
        self.validate_layer(target, weight, mask)?;
        self.apply_layer(target, weight, mask, |base, target, effective| {
            base.blend_at(target.pose, effective)
        })
    }

    /// Adds a sparse sample relative to explicit reference poses. Sample nodes
    /// must exist in both this base and `reference`, even at zero weight. Extra
    /// reference nodes are ignored. Masks use the same local-node rules as `blend`.
    /// The output retains the base's nodes and order; failures leave all inputs
    /// unchanged. See `TransformPose::additive` for the TRS composition convention.
    pub fn additive(
        &self,
        sample: &Self,
        reference: &Self,
        weight: f32,
        mask: Option<&PoseMask>,
    ) -> Result<Self, PoseError> {
        self.validate_layer(sample, weight, mask)?;
        for entry in &sample.0.entries {
            if !reference.0.index.contains_key(&entry.node) {
                return Err(PoseError::MissingReference(entry.node));
            }
        }
        self.apply_layer(sample, weight, mask, |base, sample, effective| {
            base.additive_at(
                sample.pose,
                reference.0.entries[reference.0.index[&sample.node]].pose,
                effective,
            )
        })
    }

    fn validate_layer(
        &self,
        target: &Self,
        weight: f32,
        mask: Option<&PoseMask>,
    ) -> Result<(), PoseError> {
        if !valid_weight(weight) {
            return Err(PoseError::InvalidWeight { node: None });
        }
        for entry in &target.0.entries {
            if !self.0.index.contains_key(&entry.node) {
                return Err(PoseError::MissingNode(entry.node));
            }
        }
        if let Some(mask) = mask {
            for node in mask.nodes.iter() {
                if !self.0.index.contains_key(node) {
                    return Err(PoseError::MissingNode(*node));
                }
            }
        }
        Ok(())
    }

    fn apply_layer(
        &self,
        target: &Self,
        weight: f32,
        mask: Option<&PoseMask>,
        apply: impl Fn(TransformPose, &Entry, f64) -> Result<TransformPose, AnimationError>,
    ) -> Result<Self, PoseError> {
        if weight == 0. || target.is_empty() {
            return Ok(self.clone());
        }
        let mut entries = self.0.entries.clone();
        for target in &target.0.entries {
            let entry = &mut entries[self.0.index[&target.node]];
            let effective =
                f64::from(weight) * f64::from(mask.map_or(1., |mask| mask.weight(target.node)));
            entry.pose =
                apply(entry.pose, target, effective).map_err(|source| PoseError::InvalidPose {
                    node: entry.node,
                    source,
                })?;
            entry.affine = entry
                .pose
                .affine()
                .map_err(|source| PoseError::InvalidPose {
                    node: entry.node,
                    source: AnimationError::InvalidTransform(source),
                })?;
        }
        Ok(Self(Arc::new(PoseData {
            entries,
            index: self.0.index.clone(),
        })))
    }
}

/// Immutable per-node blend weights with an explicit default. Weights are finite
/// values in [0, 1]. A mask applies only to each node's pose or weight array.
/// Parent motion still affects descendants; masks do not expand through the hierarchy.
#[derive(Clone, Debug)]
pub struct PoseMask {
    default_weight: f32,
    weights: Arc<HashMap<NodeHandle, f32>>,
    nodes: Arc<[NodeHandle]>,
}
impl PoseMask {
    pub fn new(
        default_weight: f32,
        weights: impl IntoIterator<Item = (NodeHandle, f32)>,
    ) -> Result<Self, PoseError> {
        if !valid_weight(default_weight) {
            return Err(PoseError::InvalidWeight { node: None });
        }
        let mut entries = HashMap::new();
        let mut nodes = Vec::new();
        for (node, weight) in weights {
            if !valid_weight(weight) {
                return Err(PoseError::InvalidWeight { node: Some(node) });
            }
            if entries.insert(node, weight).is_some() {
                return Err(PoseError::DuplicateNode(node));
            }
            nodes.push(node);
        }
        Ok(Self {
            default_weight,
            weights: Arc::new(entries),
            nodes: nodes.into(),
        })
    }
    pub fn weight(&self, node: NodeHandle) -> f32 {
        self.weights
            .get(&node)
            .copied()
            .unwrap_or(self.default_weight)
    }
}

fn valid_weight(weight: f32) -> bool {
    weight.is_finite() && (0. ..=1.).contains(&weight)
}
