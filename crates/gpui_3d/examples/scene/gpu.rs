use anyhow::{Context as _, Result};
use gpui::Window;
use gpui_3d::{
    Aabb, AffineTransform, GpuDeformationBounds, GpuDeformationLimits, GpuDeformationOutput,
    GpuGeometryPreparation, GpuMorph, GpuSkin, GpuTangentGeneration, MorphTargets, NodeHandle,
    ObjectUpdate, Scene, Scene3dDeviceCapabilities, Scene3dGpuGeometry, Skin,
    TangentGenerationMode, Viewport3d, WgpuContext, WgpuScene3dGeometry, viewport3d,
};
use std::sync::Arc;

pub(super) struct Deformation {
    context: WgpuContext,
    morph: GpuMorph,
    skin: GpuSkin,
    tangents: Option<GpuTangentGeneration>,
    base: GpuDeformationOutput,
    source: WgpuScene3dGeometry,
    bounds: GpuDeformationBounds,
    output: Output,
    pending: Option<Pending>,
}

struct Output {
    sample: Option<Sample>,
    geometry: Arc<Scene3dGpuGeometry>,
    bounds: Aabb,
}

struct Pending {
    sample: Sample,
    preparation: GpuGeometryPreparation,
}

#[derive(Clone, Copy, PartialEq)]
pub(super) struct Sample {
    pub weights: [f32; 2],
    pub bend: Option<f32>,
}

impl Deformation {
    pub(super) fn matches(&self, window: &Window, regenerate_tangents: bool) -> bool {
        WgpuContext::for_window(window)
            .is_some_and(|context| Arc::ptr_eq(&context.device, &self.context.device))
            && !self.context.device_lost()
            && self.tangents.is_some() == regenerate_tangents
    }

    pub(super) fn new(
        window: &Window,
        mut morphs: MorphTargets,
        mut skin: Skin,
        regenerate_tangents: bool,
    ) -> Result<Self> {
        let context = WgpuContext::for_window(window).context("A wgpu window is required")?;
        let capabilities = Scene3dDeviceCapabilities::query(&context);
        GpuMorph::check_support(&capabilities)?;
        GpuSkin::check_support(&capabilities)?;
        GpuDeformationBounds::check_support(&capabilities)?;
        WgpuScene3dGeometry::check_support(&capabilities)?;
        let limits = GpuDeformationLimits::default();
        let tangents = if regenerate_tangents {
            GpuTangentGeneration::check_support(&capabilities)?;
            let expanded = morphs.base_mesh().expand_corners(4096)?;
            morphs = morphs.remap_vertices(expanded.mesh().clone(), expanded.source_vertices())?;
            skin = skin.remap_vertices(expanded.source_vertices())?;
            Some(GpuTangentGeneration::new(
                context.clone(),
                morphs.base_mesh().clone(),
                0,
                TangentGenerationMode::Strict,
                limits,
            )?)
        } else {
            None
        };
        let base =
            GpuDeformationOutput::upload(context.clone(), morphs.base_mesh().clone(), limits)?;
        let initial = tangents
            .as_ref()
            .map(|generator| {
                GpuDeformationOutput::upload(
                    context.clone(),
                    generator.output_mesh().clone(),
                    limits,
                )
            })
            .transpose()?;
        let initial = initial.as_ref().unwrap_or(&base);
        let source = initial.render_source([0; 5], Some(4 * 1024 * 1024))?;
        let output = Output {
            sample: tangents.is_none().then_some(Sample {
                weights: [0.; 2],
                bend: None,
            }),
            geometry: Arc::new(initial.render_geometry(&source)?),
            bounds: initial.base_mesh().bounds(),
        };
        Ok(Self {
            morph: GpuMorph::new(context.clone(), morphs, limits)?,
            skin: GpuSkin::new(context.clone(), skin, limits)?,
            tangents,
            bounds: GpuDeformationBounds::new(context.clone())?,
            context,
            base,
            source,
            output,
            pending: None,
        })
    }

    pub(super) fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    pub(super) fn view(
        &mut self,
        scene: Scene,
        bodies: &[NodeHandle],
        sample: Sample,
    ) -> Result<Viewport3d> {
        if let Some(mut pending) = self.pending.take() {
            if let Some(prepared) = pending.preparation.try_read()? {
                if self.output.sample != Some(sample) {
                    self.output = Output {
                        sample: Some(pending.sample),
                        geometry: prepared.geometry().clone(),
                        bounds: prepared.bounds(),
                    };
                }
            } else {
                self.pending = Some(pending);
            }
        }
        if self.pending.is_none() && self.output.sample != Some(sample) {
            let morphed = if sample.weights == [0.; 2] {
                None
            } else {
                Some(self.morph.evaluate(&sample.weights)?)
            };
            let input = morphed.as_ref().unwrap_or(&self.base);
            let generated = self
                .tangents
                .as_ref()
                .map(|generator| {
                    generator
                        .evaluate(input)
                        .map(|result| result.into_deformation())
                })
                .transpose()?;
            let input = generated.as_ref().unwrap_or(input);
            let skinned = if let Some(angle) = sample.bend {
                let tip = AffineTransform::from_trs(
                    [0., -0.25, 0.],
                    [0., 0., angle.sin(), angle.cos()],
                    [1.; 3],
                )?;
                let palette = self
                    .skin
                    .palette(AffineTransform::IDENTITY, &[AffineTransform::IDENTITY, tip])?;
                Some(self.skin.evaluate(input, &palette)?)
            } else {
                None
            };
            let output = skinned.as_ref().unwrap_or(input);
            self.pending = Some(Pending {
                sample,
                preparation: output.prepare_render_geometry(
                    &self.source,
                    &self.bounds,
                    Some(4 * 1024 * 1024),
                )?,
            });
        }
        let updates = scene
            .geometry_inputs()?
            .filter(|object| object.node.is_some_and(|node| bodies.contains(&node)))
            .map(|object| {
                (
                    object.output_id,
                    ObjectUpdate::new()
                        .gpu_geometry(self.output.geometry.clone(), self.output.bounds),
                )
            });
        let submitted = scene.with_object_updates(updates)?;
        Ok(viewport3d("scene", submitted))
    }
}
