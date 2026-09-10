use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
};

use anyhow::{Context, Result, ensure};
use gltf::{
    Semantic,
    accessor::{DataType, Dimensions},
};
use gpui_3d::{
    AffineTransform, EvaluatedScene, Mesh, NodeHandle, Skin, SkinInfluence, SubtreeInstance,
};

use crate::{PreparedDocument, PrimitiveGeometry, geometry::collect};

/// Joint admission for one skin definition.
#[derive(Clone, Copy, Debug)]
pub struct SkinOptions {
    pub joint_limit: usize,
}

impl Default for SkinOptions {
    fn default() -> Self {
        Self {
            joint_limit: 65_536,
        }
    }
}

/// Document-local joint order and inverse bind matrices, independent of geometry.
#[derive(Clone, Debug)]
pub struct SkinDefinition {
    index: usize,
    name: Option<Arc<str>>,
    joints: Arc<[usize]>,
    skeleton: Option<usize>,
    inverse_bind: Arc<[AffineTransform]>,
}

impl SkinDefinition {
    pub fn index(&self) -> usize {
        self.index
    }
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }
    pub fn joints(&self) -> &[usize] {
        &self.joints
    }
    pub fn skeleton(&self) -> Option<usize> {
        self.skeleton
    }
    pub fn inverse_bind_matrices(&self) -> &[AffineTransform] {
        &self.inverse_bind
    }

    /// Binds output-vertex influences from a primitive in the same document.
    /// Skin clones share normalized weights and inverse matrices.
    pub fn bind(&self, geometry: &PrimitiveGeometry) -> Result<Skin> {
        let bind = (|| -> Result<Skin> {
            let data = geometry
                .influences
                .as_ref()
                .context("primitive has no joint/weight attributes")?;
            ensure!(
                data.max_joint < self.joints.len(),
                "joint index {} exceeds {} skin joints",
                data.max_joint,
                self.joints.len()
            );
            Ok(Skin::new(
                self.inverse_bind.iter().copied(),
                data.values
                    .chunks_exact(data.stride)
                    .map(|values| values.iter().copied()),
            )?)
        })();
        bind.with_context(|| {
            format!(
                "skin {} mesh {} primitive {}",
                self.index,
                geometry.mesh_index(),
                geometry.primitive_index()
            )
        })
    }
}

/// A skinned primitive's source-subtree handles and shared bind-space inputs.
#[derive(Clone, Debug)]
pub struct SceneSkin {
    pub(crate) index: usize,
    pub(crate) primitive: NodeHandle,
    pub(crate) joints: Arc<[NodeHandle]>,
    pub(crate) binding: Skin,
    pub(crate) base: Mesh,
}

impl SceneSkin {
    pub fn skin_index(&self) -> usize {
        self.index
    }
    pub fn primitive(&self) -> NodeHandle {
        self.primitive
    }
    pub fn joints(&self) -> &[NodeHandle] {
        &self.joints
    }
    pub fn binding(&self) -> &Skin {
        &self.binding
    }
    pub fn base_mesh(&self) -> &Mesh {
        &self.base
    }

    /// Skins the undeformed base from final world poses without mutating the graph.
    /// The result is local to the returned primitive node. Use `SceneAsset::deform`
    /// to apply imported Morph targets before Skin.
    pub fn evaluate(
        &self,
        instance: &SubtreeInstance,
        poses: &EvaluatedScene,
    ) -> Result<(NodeHandle, Mesh)> {
        self.evaluate_using(|source| instance.node(source), poses)
    }

    pub(crate) fn evaluate_using(
        &self,
        map: impl Fn(NodeHandle) -> Option<NodeHandle>,
        poses: &EvaluatedScene,
    ) -> Result<(NodeHandle, Mesh)> {
        self.evaluate_mesh_using(map, poses, &self.base)
    }

    pub(crate) fn evaluate_mesh_using(
        &self,
        map: impl Fn(NodeHandle) -> Option<NodeHandle>,
        poses: &EvaluatedScene,
        mesh: &Mesh,
    ) -> Result<(NodeHandle, Mesh)> {
        let evaluate = (|| -> Result<_> {
            let primitive = map(self.primitive).context("primitive is absent from the instance")?;
            let mesh_world = poses
                .node(primitive)
                .context("primitive is absent from the pose snapshot")?
                .world;
            let joints = self
                .joints
                .iter()
                .map(|&source| {
                    let handle = map(source).context("joint is absent from the instance")?;
                    Ok(poses
                        .node(handle)
                        .context("joint is absent from the pose snapshot")?
                        .world)
                })
                .collect::<Result<Vec<_>>>()?;
            Ok((
                primitive,
                self.binding.evaluate_world(mesh, mesh_world, &joints)?,
            ))
        })();
        evaluate.with_context(|| format!("skin {} primitive {:?}", self.index, self.primitive))
    }
}

