use std::{collections::HashMap, sync::Arc};

use anyhow::{Context as _, Result, ensure};
use gpui::Window;
use gpui_3d::{
    Aabb, Camera, EvaluatedScene, GpuDeformationBounds, GpuDeformationLimits,
    GpuGeometryPreparation, Mesh, NodeHandle, PreparedGpuGeometry, Scene, Scene3dGpuDraw,
    Viewport3d, WgpuContext, WgpuScene3dGeometry, viewport3d,
};
use gpui_3d_gltf::{GpuSceneDeformation, SceneInstance};

pub(super) struct Deformation {
    context: WgpuContext,
    source: GpuSceneDeformation,
    bounds: GpuDeformationBounds,
    packing: HashMap<(NodeHandle, [u32; 5]), (Mesh, WgpuScene3dGeometry)>,
    pending: Option<Pending>,
    ready: Option<Ready>,
    failed: Option<(u64, String)>,
    device_valid: bool,
}

struct Entry {
    uv_sets: [u32; 5],
    prepared: PreparedGpuGeometry,
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
    uv_sets: [u32; 5],
    request: Option<GpuGeometryPreparation>,
    prepared: Option<PreparedGpuGeometry>,
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
        if let Some(pending_revision) = self.pending.as_ref().map(|pending| pending.revision)
            && let Err(error) = self.poll_pending()
        {
            self.failed = Some((pending_revision, format!("{error:#}")));
            if pending_revision != revision {
                window.request_animation_frame();
            }
            return Err(error);
        }
        if let Some((failed, error)) = &self.failed
            && *failed == revision
        {
            if self.pending.is_some() {
                window.request_animation_frame();
            }
            anyhow::bail!("{error}");
        }
        let result = self.submit(instance, poses, weights, revision);
        if let Err(error) = &result {
            self.failed = Some((revision, format!("{error:#}")));
        }
        if self.pending.is_some() {
            window.request_animation_frame();
        }
        result
    }

    fn poll_pending(&mut self) -> Result<()> {
        if let Some(mut pending) = self.pending.take() {
            for entry in &mut pending.entries {
                if let Some(request) = &mut entry.request
                    && let Some(value) = request.try_read()?
                {
                    entry.prepared = Some(value);
                    entry.request = None;
                }
            }
            if pending.entries.iter().all(|entry| entry.prepared.is_some()) {
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
                            uv_sets: entry.uv_sets,
                            prepared: entry.prepared.unwrap(),
                        })
                        .collect(),
                };
                self.ready = Some(ready);
            } else {
                self.pending = Some(pending);
            }
        }
        Ok(())
    }

    fn submit(
        &mut self,
        instance: &SceneInstance,
        poses: &EvaluatedScene,
        weights: &[(NodeHandle, Vec<f32>)],
        revision: u64,
    ) -> Result<()> {
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
            let scene = poses.scene(Camera::default());
            let coordinates: HashMap<_, _> = scene
                .geometry_inputs()?
                .filter_map(|object| object.node.map(|node| (node, object.uv_sets)))
                .collect();
            let entries = outputs
                .into_iter()
                .map(|(node, output)| {
                    let uv_sets = *coordinates
                        .get(&node)
                        .context("GPU primitive is absent from its evaluated scene")?;
                    let key = (node, uv_sets);
                    if self
                        .packing
                        .get(&key)
                        .is_none_or(|(mesh, _)| !mesh.ptr_eq(output.base_mesh()))
                    {
                        let source = match self.packing.get(&key) {
                            Some((_, source)) => output.rebind_render_source(
                                source,
                                uv_sets,
                                Some(256 * 1024 * 1024),
                            )?,
                            None => output.render_source(uv_sets, Some(256 * 1024 * 1024))?,
                        };
                        self.packing
                            .insert(key, (output.base_mesh().clone(), source));
                    }
                    let source = &self.packing[&key].1;
                    let request = output.prepare_render_geometry(
                        source,
                        &self.bounds,
                        Some(256 * 1024 * 1024),
                    )?;
                    Ok(PendingEntry {
                        node,
                        uv_sets,
                        request: Some(request),
                        prepared: None,
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
                Some(entry.prepared.bounds().transformed(node.world)?)
            } else {
                node.bounds
            };
            if let Some(bounds) = bounds {
                combined = Some(combined.map_or(bounds, |old: Aabb| old.union(bounds)));
            }
        }
        Ok(combined)
    }

    pub fn view(&self, scene: Scene) -> Result<Option<Viewport3d>> {
        ensure!(
            self.device_valid,
            "GPU device changed; disable and re-enable GPU deformation"
        );
        let Some(ready) = &self.ready else {
            return Ok(None);
        };
        let mut draws = Vec::new();
        for object in scene.geometry_inputs()? {
            let node = object.node;
            let Some(&index) = node.and_then(|node| ready.indices.get(&node)) else {
                continue;
            };
            let entry = &ready.entries[index];
            ensure!(
                entry.uv_sets == object.uv_sets,
                "Material coordinates differ from the prepared GPU frame"
            );
            let bounds = entry.prepared.bounds();
            draws.push(Scene3dGpuDraw {
                output_id: object.output_id,
                geometry: entry.prepared.geometry().clone(),
                bounds: [bounds.min(), bounds.max()],
            });
        }
        Ok(Some(viewport3d("model", scene).gpu_geometry(&draws)?))
    }
}
