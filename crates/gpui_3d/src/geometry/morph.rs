use crate::{Mesh, MeshUpdateError, Vertex};
use std::{fmt, sync::Arc};

/// Per-vertex mesh-local attribute deltas. Every supplied array includes unused
/// vertices. Omitted attributes contribute zero; tangent deltas contain XYZ only.
#[derive(Clone, Debug, Default)]
pub struct MorphTarget {
    /// Position offsets in mesh-local units.
    pub positions: Option<Arc<[[f32; 3]]>>,
    /// Offsets added to base vertex normals before normalization.
    pub normals: Option<Arc<[[f32; 3]]>>,
    /// Offsets added to base tangent XYZ; handedness W is unchanged.
    pub tangents: Option<Arc<[[f32; 3]]>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MorphAttribute {
    Position,
    Normal,
    Tangent,
}

/// Invalid morph data or sampled geometry. All offsets are zero-based.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MorphError {
    VertexCount {
        expected: usize,
        actual: usize,
    },
    VertexIndex {
        vertex: usize,
        vertex_count: usize,
    },
    EmptyTarget {
        target: usize,
    },
    AttributeCount {
        target: usize,
        attribute: MorphAttribute,
        expected: usize,
        actual: usize,
    },
    NonFiniteDelta {
        target: usize,
        attribute: MorphAttribute,
        vertex: usize,
        component: usize,
    },
    MissingBaseTangents {
        target: usize,
    },
    WeightCount {
        expected: usize,
        actual: usize,
    },
    NonFiniteWeight {
        target: usize,
    },
    UnrepresentableResult {
        vertex: usize,
        attribute: MorphAttribute,
    },
    Mesh(MeshUpdateError),
}

impl fmt::Display for MorphError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::VertexCount { expected, actual } => write!(
                f,
                "expected {expected} mapped morph vertices, received {actual}"
            ),
            Self::VertexIndex {
                vertex,
                vertex_count,
            } => write!(
                f,
                "morph source vertex {vertex} is outside {vertex_count} vertices"
            ),
            Self::EmptyTarget { target } => write!(f, "morph target {target} has no attributes"),
            Self::AttributeCount {
                target,
                attribute,
                expected,
                actual,
            } => write!(
                f,
                "morph target {target} {attribute:?}: expected {expected} vertices, received {actual}"
            ),
            Self::NonFiniteDelta {
                target,
                attribute,
                vertex,
                component,
            } => write!(
                f,
                "morph target {target} has a nonfinite {attribute:?} delta at vertex {vertex}, component {component}"
            ),
            Self::MissingBaseTangents { target } => {
                write!(f, "morph target {target} requires base mesh tangents")
            }
            Self::WeightCount { expected, actual } => {
                write!(f, "expected {expected} morph weights, received {actual}")
            }
            Self::NonFiniteWeight { target } => write!(f, "morph weight {target} is nonfinite"),
            Self::UnrepresentableResult { vertex, attribute } => write!(
                f,
                "morphed {attribute:?} at vertex {vertex} is outside finite f32 range"
            ),
            Self::Mesh(source) => write!(f, "morphed mesh: {source}"),
        }
    }
}

impl std::error::Error for MorphError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Mesh(source) => Some(source),
            _ => None,
        }
    }
}

/// Validated immutable targets bound to one base mesh. Clones share all inputs.
/// Evaluation is synchronous and CPU-only; the caller owns weights and scheduling.
#[derive(Clone, Debug)]
pub struct MorphTargets {
    base: Mesh,
    targets: Arc<[MorphTarget]>,
}

impl MorphTargets {
    pub fn new(
        base: Mesh,
        targets: impl IntoIterator<Item = MorphTarget>,
    ) -> Result<Self, MorphError> {
        let targets: Vec<_> = targets.into_iter().collect();
        for (target, data) in targets.iter().enumerate() {
            if data.positions.is_none() && data.normals.is_none() && data.tangents.is_none() {
                return Err(MorphError::EmptyTarget { target });
            }
            for (attribute, values) in [
                (MorphAttribute::Position, &data.positions),
                (MorphAttribute::Normal, &data.normals),
                (MorphAttribute::Tangent, &data.tangents),
            ] {
                let Some(values) = values else {
                    continue;
                };
                if values.len() != base.vertex_count() {
                    return Err(MorphError::AttributeCount {
                        target,
                        attribute,
                        expected: base.vertex_count(),
                        actual: values.len(),
                    });
                }
                for (vertex, value) in values.iter().enumerate() {
                    if let Some(component) = value.iter().position(|v| !v.is_finite()) {
                        return Err(MorphError::NonFiniteDelta {
                            target,
                            attribute,
                            vertex,
                            component,
                        });
                    }
                }
            }
            if data.tangents.is_some() && base.tangents().is_none() {
                return Err(MorphError::MissingBaseTangents { target });
            }
        }
        Ok(Self {
            base,
            targets: targets.into(),
        })
    }

    pub fn base_mesh(&self) -> &Mesh {
        &self.base
    }

    pub fn targets(&self) -> &[MorphTarget] {
        &self.targets
    }

