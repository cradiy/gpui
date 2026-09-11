use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use anyhow::{Context, Result, ensure};
use gltf::{Semantic, accessor::DataType};
use gpui_3d::{
    EvaluatedScene, Mesh, MorphTarget, MorphTargets, NodeHandle, NormalMode, SubtreeInstance,
};

use crate::{PreparedDocument, SceneAsset, ScenePrimitive, SceneSkin};

pub(crate) fn unsupported_attributes(raw: &serde_json::Value) -> HashMap<(usize, usize), String> {
    let mut unsupported = HashMap::new();
    for (mesh, value) in raw["meshes"].as_array().into_iter().flatten().enumerate() {
        for (primitive, value) in value["primitives"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
        {
            for (target, value) in value["targets"]
                .as_array()
                .into_iter()
                .flatten()
                .enumerate()
            {
                if let Some(name) = value
                    .as_object()
                    .into_iter()
                    .flat_map(|object| object.keys())
                    .find(|name| !matches!(name.as_str(), "POSITION" | "NORMAL" | "TANGENT"))
                {
                    unsupported.insert(
                        (mesh, primitive),
                        format!("morph target {target}: unsupported attribute {name}"),
                    );
                    break;
                }
            }
        }
    }
    unsupported
}

/// Shared morph deltas with the primitive's normal/tangent evaluation policy.
#[derive(Clone, Debug)]
pub struct MorphGeometry {
    base: Mesh,
    targets: MorphTargets,
    flat_normals: bool,
    generated_tangents: bool,
    attribute_count: usize,
}

impl MorphGeometry {
    pub fn base_mesh(&self) -> &Mesh {
        &self.base
    }
    pub fn targets(&self) -> &[MorphTarget] {
        self.targets.targets()
    }
    /// Shared, validated inputs for attribute-delta evaluation, before direction regeneration.
    /// Generated tangents are absent from this input mesh. Clone this value for a retained
    /// compute source; inspect the regeneration requirements before using its output directly.
    pub fn attribute_targets(&self) -> &MorphTargets {
        &self.targets
    }

    /// Whether nonzero-weight samples rebuild flat normals from deformed triangle positions.
    pub fn regenerates_normals(&self) -> bool {
        self.flat_normals
    }

    /// Whether nonzero-weight samples rebuild MikkTSpace tangents using the selected UV set.
    pub fn regenerates_tangents(&self) -> bool {
        self.generated_tangents
    }
    /// Total retained VEC3 elements across all target attributes.
    pub fn attribute_vertex_count(&self) -> usize {
        self.attribute_count
    }

    /// Samples signed weights without history. Generated directions are recomputed
    /// from the final geometry while retaining vertex order and index storage.
    pub fn evaluate(&self, weights: &[f32]) -> Result<Mesh> {
        let mut mesh = self.targets.evaluate(weights)?;
        if weights.iter().all(|weight| *weight == 0.) {
            return Ok(self.base.clone());
        }
        if !self.flat_normals && !self.generated_tangents {
            return Ok(mesh);
        }
        if self.flat_normals {
            let generated = mesh.generate_normals(NormalMode::Flat)?;
            ensure!(
                generated
                    .source_vertices()
                    .iter()
                    .enumerate()
                    .all(|(index, &source)| index == source as usize),
                "morph normal generation changed vertex correspondence"
            );
            mesh = generated.into_parts().0;
        }
        if self.generated_tangents {
            let generated = mesh.generate_tangents_for_uv_set(
                self.base
                    .tangent_uv_set()
                    .context("generated tangent set is missing")?,
                gpui_3d::TangentGenerationMode::Repair,
            )?;
            ensure!(
                generated
                    .source_vertices()
                    .iter()
                    .enumerate()
                    .all(|(index, &source)| index == source as usize),
                "morph tangent generation changed vertex correspondence"
            );
            mesh = generated.into_parts().0;
        }
        ensure!(
            mesh.indices() == self.base.indices()
                && mesh.vertex_count() == self.base.vertex_count(),
            "morph generation changed topology"
        );
        Ok(self.base.with_vertices(
            mesh.vertices().to_vec(),
            mesh.tangents().map(|values| values.to_vec()),
        )?)
    }
}

/// Source-subtree association and authored weights for one morphable primitive.
#[derive(Clone, Debug)]
pub struct SceneMorph {
    pub(crate) node_index: usize,
    pub(crate) node: NodeHandle,
    pub(crate) primitive: NodeHandle,
    pub(crate) geometry: MorphGeometry,
    pub(crate) weights: Arc<[f32]>,
}

impl SceneMorph {
    pub fn node_index(&self) -> usize {
        self.node_index
    }
    pub fn node(&self) -> NodeHandle {
        self.node
    }
    pub fn primitive(&self) -> NodeHandle {
        self.primitive
    }
    pub fn geometry(&self) -> &MorphGeometry {
        &self.geometry
    }
    pub fn default_weights(&self) -> &[f32] {
        &self.weights
    }
}

impl SceneAsset {
    /// Returns mesh replacements in primitive order, applying Morph before Skin.
    /// Weight overrides address original-node handles mapped into this instance.
    /// Omitted overrides use authored weights; unknown or duplicate targets fail.
    /// No graph changes occur, including on partial evaluation failure.
    pub fn deform(
        &self,
        instance: &SubtreeInstance,
        poses: &EvaluatedScene,
        weights: &[(NodeHandle, Vec<f32>)],
    ) -> Result<Vec<(NodeHandle, Mesh)>> {
        deform(
            self.primitives(),
            self.skins(),
            self.morphs(),
            |source| instance.node(source),
            poses,
            weights,
        )
    }
}

pub(crate) fn deform(
    primitives: &[ScenePrimitive],
    skins: &[SceneSkin],
    morphs: &[SceneMorph],
    map: impl Fn(NodeHandle) -> Option<NodeHandle>,
    poses: &EvaluatedScene,
    weights: &[(NodeHandle, Vec<f32>)],
) -> Result<Vec<(NodeHandle, Mesh)>> {
    let skins: HashMap<_, _> = skins.iter().map(|skin| (skin.primitive(), skin)).collect();
    let morphs: HashMap<_, _> = morphs
        .iter()
        .map(|morph| (morph.primitive, morph))
        .collect();
    let targets: HashSet<_> = morphs
        .values()
        .map(|morph| map(morph.node).context("morph node is absent from the instance"))
        .collect::<Result<_>>()?;
    let mut overrides = HashMap::new();
    for (node, values) in weights {
        ensure!(
            targets.contains(node),
            "weight target {node:?} is not a morph node in this instance"
        );
        ensure!(
            overrides.insert(*node, values.as_slice()).is_none(),
            "duplicate weight target {node:?}"
        );
    }
    let mut result = Vec::new();
    for primitive in primitives {
        let morph = morphs.get(&primitive.handle);
        let skin = skins.get(&primitive.handle);
        if morph.is_none() && skin.is_none() {
            continue;
        }
        let evaluate = (|| -> Result<_> {
            let handle = map(primitive.handle).context("primitive is absent from the instance")?;
            ensure!(
                poses.node(handle).is_some(),
                "primitive is absent from the pose snapshot"
            );
            let mesh = if let Some(morph) = morph {
                let node = map(morph.node).context("morph node is absent from the instance")?;
                let weights = overrides.get(&node).copied().unwrap_or(&morph.weights);
                Some(morph.geometry.evaluate(weights)?)
            } else {
                None
            };
            let mesh = if let Some(skin) = skin {
                skin.evaluate_mesh_using(&map, poses, mesh.as_ref().unwrap_or(skin.base_mesh()))?
                    .1
            } else {
                mesh.context("missing morph geometry")?
            };
            Ok((handle, mesh))
        })();
        result.push(evaluate.with_context(|| {
            format!(
                "node {} mesh {} primitive {} deformation",
                primitive.node_index, primitive.mesh_index, primitive.primitive_index
            )
        })?);
    }
    Ok(result)
}

pub(crate) fn target_count(mesh: &gltf::Mesh<'_>) -> Result<usize> {
    let mut counts = mesh
        .primitives()
        .map(|primitive| primitive.morph_targets().len());
    let count = counts.next().unwrap_or(0);
    ensure!(
        counts.all(|other| other == count),
        "mesh primitives have different morph target counts"
    );
    if let Some(weights) = mesh.weights() {
        validate_weights(weights, count)?;
    }
    Ok(count)
}

pub(crate) fn default_weights(
    mesh: &gltf::Mesh<'_>,
    node_weights: Option<&[f32]>,
    limit: usize,
) -> Result<Arc<[f32]>> {
    let count = target_count(mesh)?;
    ensure!(count <= limit, "morph target limit exceeded");
    if let Some(weights) = node_weights {
        validate_weights(weights, count)?;
    }
    Ok(node_weights
        .or(mesh.weights())
        .map_or_else(|| vec![0.; count], |weights| weights.to_vec())
        .into())
}

fn validate_weights(weights: &[f32], count: usize) -> Result<()> {
    ensure!(
        count > 0 && weights.len() == count && weights.iter().all(|weight| weight.is_finite()),
        "default morph weights must be finite and match target count {count}"
    );
    Ok(())
}

pub(crate) fn convert(
    document: &PreparedDocument,
    primitive: &gltf::Primitive<'_>,
    base: &Mesh,
    mapping: &[u32],
    flat_normals: bool,
    generated_tangents: bool,
    limit: usize,
) -> Result<Option<MorphGeometry>> {
    if primitive.morph_targets().len() == 0 {
        return Ok(None);
    }
    let count = primitive
        .get(&Semantic::Positions)
        .context("missing base POSITION")?
        .count();
    let mut targets = Vec::new();
    let mut input_count = 0usize;
    let mut output_count = 0usize;
    for (index, target) in primitive.morph_targets().enumerate() {
        let convert = (|| -> Result<_> {
            let mut result = MorphTarget::default();
            for (semantic, accessor, destination, keep) in [
                (
                    Semantic::Positions,
                    target.positions(),
                    &mut result.positions,
                    true,
                ),
                (
                    Semantic::Normals,
                    target.normals(),
                    &mut result.normals,
                    !flat_normals,
                ),
                (
                    Semantic::Tangents,
                    target.tangents(),
                    &mut result.tangents,
                    !flat_normals && !generated_tangents,
                ),
            ] {
                let Some(accessor) = accessor else {
                    continue;
                };
                ensure!(
                    primitive.get(&semantic).is_some(),
                    "{semantic:?} morph attribute has no base attribute"
                );
                if semantic == Semantic::Positions {
                    let bounds = accessor.min().zip(accessor.max()).with_context(|| {
                        format!(
                            "POSITION morph accessor {} requires min and max",
                            accessor.index()
                        )
                    })?;
                    let min: [f32; 3] =
                        serde_json::from_value(bounds.0).context("invalid POSITION morph min")?;
                    let max: [f32; 3] =
                        serde_json::from_value(bounds.1).context("invalid POSITION morph max")?;
                    ensure!(
                        min.into_iter()
                            .zip(max)
                            .all(|(min, max)| min.is_finite() && max.is_finite() && min <= max),
                        "invalid POSITION morph bounds"
                    );
                }
                ensure!(
                    accessor.count() == count
                        && crate::attribute::format(
                            &accessor,
                            &semantic,
                            true,
                            crate::validation::quantization(document.gltf())
                        ),
                    "{semantic:?} accessor {} has an unsupported format or count; expected {count} VEC3 values",
                    accessor.index()
                );
                input_count = input_count
                    .checked_add(count)
                    .context("morph input count overflow")?;
                ensure!(input_count <= limit, "morph input attributes exceed limit");
                if keep {
                    output_count = output_count
                        .checked_add(mapping.len())
                        .context("morph output count overflow")?;
                    ensure!(
                        output_count <= limit,
                        "morph output attributes exceed limit"
                    );
                }
                if accessor.data_type() != DataType::F32 {
                    crate::attribute::alignment(&accessor)?;
                }
                let values = document.vector::<3>(&accessor)?;
                ensure!(
                    values.iter().flatten().all(|value| value.is_finite()),
                    "{semantic:?} contains nonfinite morph deltas"
                );
                if keep {
                    *destination = Some(
                        mapping
                            .iter()
                            .map(|&source| values[source as usize])
                            .collect(),
                    );
                }
            }
            if result.positions.is_none() && result.normals.is_none() && result.tangents.is_none() {
                ensure!(
                    target.tangents().is_some(),
                    "morph target has no supported attributes"
                );
                output_count = output_count
                    .checked_add(mapping.len())
                    .context("morph output count overflow")?;
                ensure!(
                    output_count <= limit,
                    "morph output attributes exceed limit"
                );
                result.positions = Some(vec![[0.; 3]; mapping.len()].into());
            }
            Ok(result)
        })();
        targets.push(convert.with_context(|| format!("morph target {index}"))?);
    }
    let input_base = if generated_tangents {
        base.with_vertices(base.vertices().to_vec(), None)?
    } else {
        base.clone()
    };
    Ok(Some(MorphGeometry {
        base: base.clone(),
        targets: MorphTargets::new(input_base, targets)?,
        flat_normals,
        generated_tangents,
        attribute_count: output_count,
    }))
}
