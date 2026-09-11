use std::{collections::HashMap, sync::Arc};

use anyhow::{Context as _, Result, ensure};
use gpui::Window;
use gpui_3d::{
    Aabb, EvaluatedScene, GpuDeformationBounds, GpuDeformationBoundsReadback, GpuDeformationLimits,
    GpuDeformationOutput, NodeHandle, Scene, Scene3dGpuDraw, Scene3dGpuGeometry, Viewport3d,
    WgpuContext, WgpuScene3dGeometry, viewport3d,
};
use gpui_3d_gltf::{GpuSceneDeformation, SceneInstance};

pub(super) struct Deformation {
    context: WgpuContext,
    source: GpuSceneDeformation,
    bounds: GpuDeformationBounds,
    packing: HashMap<(NodeHandle, [u32; 5]), WgpuScene3dGeometry>,
    pending: Option<Pending>,
    ready: Option<Ready>,
    failed: Option<(u64, String)>,
    device_valid: bool,
}

struct Entry {
    node: NodeHandle,
    output: GpuDeformationOutput,
    bounds: Aabb,
    packed: Option<([u32; 5], Arc<Scene3dGpuGeometry>)>,
}

struct Ready {
    revision: u64,
    poses: EvaluatedScene,
    entries: Vec<Entry>,
    indices: HashMap<NodeHandle, usize>,
}

struct Pending {
    revision: u64,
    poses: EvaluatedScene,
    entries: Vec<PendingEntry>,
}

struct PendingEntry {
    node: NodeHandle,
    output: GpuDeformationOutput,
    readback: Option<GpuDeformationBoundsReadback>,
    bounds: Option<Aabb>,
}

impl Deformation {
    pub fn ready_pose(&self) -> Option<(u64, &EvaluatedScene)> {
        self.ready
            .as_ref()
            .map(|ready| (ready.revision, &ready.poses))
    }
    pub fn new(window: &Window, instance: &SceneInstance) -> Result<Self> {
        GpuSceneDeformation::check_asset(instance.asset())?;
        let context = WgpuContext::for_window(window).context("A WGPU window is required")?;
        Ok(Self {
            source: GpuSceneDeformation::new(
                context.clone(),
                instance.asset(),
                GpuDeformationLimits::default(),
            )?,
            bounds: GpuDeformationBounds::new(context.clone())?,
            context,
            packing: HashMap::new(),
            pending: None,
            ready: None,
            failed: None,
            device_valid: true,
        })
    }

    pub fn update(
        &mut self,
        window: &Window,
        instance: &SceneInstance,
        poses: &EvaluatedScene,
        weights: &[(NodeHandle, Vec<f32>)],
        revision: u64,
    ) -> Result<()> {
        self.device_valid = WgpuContext::for_window(window)
            .is_some_and(|cx| Arc::ptr_eq(&cx.device, &self.context.device))
            && !self.context.device_lost();
        ensure!(
            self.device_valid,
            "GPU device changed; disable and re-enable GPU deformation"
        );
        if let Some((failed, error)) = &self.failed
            && *failed == revision
        {
            anyhow::bail!("{error}");
        }
        let result = self.advance(instance, poses, weights, revision);
        if let Err(error) = &result {
            self.failed = Some((revision, format!("{error:#}")));
        }
        if self.pending.is_some() {
            window.request_animation_frame();
        }
        result
    }

    fn advance(
        &mut self,
        instance: &SceneInstance,
        poses: &EvaluatedScene,
        weights: &[(NodeHandle, Vec<f32>)],
        revision: u64,
    ) -> Result<()> {
        if let Some(mut pending) = self.pending.take() {
            for entry in &mut pending.entries {
                if let Some(request) = &mut entry.readback
                    && let Some(value) = request.try_read()?
                {
                    entry.bounds = Some(value);
                    entry.readback = None;
                }
            }
            if pending.entries.iter().all(|entry| entry.bounds.is_some()) {
                let ready = Ready {
                    revision: pending.revision,
                    poses: pending.poses,
                    indices: pending
                        .entries
                        .iter()
                        .enumerate()
                        .map(|(index, entry)| (entry.node, index))
                        .collect(),
                    entries: pending
                        .entries
                        .into_iter()
                        .map(|entry| Entry {
                            node: entry.node,
                            output: entry.output,
                            bounds: entry.bounds.unwrap(),
                            packed: None,
                        })
                        .collect(),
                };
                self.ready = Some(ready);
            } else {
                self.pending = Some(pending);
            }
        }
        if self.pending.is_none()
            && self
                .ready
                .as_ref()
                .is_none_or(|ready| ready.revision != revision)
        {
            let outputs = self
                .source
                .evaluate(instance.subtree_instance(), poses, weights)?;
            let poses = poses.with_meshes(
                outputs
                    .iter()
                    .map(|(node, output)| (*node, output.base_mesh().clone())),
            )?;
            let entries = outputs
                .into_iter()
                .map(|(node, output)| {
                    let bounds = self.bounds.request(&output, Some(64))?;
                    Ok(PendingEntry {
                        node,
                        output,
                        readback: Some(bounds),
                        bounds: None,
                    })
                })
                .collect::<Result<_>>()?;
            self.pending = Some(Pending {
                revision,
                poses,
                entries,
            });
        }
        Ok(())
    }

    pub fn bounds(&self, selected: Option<NodeHandle>) -> Result<Option<Aabb>> {
        let Some(ready) = &self.ready else {
            return Ok(None);
        };
        let mut combined = None;
        for node in
            ready.poses.nodes().iter().filter(|node| {
                node.visible && selected.is_none_or(|selected| node.handle == selected)
            })
        {
            let bounds = if let Some(&index) = ready.indices.get(&node.handle) {
                let entry = &ready.entries[index];
                Some(entry.bounds.transformed(node.world)?)
            } else {
                node.bounds
            };
            if let Some(bounds) = bounds {
                combined = Some(combined.map_or(bounds, |old: Aabb| old.union(bounds)));
            }
        }
        Ok(combined)
    }

    pub fn view(&mut self, scene: Scene) -> Result<Option<Viewport3d>> {
        ensure!(
            self.device_valid,
            "GPU device changed; disable and re-enable GPU deformation"
        );
        let Some(ready) = &mut self.ready else {
            return Ok(None);
        };
        let mut draws = Vec::new();
        for object in scene.geometry_inputs()? {
            let node = object.node;
            let Some(&index) = node.and_then(|node| ready.indices.get(&node)) else {
                continue;
            };
            let entry = &mut ready.entries[index];
            let uv_sets = object.uv_sets;
            if entry.packed.as_ref().is_none_or(|(uv, _)| *uv != uv_sets) {
                let key = (entry.node, uv_sets);
                if let std::collections::hash_map::Entry::Vacant(slot) = self.packing.entry(key) {
                    slot.insert(
                        entry
                            .output
                            .render_source(uv_sets, Some(256 * 1024 * 1024))?,
                    );
                }
                entry.packed = Some((
                    uv_sets,
                    Arc::new(entry.output.render_geometry(&self.packing[&key])?),
                ));
            }
            draws.push(Scene3dGpuDraw {
                output_id: object.output_id,
                geometry: entry.packed.as_ref().unwrap().1.clone(),
                bounds: [entry.bounds.min(), entry.bounds.max()],
            });
        }
        Ok(Some(viewport3d("model", scene).gpu_geometry(&draws)?))
    }
}