    /// Rebinds targets to a mesh using one output-to-source index per new vertex.
    /// Reordering, duplication and subsets preserve target order, absent attributes
    /// and stored delta bits. The caller supplies the matching base attributes;
    /// this operation neither remaps the base nor transforms deltas between spaces.
    /// Tangent deltas require tangents on the supplied base. Validation precedes
    /// target allocation; an identity map shares the original delta storage.
    pub fn remap_vertices(&self, base: Mesh, source_vertices: &[u32]) -> Result<Self, MorphError> {
        if source_vertices.len() != base.vertex_count() {
            return Err(MorphError::VertexCount {
                expected: base.vertex_count(),
                actual: source_vertices.len(),
            });
        }
        for &vertex in source_vertices {
            if vertex as usize >= self.base.vertex_count() {
                return Err(MorphError::VertexIndex {
                    vertex: vertex as usize,
                    vertex_count: self.base.vertex_count(),
                });
            }
        }
        if base.tangents().is_none()
            && let Some(target) = self
                .targets
                .iter()
                .position(|target| target.tangents.is_some())
        {
            return Err(MorphError::MissingBaseTangents { target });
        }
        let identity = source_vertices.len() == self.base.vertex_count()
            && source_vertices
                .iter()
                .enumerate()
                .all(|(index, &source)| index == source as usize);
        let remap = |values: &Option<Arc<[[f32; 3]]>>| {
            values.as_ref().map(|values| {
                source_vertices
                    .iter()
                    .map(|&source| values[source as usize])
                    .collect()
            })
        };
        let targets = if identity {
            self.targets.clone()
        } else {
            self.targets
                .iter()
                .map(|target| MorphTarget {
                    positions: remap(&target.positions),
                    normals: remap(&target.normals),
                    tangents: remap(&target.tangents),
                })
                .collect()
        };
        Ok(Self { base, targets })
    }

    /// Evaluates `base + sum(weight * delta)` before node or skin transforms.
    /// Weights must be finite and match the target count; they are neither clamped
    /// nor normalized. Zero weights return the shared base mesh. Normals with
    /// active deltas are normalized after blending; zero normals remain zero. Tangents
    /// are orthogonalized against the result, preserving base handedness.
    /// Undefined tangent bases and unrepresentable positions return errors.
    pub fn evaluate(&self, weights: &[f32]) -> Result<Mesh, MorphError> {
        if weights.len() != self.targets.len() {
            return Err(MorphError::WeightCount {
                expected: self.targets.len(),
                actual: weights.len(),
            });
        }
        for (target, weight) in weights.iter().enumerate() {
            if !weight.is_finite() {
                return Err(MorphError::NonFiniteWeight { target });
            }
        }
        let active: Vec<_> = self
            .targets
            .iter()
            .zip(weights)
            .filter_map(|(target, &weight)| (weight != 0.).then_some((target, f64::from(weight))))
            .collect();
        if active.is_empty() {
            return Ok(self.base.clone());
        }
        let blend_normals = active.iter().any(|(target, _)| target.normals.is_some());
        let mut vertices = Vec::with_capacity(self.base.vertex_count());
        let mut tangents = self
            .base
            .tangents()
            .map(|_| Vec::with_capacity(self.base.vertex_count()));
        for (vertex, base) in self.base.vertices().iter().enumerate() {
            let mut position = base.position.map(f64::from);
            let mut normal = base.normal.map(f64::from);
            let base_tangent = self.base.tangents().map(|t| t[vertex]);
            let mut tangent = base_tangent
                .map(|t| [t[0], t[1], t[2]].map(f64::from))
                .unwrap_or([0.; 3]);
            for &(target, weight) in &active {
                for (value, deltas) in [
                    (&mut position, &target.positions),
                    (&mut normal, &target.normals),
                    (&mut tangent, &target.tangents),
                ] {
                    if let Some(deltas) = deltas {
                        for i in 0..3 {
                            value[i] += weight * f64::from(deltas[vertex][i]);
                        }
                    }
                }
            }
            if blend_normals {
                normal = normalize(normal);
            }
            vertices.push(Vertex {
                position: finite(position, vertex, MorphAttribute::Position)?,
                normal: finite(normal, vertex, MorphAttribute::Normal)?,
                uv: base.uv,
            });
            if let Some(tangents) = &mut tangents {
                let [x, y, z] = finite(normalize(tangent), vertex, MorphAttribute::Tangent)?;
                tangents.push([x, y, z, base_tangent.unwrap()[3]]);
            }
        }
        self.base
            .with_vertices(vertices, tangents)
            .map_err(MorphError::Mesh)
    }
}

fn normalize(value: [f64; 3]) -> [f64; 3] {
    let length = value.iter().map(|v| v * v).sum::<f64>().sqrt();
    if length > 0. {
        value.map(|v| v / length)
    } else {
        value
    }
}

fn finite(
    value: [f64; 3],
    vertex: usize,
    attribute: MorphAttribute,
) -> Result<[f32; 3], MorphError> {
    let value = value.map(|v| v as f32);
    if value.iter().all(|v| v.is_finite()) {
        Ok(value)
    } else {
        Err(MorphError::UnrepresentableResult { vertex, attribute })
    }
}
