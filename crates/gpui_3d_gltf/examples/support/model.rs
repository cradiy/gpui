use std::{collections::HashMap, path::Path, time::Duration};

use anyhow::{Context as _, Result};
use gpui::{Pixels, size};
use gpui_3d::{EvaluatedScene, NodeHandle, SceneGraph};
use gpui_3d_gltf::{
    DecodedScene, Document, ImageCache, ImageDecodeLimits, Limits, SceneAsset, SceneInstance,
    SceneLoadCompletion, SceneLoadSlot, SceneLoadStatus, SceneOptions,
};

use super::files;
use gpui_3d_gltf::{
    AnimationClip, AnimationOptions, AnimationPlayback, AnimationTargetPolicy, BoundAnimation,
};

pub(super) struct PixelsReady {
    decoded: DecodedScene,
    materials: HashMap<Option<usize>, String>,
    animation: Option<AnimationClip>,
}

pub(super) fn fitted_size(
    available: gpui::Size<Pixels>,
    aspect: Option<f32>,
) -> gpui::Size<Pixels> {
    let Some(aspect) = aspect else {
        return available;
    };
    let width = available.width.min(available.height * aspect);
    size(width, width / aspect)
}

pub(super) async fn load(
    path: &Path,
    scene: Option<usize>,
    animation: Option<usize>,
    images: &ImageCache,
) -> Result<PixelsReady> {
    let limits = Limits::default();
    let path = path.canonicalize().context("asset path")?;
    let root = path.parent().context("asset has no directory")?;
    let document =
        Document::from_slice(&files::read_bounded(&path, limits.document_bytes)?, limits)?;
    let materials = document.gltf().materials().map(|material| {
        let pbr = material.pbr_metallic_roughness();
        (material.index(), format!("{}\nBase color {:?}\nMetallic {:.2} · Roughness {:.2}\nAlpha {:?} · Double sided {}",
            material.name().unwrap_or("Unnamed material"), pbr.base_color_factor(),
            pbr.metallic_factor(), pbr.roughness_factor(), material.alpha_mode(), material.double_sided()))
    }).collect();
    let prepared = document
        .prepare_async(|request| {
            let result = files::resource_path(root, &request.uri)
                .and_then(|path| files::read_bounded(&path, request.byte_limit));
            std::future::ready(result)
        })
        .await?;
    let decoded = prepared
        .scene(scene, SceneOptions::default())?
        .decode_resources_cached(images, ImageDecodeLimits::default())?;
    let animation = animation
        .map(|index| prepared.animation(index, AnimationOptions::default()))
        .transpose()?;
    Ok(PixelsReady {
        decoded,
        materials,
        animation,
    })
}

pub(super) struct Model {
    pub(super) instance: SceneInstance,
    pub(super) evaluated: EvaluatedScene,
    materials: HashMap<Option<usize>, String>,
    graph: SceneGraph,
    animation: Option<BoundAnimation>,
    pub(super) playback: Option<AnimationPlayback>,
    pub(super) skipped_tracks: usize,
    #[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
    gpu: Option<super::gpu::Deformation>,
    #[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
    desired: Option<(EvaluatedScene, Vec<(NodeHandle, Vec<f32>)>)>,
    #[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
    revision: u64,
    #[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
    gpu_display_revision: Option<u64>,
}

pub(super) fn publish(
    slot: &mut SceneLoadSlot,
    model: &mut Option<Model>,
    completion: SceneLoadCompletion<PixelsReady>,
) -> bool {
    let mut replacement = None;
    let accepted = slot.accept_with(completion, |pixels| {
        let asset = pixels.decoded.resolve()?;
        let mut graph = SceneGraph::new();
        let instance = asset.instantiate(&mut graph, None)?;
        let evaluated = graph.evaluate()?;
        let playback = pixels.animation.as_ref().map(AnimationPlayback::new);
        let animation = pixels
            .animation
            .as_ref()
            .map(|clip| clip.bind(&instance, AnimationTargetPolicy::SkipMissing))
            .transpose()?;
        let mut model = Model {
            instance,
            evaluated,
            materials: pixels.materials,
            graph,
            animation,
            playback,
            skipped_tracks: 0,
            #[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
            gpu: None,
            #[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
            desired: None,
            #[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
            revision: 0,
            #[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
            gpu_display_revision: None,
        };
        if let Some(playback) = &model.playback {
            model.sample(playback.time())?;
        }
        replacement = Some(model);
        Ok(asset)
    });
    if let Some(next) = replacement {
        *model = Some(next);
    }
    accepted && slot.status() == SceneLoadStatus::Ready
}

impl Model {
    pub(super) fn animation_label(&self) -> Option<String> {
        self.animation.as_ref().map(|binding| {
            let clip = binding.clip();
            format!(
                "Clip {} · {}",
                clip.index(),
                clip.name().unwrap_or("Unnamed")
            )
        })
    }

