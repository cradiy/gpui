use anyhow::{Context as _, Result};
use gpui::Window;
use gpui_3d::{
    Aabb, AffineTransform, GpuDeformationBounds, GpuDeformationBoundsReadback,
    GpuDeformationLimits, GpuDeformationOutput, GpuMorph, GpuSkin, MorphTargets, NodeHandle,
    ObjectUpdate, Scene, Scene3dDeviceCapabilities, Scene3dGpuGeometry, Skin, Viewport3d,
    WgpuContext, WgpuScene3dGeometry, viewport3d,
};
use std::sync::Arc;

pub(super) struct Deformation {
    context: WgpuContext,
    morph: GpuMorph,
    skin: GpuSkin,
    base: GpuDeformationOutput,
    source: WgpuScene3dGeometry,
    bounds: GpuDeformationBounds,
    output: Output,
    pending: Option<Pending>,
}

struct Output {
    sample: Sample,
    geometry: Arc<Scene3dGpuGeometry>,
    bounds: Aabb,
}

struct Pending {
    sample: Sample,
    geometry: Arc<Scene3dGpuGeometry>,
    bounds: GpuDeformationBoundsReadback,
}

#[derive(Clone, Copy, PartialEq)]
pub(super) struct Sample {
    pub weights: [f32; 2],
    pub bend: Option<f32>,
}

impl Deformation {
    pub(super) fn matches_window(&self, window: &Window) -> bool {
        WgpuContext::for_window(window)
            .is_some_and(|context| Arc::ptr_eq(&context.device, &self.context.device))
            && !self.context.device_lost()
    }

    pub(super) fn new(window: &Window, morphs: MorphTargets, skin: Skin) -> Result<Self> {
        let context = WgpuContext::for_window(window).context("A wgpu window is required")?;
        let capabilities = Scene3dDeviceCapabilities::query(&context);
        GpuMorph::check_support(&capabilities)?;
        GpuSkin::check_support(&capabilities)?;
        GpuDeformationBounds::check_support(&capabilities)?;
        WgpuScene3dGeometry::check_support(&capabilities)?;
        let limits = GpuDeformationLimits::default();
        let base =
            GpuDeformationOutput::upload(context.clone(), morphs.base_mesh().clone(), limits)?;
        let source = base.render_source([0; 5], Some(4 * 1024 * 1024))?;
        let output = Output {
            sample: Sample {
                weights: [0.; 2],
                bend: None,
            },
            geometry: Arc::new(base.render_geometry(&source)?),
            bounds: base.base_mesh().bounds(),
        };
        Ok(Self {
            morph: GpuMorph::new(context.clone(), morphs, limits)?,
            skin: GpuSkin::new(context.clone(), skin, limits)?,
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
            if let Some(bounds) = pending.bounds.try_read()? {
                if self.output.sample != sample {
                    self.output = Output {
                        sample: pending.sample,
                        geometry: pending.geometry,
                        bounds,
                    };
                }
            } else {
                self.pending = Some(pending);
            }
        }
        if self.pending.is_none() && self.output.sample != sample {
            let morphed = if sample.weights == [0.; 2] {
                None
            } else {
                Some(self.morph.evaluate(&sample.weights)?)
            };
            let input = morphed.as_ref().unwrap_or(&self.base);
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
                geometry: Arc::new(output.render_geometry(&self.source)?),
                bounds: self.bounds.request(output, Some(64))?,
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
