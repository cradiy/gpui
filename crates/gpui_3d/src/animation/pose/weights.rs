use super::{PoseMask, valid_weight};
use crate::NodeHandle;
use std::{collections::HashMap, fmt, sync::Arc};

/// Invalid node weight arrays or a layer that cannot be represented.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WeightPoseError {
    DuplicateNode(NodeHandle),
    MissingNode(NodeHandle),
    MissingReference(NodeHandle),
    EmptyWeights(NodeHandle),
    WeightCount {
        node: NodeHandle,
        expected: usize,
        actual: usize,
    },
    InvalidValue {
        node: NodeHandle,
        component: usize,
    },
    InvalidBlendWeight,
    Unrepresentable {
        node: NodeHandle,
        component: usize,
    },
}

impl fmt::Display for WeightPoseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateNode(node) => write!(f, "duplicate weight pose node {node:?}"),
            Self::MissingNode(node) => write!(f, "node {node:?} has no base weights"),
            Self::MissingReference(node) => write!(f, "node {node:?} has no reference weights"),
            Self::EmptyWeights(node) => write!(f, "node {node:?} requires nonempty weights"),
            Self::WeightCount {
                node,
                expected,
                actual,
            } => write!(
                f,
                "node {node:?} requires {expected} weights, received {actual}"
            ),
            Self::InvalidValue { node, component } => {
                write!(f, "node {node:?} weight {component} must be finite")
            }
            Self::InvalidBlendWeight => {
                f.write_str("blend weight must be finite and between zero and one")
            }
            Self::Unrepresentable { node, component } => write!(
                f,
                "node {node:?} blended weight {component} is outside finite f32 range"
            ),
        }
    }
}
impl std::error::Error for WeightPoseError {}

#[derive(Debug, Default)]
struct WeightData {
    entries: Vec<(NodeHandle, Vec<f32>)>,
    index: HashMap<NodeHandle, usize>,
}

/// Immutable signed weight arrays indexed by graph-scoped node handles.
/// Clones share storage. Arrays are nonempty and finite, but neither clamped nor
/// normalized. The consumer validates node liveness and geometry compatibility.
#[derive(Clone, Debug, Default)]
pub struct WeightPose(Arc<WeightData>);

impl WeightPose {
    pub fn new(
        weights: impl IntoIterator<Item = (NodeHandle, Vec<f32>)>,
    ) -> Result<Self, WeightPoseError> {
        let mut data = WeightData::default();
        for (node, values) in weights {
            if data.index.insert(node, data.entries.len()).is_some() {
                return Err(WeightPoseError::DuplicateNode(node));
            }
            if values.is_empty() {
                return Err(WeightPoseError::EmptyWeights(node));
            }
            if let Some(component) = values.iter().position(|value| !value.is_finite()) {
                return Err(WeightPoseError::InvalidValue { node, component });
            }
            data.entries.push((node, values));
        }
        Ok(Self(Arc::new(data)))
    }

    pub fn len(&self) -> usize {
        self.0.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.entries.is_empty()
    }
    pub fn get(&self, node: NodeHandle) -> Option<&[f32]> {
        self.0
            .index
            .get(&node)
            .map(|&index| self.0.entries[index].1.as_slice())
    }
    /// Node arrays in original insertion order, suitable for deformation inputs.
    pub fn weights(&self) -> &[(NodeHandle, Vec<f32>)] {
        &self.0.entries
    }
    /// Takes the arrays, copying them only when this collection still has clones.
    pub fn into_weights(self) -> Vec<(NodeHandle, Vec<f32>)> {
        Arc::try_unwrap(self.0).map_or_else(|shared| shared.entries.clone(), |data| data.entries)
    }

    /// Applies a sparse override layer with `base * (1 - t) + target * t`.
    /// Layer and explicit mask nodes must exist in the base, even at zero weight.
    /// Arrays must match the base component count. Omitted nodes retain base values.
    /// `t` is `weight * mask.weight(node)`, or `weight` without a mask. The global
    /// weight must be finite and in [0, 1]. Inputs are unchanged on success or error.
    pub fn blend(
        &self,
        target: &Self,
        weight: f32,
        mask: Option<&PoseMask>,
    ) -> Result<Self, WeightPoseError> {
        self.combine(target, None, weight, mask)
    }

    /// Applies `base + (sample - reference) * t` componentwise.
    /// Every sample node needs equally sized base and reference arrays, even at
    /// zero weight. Extra reference nodes are ignored. All inputs remain unchanged.
    pub fn additive(
        &self,
        sample: &Self,
        reference: &Self,
        weight: f32,
        mask: Option<&PoseMask>,
    ) -> Result<Self, WeightPoseError> {
        self.combine(sample, Some(reference), weight, mask)
    }

    fn combine(
        &self,
        target: &Self,
        reference: Option<&Self>,
        weight: f32,
        mask: Option<&PoseMask>,
    ) -> Result<Self, WeightPoseError> {
        if !valid_weight(weight) {
            return Err(WeightPoseError::InvalidBlendWeight);
        }
        for (node, values) in &target.0.entries {
            let base = self.get(*node).ok_or(WeightPoseError::MissingNode(*node))?;
            if base.len() != values.len() {
                return Err(WeightPoseError::WeightCount {
                    node: *node,
                    expected: base.len(),
                    actual: values.len(),
                });
            }
            if let Some(reference) = reference {
                let reference = reference
                    .get(*node)
                    .ok_or(WeightPoseError::MissingReference(*node))?;
                if base.len() != reference.len() {
                    return Err(WeightPoseError::WeightCount {
                        node: *node,
                        expected: base.len(),
                        actual: reference.len(),
                    });
                }
            }
        }
        if let Some(mask) = mask {
            for node in mask.nodes.iter() {
                if !self.0.index.contains_key(node) {
                    return Err(WeightPoseError::MissingNode(*node));
                }
            }
        }
        if weight == 0. || target.is_empty() {
            return Ok(self.clone());
        }
        let mut entries = self.0.entries.clone();
        for (node, values) in &target.0.entries {
            let effective =
                f64::from(weight) * f64::from(mask.map_or(1., |mask| mask.weight(*node)));
            if effective == 0. {
                continue;
            }
            let output = &mut entries[self.0.index[node]].1;
            let reference = reference.and_then(|reference| reference.get(*node));
            for (component, (base, sample)) in output.iter_mut().zip(values).enumerate() {
                let value = if let Some(reference) = reference {
                    if *sample == reference[component] {
                        continue;
                    }
                    (f64::from(*base)
                        + (f64::from(*sample) - f64::from(reference[component])) * effective)
                        as f32
                } else if effective == 1. {
                    *sample
                } else {
                    (f64::from(*base) * (1. - effective) + f64::from(*sample) * effective) as f32
                };
                if !value.is_finite() {
                    return Err(WeightPoseError::Unrepresentable {
                        node: *node,
                        component,
                    });
                }
                *base = value;
            }
        }
        Ok(Self(Arc::new(WeightData {
            entries,
            index: self.0.index.clone(),
        })))
    }
}
