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
        let replacements = self.instance.deform(&poses, sample.weights())?;
        let evaluated = poses.with_meshes(replacements)?;
        self.evaluated = evaluated;
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