impl PreparedDocument {
    /// Converts joint metadata and float MAT4 inverse binds without loading geometry.
    /// Omitted inverse binds use identity; extra accessor matrices are not retained.
    pub fn skin(&self, index: usize, options: SkinOptions) -> Result<SkinDefinition> {
        let convert = (|| -> Result<_> {
            crate::validation::supported_extensions(self.gltf())?;
            let source = self
                .gltf()
                .skins()
                .nth(index)
                .context("skin index out of range")?;
            let count = source.joints().len();
            ensure!(
                count > 0 && count <= options.joint_limit,
                "joint count {count} must be nonzero and within joint limit"
            );
            let joints: Vec<_> = source.joints().map(|node| node.index()).collect();
            let unique: HashSet<_> = joints.iter().copied().collect();
            ensure!(unique.len() == count, "duplicate joint node");
            let inverse_bind = if let Some(accessor) = source.inverse_bind_matrices() {
                ensure!(
                    accessor.data_type() == DataType::F32
                        && accessor.dimensions() == Dimensions::Mat4
                        && !accessor.normalized(),
                    "inverse bind accessor must contain float MAT4 values"
                );
                ensure!(
                    accessor.count() >= count,
                    "inverse bind accessor has fewer matrices than joints"
                );
                let reader = source.reader(|buffer| self.buffer(buffer.index()));
                let values = reader
                    .read_inverse_bind_matrices()
                    .context("inverse bind data is unavailable")?;
                values
                    .take(count)
                    .enumerate()
                    .map(|(joint, matrix)| {
                        AffineTransform::from_matrix(matrix)
                            .with_context(|| format!("inverse bind matrix {joint}"))
                    })
                    .collect::<Result<Vec<_>>>()?
            } else {
                vec![AffineTransform::IDENTITY; count]
            };
            ensure!(
                inverse_bind.len() == count,
                "inverse bind accessor length mismatch"
            );
            Ok(SkinDefinition {
                index,
                name: source.name().map(Arc::from),
                joints: joints.into(),
                skeleton: source.skeleton().map(|node| node.index()),
                inverse_bind: inverse_bind.into(),
            })
        })();
        convert.with_context(|| format!("skin {index}"))
    }
}

#[derive(Clone, Debug)]
pub(crate) struct VertexInfluences {
    pub values: Arc<[SkinInfluence]>,
    pub stride: usize,
    pub max_joint: usize,
}

pub(crate) fn influences(
    document: &PreparedDocument,
    primitive: &gltf::Primitive<'_>,
    count: usize,
    mapping: &[u32],
    limit: usize,
) -> Result<Option<VertexInfluences>> {
    let mut sets = BTreeMap::new();
    for (semantic, accessor) in primitive.attributes() {
        match semantic {
            Semantic::Joints(set) => sets.entry(set).or_insert((None, None)).0 = Some(accessor),
            Semantic::Weights(set) => sets.entry(set).or_insert((None, None)).1 = Some(accessor),
            _ => {}
        }
    }
    if sets.is_empty() {
        return Ok(None);
    }
    let stride = sets
        .len()
        .checked_mul(4)
        .context("influence stride overflow")?;
    let input_count = count
        .checked_mul(stride)
        .context("input influence count overflow")?;
    let output_count = mapping
        .len()
        .checked_mul(stride)
        .context("output influence count overflow")?;
    ensure!(
        input_count <= limit && output_count <= limit,
        "skin influences exceed influence limit"
    );
    let mut values = vec![
        SkinInfluence {
            joint: 0,
            weight: 0.
        };
        input_count
    ];
    let reader = primitive.reader(|buffer| document.buffer(buffer.index()));
    let mut max_joint = 0;
    for (slot, (set, (joints, weights))) in sets.into_iter().enumerate() {
        ensure!(
            set as usize == slot,
            "joint/weight sets must be consecutive from zero"
        );
        let joints = joints.with_context(|| format!("missing JOINTS_{set}"))?;
        let weights = weights.with_context(|| format!("missing WEIGHTS_{set}"))?;
        let joints = collect(
            &joints,
            reader.read_joints(set).map(|values| values.into_u16()),
        )?;
        let weights = collect(
            &weights,
            reader.read_weights(set).map(|values| values.into_f32()),
        )?;
        for vertex in 0..count {
            for component in 0..4 {
                let joint = joints[vertex][component] as usize;
                let weight = weights[vertex][component];
                ensure!(
                    weight.is_finite() && weight >= 0.,
                    "vertex {vertex} WEIGHTS_{set}: weights must be finite and nonnegative"
                );
                max_joint = max_joint.max(joint);
                values[vertex * stride + slot * 4 + component] = SkinInfluence { joint, weight };
            }
        }
    }
    for (vertex, values) in values.chunks_exact(stride).enumerate() {
        ensure!(
            values.iter().any(|value| value.weight > 0.),
            "vertex {vertex}: skin needs a positive total weight"
        );
    }
    let mut mapped = Vec::with_capacity(output_count);
    for &source in mapping {
        mapped.extend_from_slice(&values[source as usize * stride..(source as usize + 1) * stride]);
    }
    Ok(Some(VertexInfluences {
        values: mapped.into(),
        stride,
        max_joint,
    }))
}