    fn sample(&mut self, time: Duration) -> Result<()> {
        let sample = self
            .animation
            .as_ref()
            .map(|binding| binding.sample(time))
            .transpose()?
            .unwrap_or_default();
        let poses = self
            .graph
            .evaluate_with_transforms(sample.pose().transforms())?;
        if !self.gpu_enabled() {
            let replacements = self.instance.deform(&poses, sample.weights())?;
            self.evaluated = poses.with_meshes(replacements)?;
        }
        #[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
        {
            self.desired = Some((poses, sample.weights().to_vec()));
            self.revision = self.revision.wrapping_add(1);
        }
        self.skipped_tracks = self
            .animation
            .as_ref()
            .map_or(0, |binding| binding.missing_nodes().len());
        Ok(())
    }

    pub(super) fn control(
        &mut self,
        update: impl FnOnce(&mut AnimationPlayback) -> Result<()>,
    ) -> Result<()> {
        let Some(mut playback) = self.playback.clone() else {
            return Ok(());
        };
        let previous = playback.time();
        update(&mut playback)?;
        if playback.time() != previous {
            self.sample(playback.time())?;
        }
        self.playback = Some(playback);
        Ok(())
    }

    pub(super) fn advance(&mut self, elapsed: Duration) -> Result<()> {
        self.control(|playback| {
            playback.advance(elapsed)?;
            Ok(())
        })
    }

    pub(super) fn asset(&self) -> &SceneAsset {
        self.instance.asset()
    }

    pub(super) fn gpu_enabled(&self) -> bool {
        #[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
        {
            self.gpu.is_some()
        }
        #[cfg(not(all(feature = "wgpu", not(target_family = "wasm"))))]
        {
            false
        }
    }

    #[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
    pub(super) fn toggle_gpu(&mut self, window: &gpui::Window) -> Result<()> {
        let next = if self.gpu.is_some() {
            None
        } else {
            Some(super::gpu::Deformation::new(window, &self.instance)?)
        };
        let previous = std::mem::replace(&mut self.gpu, next);
        if let Err(error) = self.sample(
            self.playback
                .as_ref()
                .map_or(Duration::ZERO, AnimationPlayback::time),
        ) {
            self.gpu = previous;
            return Err(error);
        }
        self.gpu_display_revision = None;
        Ok(())
    }

    #[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
    pub(super) fn poll_gpu(&mut self, window: &gpui::Window) -> Result<()> {
        let Some(gpu) = &mut self.gpu else {
            return Ok(());
        };
        let (poses, weights) = self
            .desired
            .as_ref()
            .context("GPU pose has not been sampled")?;
        let result = gpu.update(window, &self.instance, poses, weights, self.revision);
        if let Some((revision, poses)) = gpu.ready_pose()
            && self.gpu_display_revision != Some(revision)
        {
            self.evaluated = poses.clone();
            self.gpu_display_revision = Some(revision);
        }
        result
    }

    pub(super) fn display_bounds(
        &self,
        selected: Option<NodeHandle>,
    ) -> Result<Option<gpui_3d::Aabb>> {
        #[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
        if let Some(gpu) = &self.gpu {
            return gpu.bounds(selected);
        }
        Ok(if let Some(node) = selected {
            self.evaluated
                .node(node)
                .and_then(|node| node.subtree_bounds)
        } else {
            self.evaluated.bounds()
        })
    }

    pub(super) fn viewport(&self, scene: gpui_3d::Scene) -> Result<Option<gpui_3d::Viewport3d>> {
        #[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
        if let Some(gpu) = &self.gpu {
            return gpu.view(scene);
        }
        Ok(Some(gpui_3d::viewport3d("model", scene)))
    }

    pub(super) fn details(&self, selected: Option<NodeHandle>) -> Vec<String> {
        let Some(primitive) = selected.and_then(|node| self.instance.source_primitive(node)) else {
            return vec![
                format!("Scene {}", self.asset().index()),
                format!(
                    "{} nodes · {} primitives",
                    self.asset().nodes().len(),
                    self.asset().primitives().len()
                ),
                format!(
                    "{} skins · {} morph bindings",
                    self.asset().skins().len(),
                    self.asset().morphs().len()
                ),
                "Click a surface to inspect its source node and material.".into(),
            ];
        };
        let node = self
            .asset()
            .nodes()
            .iter()
            .find(|node| node.index == primitive.node_index);
        vec![
            format!(
                "Node {} · {}",
                primitive.node_index,
                node.and_then(|n| n.name.as_deref()).unwrap_or("Unnamed")
            ),
            format!(
                "Mesh {} · Primitive {}",
                primitive.mesh_index, primitive.primitive_index
            ),
            format!("Material {:?}", primitive.material_index),
            self.materials
                .get(&primitive.material_index)
                .cloned()
                .unwrap_or_else(|| "Default material".into()),
        ]
    }
}
