use std::collections::HashMap;

use anyhow::{Context, Result, ensure};
use gpui_3d::{
    EvaluatedScene, GpuDeformationLimits, GpuDeformationOutput, GpuFlatNormals, GpuMorph, GpuSkin,
    GpuTangentGeneration, NodeHandle, SubtreeInstance, TangentGenerationMode, WgpuContext,
};

use crate::{SceneAsset, SceneMorph, ScenePrimitive, SceneSkin, morph::resolve_weights};

mod memory;
mod source_memory;
pub use memory::GpuSceneEvaluationMemory;
pub use source_memory::GpuSceneSourceMemory;

/// Retained imported Morph and Skin sources on one device, reusable across scene instances.
/// Evaluation submits immutable outputs without reading vertices or changing the CPU scene.
pub struct GpuSceneDeformation {
    primitives: Vec<PrimitiveSource>,
    memory: GpuSceneSourceMemory,
}

struct PrimitiveSource {
    primitive: ScenePrimitive,
    morph: Option<MorphSource>,
    skin: Option<(SceneSkin, GpuSkin)>,
    base: Option<GpuDeformationOutput>,
}

struct MorphSource {
    source: SceneMorph,
    gpu: GpuMorph,
    normals: Option<GpuFlatNormals>,
    tangents: Option<(GpuTangentGeneration, GpuDeformationOutput)>,
}

impl GpuSceneDeformation {
    /// Checks imported direction topology and coordinate metadata without GPU allocation.
    /// Device limits, payload budgets, and arithmetic validity are checked separately.
    pub fn check_asset(asset: &SceneAsset) -> Result<()> {
        for morph in asset.morphs() {
            let geometry = morph.geometry();
            let input = geometry.attribute_targets().base_mesh();
            if geometry.regenerates_normals() || geometry.regenerates_tangents() {
                ensure!(
                    input.vertex_count() == input.index_count()
                        && input
                            .indices()
                            .iter()
                            .enumerate()
                            .all(|(i, &v)| i == v as usize),
                    "node {} primitive {:?}: GPU direction regeneration requires ordered triangle corners",
                    morph.node_index(),
                    morph.primitive(),
                );
            }
            if geometry.regenerates_tangents() {
                let set = geometry
                    .base_mesh()
                    .tangent_uv_set()
                    .context("generated tangent coordinate set is missing")?;
                ensure!(
                    input.uv_at(set, 0).is_some(),
                    "generated tangent coordinates are missing"
                );
            }
        }
        Ok(())
    }

    /// Uploads each deformable primitive after checking imported direction policies,
    /// core payload limits and the optional aggregate retained-source budget.
    /// The adapter retains geometry and bindings, but not materials or decoded images.
    pub fn new(
        context: WgpuContext,
        asset: &SceneAsset,
        limits: GpuDeformationLimits,
        max_source_bytes: Option<u64>,
    ) -> Result<Self> {
        let memory = GpuSceneSourceMemory::plan(asset, limits, max_source_bytes)?;
        let morphs: HashMap<_, _> = asset
            .morphs()
            .iter()
            .map(|morph| (morph.primitive(), morph))
            .collect();
        let skins: HashMap<_, _> = asset
            .skins()
            .iter()
            .map(|skin| (skin.primitive(), skin))
            .collect();
        let mut primitives = Vec::new();
        for &primitive in asset.primitives() {
            let morph = morphs.get(&primitive.handle);
            let skin = skins.get(&primitive.handle);
            if morph.is_none() && skin.is_none() {
                continue;
            }
            let prepare = (|| -> Result<_> {
                let morph = morph
                    .map(|source| {
                        let geometry = source.geometry();
                        let normals = geometry
                            .regenerates_normals()
                            .then(|| {
                                GpuFlatNormals::new(
                                    context.clone(),
                                    geometry.attribute_targets().base_mesh().clone(),
                                    limits,
                                )
                            })
                            .transpose()?;
                        let tangents = geometry
                            .regenerates_tangents()
                            .then(|| {
                                let source = GpuTangentGeneration::new(
                                    context.clone(),
                                    geometry.attribute_targets().base_mesh().clone(),
                                    geometry
                                        .base_mesh()
                                        .tangent_uv_set()
                                        .context("generated tangent coordinate set is missing")?,
                                    TangentGenerationMode::Repair,
                                    limits,
                                )?;
                                let bind = GpuDeformationOutput::upload(
                                    context.clone(),
                                    geometry.base_mesh().clone(),
                                    limits,
                                )?;
                                Ok::<_, anyhow::Error>((source, bind))
                            })
                            .transpose()?;
                        Ok::<_, anyhow::Error>(MorphSource {
                            source: (*source).clone(),
                            gpu: GpuMorph::new(
                                context.clone(),
                                geometry.attribute_targets().clone(),
                                limits,
                            )?,
                            normals,
                            tangents,
                        })
                    })
                    .transpose()?;
                let base = skin
                    .filter(|_| morph.is_none())
                    .map(|skin| {
                        GpuDeformationOutput::upload(
                            context.clone(),
                            skin.base_mesh().clone(),
                            limits,
                        )
                    })
                    .transpose()?;
                let skin = skin
                    .map(|source| {
                        Ok::<_, anyhow::Error>((
                            (*source).clone(),
                            GpuSkin::new(context.clone(), source.binding().clone(), limits)?,
                        ))
                    })
                    .transpose()?;
                Ok(PrimitiveSource {
                    primitive,
                    morph,
                    skin,
                    base,
                })
            })();
            primitives.push(prepare.with_context(|| description(primitive))?);
        }
        Ok(Self { primitives, memory })
    }

