use std::collections::HashMap;

use anyhow::{Context, Result, ensure};
use gpui_3d::{
    EvaluatedScene, GpuDeformationLimits, GpuDeformationOutput, GpuFlatNormals, GpuMorph, GpuSkin,
    NodeHandle, SubtreeInstance, WgpuContext,
};

use crate::{SceneAsset, SceneMorph, ScenePrimitive, SceneSkin, morph::resolve_weights};

/// Retained imported Morph and Skin sources on one device, reusable across scene instances.
/// Evaluation submits immutable outputs without reading vertices or changing the CPU scene.
pub struct GpuSceneDeformation {
    primitives: Vec<PrimitiveSource>,
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
}

impl GpuSceneDeformation {
    /// Checks imported direction policies without a device or GPU allocation.
    /// MikkTSpace tangent regeneration is unsupported, including zero-weight assets.
    /// Device limits, payload budgets, and arithmetic validity are checked separately.
    pub fn check_asset(asset: &SceneAsset) -> Result<()> {
        for morph in asset.morphs() {
            ensure!(
                !morph.geometry().regenerates_tangents(),
                "node {} primitive {:?}: GPU MikkTSpace tangent regeneration is unsupported",
                morph.node_index(),
                morph.primitive(),
            );
        }
        Ok(())
    }

    /// Uploads each deformable primitive after checking all imported direction policies.
    /// Limits apply per core source and per output, not to aggregate scene residency.
    /// The adapter retains geometry and bindings, but not materials or decoded images.
    pub fn new(
        context: WgpuContext,
        asset: &SceneAsset,
        limits: GpuDeformationLimits,
    ) -> Result<Self> {
        Self::check_asset(asset)?;
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
                        Ok::<_, anyhow::Error>(MorphSource {
                            source: (*source).clone(),
                            gpu: GpuMorph::new(
                                context.clone(),
                                geometry.attribute_targets().clone(),
                                limits,
                            )?,
                            normals,
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
        Ok(Self { primitives })
    }

    /// Submits Morph, optional flat normals, then Skin in primitive order.
    /// Overrides address mapped original nodes; omitted values use authored defaults.
    /// Every call starts from bind-space inputs, independently of earlier samples.
    /// Returned handles address mapped primitive children. Their output `base_mesh()`
    /// is the source identity required by GPU render packing and scene mesh overrides.
    /// CPU bounds and queries are not updated. No CPU fallback is performed.
    pub fn evaluate(
        &self,
        instance: &SubtreeInstance,
        poses: &EvaluatedScene,
        weights: &[(NodeHandle, Vec<f32>)],
    ) -> Result<Vec<(NodeHandle, GpuDeformationOutput)>> {
        let weights = resolve_weights(
            self.primitives
                .iter()
                .filter_map(|primitive| primitive.morph.as_ref().map(|morph| &morph.source)),
            &|source| instance.node(source),
            weights,
        )?;
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
                        .map(|(skin, _)| skin.pose(instance, poses))
                        .transpose()?;
                    Ok((handle, skin))
                })();
                sample.with_context(|| description(source.primitive))
            })
            .collect::<Result<Vec<_>>>()?;

        self.primitives
            .iter()
            .zip(samples)
            .map(|(source, (handle, pose))| {
                let evaluate = (|| -> Result<_> {
                    let morphed = source
                        .morph
                        .as_ref()
                        .map(|morph| {
                            let weights = weights[&source.primitive.handle];
                            let output = morph.gpu.evaluate(weights)?;
                            if let Some(normals) = &morph.normals
                                && weights.iter().any(|weight| *weight != 0.)
                            {
                                normals.evaluate(&output)
                            } else {
                                Ok(output)
                            }
                        })
                        .transpose()?;
                    let output = if let Some((_, skin)) = &source.skin {
                        let pose = pose.context("missing Skin pose")?;
                        let palette = skin.palette(pose.mesh_world, &pose.joint_world)?;
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
}

fn description(primitive: ScenePrimitive) -> String {
    format!(
        "node {} mesh {} primitive {} GPU deformation",
        primitive.node_index, primitive.mesh_index, primitive.primitive_index
    )
}