    /// Buffer payload admitted before source upload, excluding evaluation results.
    pub fn source_memory(&self) -> GpuSceneSourceMemory {
        self.memory
    }

    /// Submits Morph, required normal/tangent regeneration, then Skin in primitive order.
    /// Overrides address mapped original nodes; omitted values use authored defaults.
    /// Every call starts from bind-space inputs, independently of earlier samples.
    /// Returned handles address mapped primitive children. Their output `base_mesh()`
    /// is the source identity required by GPU render packing and scene mesh overrides.
    /// CPU bounds and queries are not updated. No CPU fallback is performed.
    /// The optional budget bounds the sum of new GPU evaluation payloads across
    /// all primitives, including intermediate stages, before the first dispatch.
    /// All mapped Skin palettes are composed and validated on the CPU before
    /// admission and GPU work. Upload and shader failures can still occur later.
    pub fn evaluate(
        &self,
        instance: &SubtreeInstance,
        poses: &EvaluatedScene,
        weights: &[(NodeHandle, Vec<f32>)],
        max_evaluation_bytes: Option<u64>,
    ) -> Result<Vec<(NodeHandle, GpuDeformationOutput)>> {
        let weights = self.resolve_weights(instance, weights)?;
        let samples = self
            .primitives
            .iter()
            .map(|source| {
                let sample = (|| -> Result<_> {
                    let handle = instance
                        .node(source.primitive.handle)
                        .context("primitive is absent from the instance")?;
                    ensure!(
                        poses.node(handle).is_some(),
                        "primitive is absent from the pose snapshot"
                    );
                    let skin = source
                        .skin
                        .as_ref()
                        .map(|(skin, _)| -> Result<_> {
                            let pose = skin.pose(instance, poses)?;
                            Ok(skin.binding().palette(pose.mesh_world, &pose.joint_world)?)
                        })
                        .transpose()?;
                    Ok((handle, skin))
                })();
                sample.with_context(|| description(source.primitive))
            })
            .collect::<Result<Vec<_>>>()?;

        let memory = self.memory_for_weights(&weights)?;
        ensure!(
            max_evaluation_bytes.is_none_or(|limit| memory.evaluation_bytes <= limit),
            "GPU scene evaluation requires {} bytes, exceeding the configured budget",
            memory.evaluation_bytes
        );

        self.primitives
            .iter()
            .zip(samples)
            .map(|(source, (handle, pose))| {
                let evaluate = (|| -> Result<_> {
                    let morphed = source
                        .morph
                        .as_ref()
                        .map(|morph| -> Result<_> {
                            let weights = weights[&source.primitive.handle];
                            let changed = weights.iter().any(|weight| *weight != 0.);
                            if !changed && let Some((_, bind)) = &morph.tangents {
                                ensure!(
                                    !bind.context().device_lost(),
                                    "GPU deformation device is lost"
                                );
                                return Ok(bind.clone());
                            }
                            let mut output = morph.gpu.evaluate(weights)?;
                            if changed {
                                if let Some(normals) = &morph.normals {
                                    output = normals.evaluate(&output)?;
                                }
                                if let Some((tangents, _)) = &morph.tangents {
                                    output = tangents.evaluate(&output)?.into_deformation();
                                }
                            }
                            Ok(output)
                        })
                        .transpose()?;
                    let output = if let Some((_, skin)) = &source.skin {
                        let pose = pose.context("missing Skin palette")?;
                        let palette = skin.upload_palette(&pose)?;
                        skin.evaluate(
                            morphed
                                .as_ref()
                                .or(source.base.as_ref())
                                .context("missing bind-space geometry")?,
                            &palette,
                        )?
                    } else {
                        morphed.context("missing Morph output")?
                    };
                    Ok((handle, output))
                })();
                evaluate.with_context(|| description(source.primitive))
            })
            .collect()
    }

    fn resolve_weights<'a>(
        &'a self,
        instance: &SubtreeInstance,
        weights: &'a [(NodeHandle, Vec<f32>)],
    ) -> Result<HashMap<NodeHandle, &'a [f32]>> {
        resolve_weights(
            self.primitives
                .iter()
                .filter_map(|primitive| primitive.morph.as_ref().map(|morph| &morph.source)),
            &|source| instance.node(source),
            weights,
        )
    }
}

fn description(primitive: ScenePrimitive) -> String {
    format!(
        "node {} mesh {} primitive {} GPU deformation",
        primitive.node_index, primitive.mesh_index, primitive.primitive_index
    )
}
