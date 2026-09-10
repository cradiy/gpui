use std::{collections::HashMap, sync::Arc};

use anyhow::{Context, Result, ensure};
use gpui::RenderImage;
use gpui_3d::{AffineTransform, Camera, Node, NodeHandle, SceneGraph, SceneSubtree};

use crate::{
    EncodedImage, GeometryOptions, MaterialDefinition, PreparedDocument, PrimitiveGeometry,
};

/// Aggregate limits for one selected scene. Geometry is charged once per unique
/// mesh primitive; nodes include the synthetic root and primitive children.
#[derive(Clone, Copy, Debug)]
pub struct SceneOptions {
    pub node_limit: usize,
    pub vertex_limit: usize,
    pub index_limit: usize,
}

impl Default for SceneOptions {
    fn default() -> Self {
        Self {
            node_limit: 100_000,
            vertex_limit: 4_194_304,
            index_limit: 12_582_912,
        }
    }
}

#[derive(Clone)]
struct DefinitionNode {
    index: usize,
    name: Option<String>,
    parent: Option<usize>,
    local: AffineTransform,
    camera: Option<(usize, Camera)>,
    primitives: Vec<usize>,
}

#[derive(Clone)]
struct DefinitionPrimitive {
    geometry: PrimitiveGeometry,
    material: usize,
}

/// A selected static scene with converted geometry and encoded material inputs.
/// Clones share the definition. Image decoding and GPU work are not performed.
#[derive(Clone)]
pub struct SceneDefinition(Arc<Definition>);

struct Definition {
    index: usize,
    nodes: Vec<DefinitionNode>,
    primitives: Vec<DefinitionPrimitive>,
    materials: Vec<MaterialDefinition>,
}

/// Original glTF node identity and its source-subtree handle. Names are metadata,
/// not application IDs, and may be duplicated.
#[derive(Clone, Debug)]
pub struct SceneNode {
    pub index: usize,
    pub name: Option<String>,
    pub handle: NodeHandle,
    pub camera_index: Option<usize>,
}

/// One primitive occurrence beneath an original glTF node.
#[derive(Clone, Copy, Debug)]
pub struct ScenePrimitive {
    pub node_index: usize,
    pub mesh_index: usize,
    pub primitive_index: usize,
    pub material_index: Option<usize>,
    pub handle: NodeHandle,
}

/// Immutable scene subtree ready for instantiation through `SceneGraph`.
/// Geometry and decoded images are shared; instances own their node properties.
#[derive(Clone)]
pub struct SceneAsset {
    index: usize,
    subtree: SceneSubtree,
    nodes: Arc<[SceneNode]>,
    primitives: Arc<[ScenePrimitive]>,
}

impl SceneAsset {
    pub fn index(&self) -> usize {
        self.index
    }
    pub fn subtree(&self) -> &SceneSubtree {
        &self.subtree
    }
    /// Source nodes in parent-first order, preserving root and sibling order.
    pub fn nodes(&self) -> &[SceneNode] {
        &self.nodes
    }
    pub fn primitives(&self) -> &[ScenePrimitive] {
        &self.primitives
    }
}

impl SceneDefinition {
    pub fn index(&self) -> usize {
        self.0.index
    }
    /// Unique active materials in first-use order, including an implicit default
    /// when referenced by a primitive without a material index.
    pub fn materials(&self) -> &[MaterialDefinition] {
        &self.0.materials
    }

    /// Decodes each active image index at most once across the entire scene.
    /// The callback has the same BGRA and allocation contract as
    /// `MaterialDefinition::resolve_images`. Failure leaves this definition usable.
    pub fn resolve_images(
        &self,
        mut decode: impl FnMut(usize, &EncodedImage) -> Result<Arc<RenderImage>>,
    ) -> Result<SceneAsset> {
        let mut images = HashMap::new();
        let materials = self
            .0
            .materials
            .iter()
            .map(|definition| {
                definition.resolve_images(|index, encoded| {
                    if let Some(image) = images.get(&index) {
                        return Ok(Arc::clone(image));
                    }
                    let image = decode(index, encoded)?;
                    images.insert(index, image.clone());
                    Ok(image)
                })
            })
            .collect::<Result<Vec<_>>>()
            .with_context(|| format!("scene {} images", self.index()))?;
        let mut graph = SceneGraph::new();
        let root = graph.insert(None, Node::new())?;
        let mut nodes: Vec<SceneNode> = Vec::with_capacity(self.0.nodes.len());
        let mut primitives = Vec::new();
        for source in &self.0.nodes {
            let parent = source.parent.map_or(root, |index| nodes[index].handle);
            let mut node = Node::new().transform(source.local);
            if let Some((_, camera)) = source.camera {
                node = node.camera(camera);
            }
            let handle = graph.insert(Some(parent), node)?;
            nodes.push(SceneNode {
                index: source.index,
                name: source.name.clone(),
                handle,
                camera_index: source.camera.map(|(index, _)| index),
            });
            for &index in &source.primitives {
                let primitive = &self.0.primitives[index];
                let geometry = &primitive.geometry;
                let handle = graph.insert(
                    Some(handle),
                    Node::new().mesh(
                        geometry.mesh().clone(),
                        materials[primitive.material].clone(),
                    ),
                )?;
                primitives.push(ScenePrimitive {
                    node_index: source.index,
                    mesh_index: geometry.mesh_index(),
                    primitive_index: geometry.primitive_index(),
                    material_index: geometry.material_index(),
                    handle,
                });
            }
        }
        Ok(SceneAsset {
            index: self.index(),
            subtree: graph.snapshot_subtree(root)?,
            nodes: nodes.into(),
            primitives: primitives.into(),
        })
    }
}

impl PreparedDocument {
    /// Converts a selected scene's static mesh hierarchy. None selects the declared
    /// default scene and fails when none exists. Only reachable nodes are converted.
    pub fn scene(&self, index: Option<usize>, options: SceneOptions) -> Result<SceneDefinition> {
        self.scene_definition(index, options)
            .with_context(|| format!("scene {index:?}"))
    }

    fn scene_definition(
        &self,
        index: Option<usize>,
        options: SceneOptions,
    ) -> Result<SceneDefinition> {
        crate::validation::supported_extensions(self.gltf())?;
        let scene = match index {
            Some(index) => self
                .gltf()
                .scenes()
                .nth(index)
                .context("scene index out of range")?,
            None => self
                .gltf()
                .default_scene()
                .context("no default scene; select an explicit scene index")?,
        };
        ensure!(options.node_limit > 0, "node limit excludes the scene root");
        let mut definition = Definition {
            index: scene.index(),
            nodes: Vec::new(),
            primitives: Vec::new(),
            materials: Vec::new(),
        };
        let mut pending = Vec::new();
        let mut discovered = vec![false; self.gltf().nodes().len()];
        for node in scene.nodes() {
            ensure!(
                !discovered[node.index()],
                "duplicate scene root node {}",
                node.index()
            );
            discovered[node.index()] = true;
            ensure!(
                pending.len() < options.node_limit - 1,
                "scene roots exceed node limit"
            );
            pending.push((node, None, AffineTransform::IDENTITY));
        }
        pending.reverse();
        let mut primitives = HashMap::new();
        let mut materials = HashMap::new();
        let mut node_count = 1usize;
        let mut vertices_left = options.vertex_limit;
        let mut indices_left = options.index_limit;
        while let Some((source, parent, parent_world)) = pending.pop() {
            let source_index = source.index();
            let convert = (|| -> Result<()> {
                let raw = &self.gltf().as_json().nodes[source_index];
                ensure!(source.skin().is_none(), "skin conversion is unsupported");
                ensure!(
                    raw.weights.is_none(),
                    "morph weight conversion is unsupported"
                );
                ensure!(
                    raw.matrix.is_none()
                        || (raw.translation.is_none()
                            && raw.rotation.is_none()
                            && raw.scale.is_none()),
                    "matrix and TRS properties cannot be combined"
                );
                let local = match source.transform() {
                    gltf::scene::Transform::Matrix { matrix } => {
                        AffineTransform::from_matrix(matrix)?
                    }
                    gltf::scene::Transform::Decomposed {
                        translation,
                        rotation,
                        scale,
                    } => AffineTransform::from_trs(translation, rotation, scale)?,
                };
                let world = parent_world.compose(local)?;
                let camera = source
                    .camera()
                    .map(|source| {
                        let camera = self.camera(source.index())?;
                        camera
                            .transformed(world)
                            .context("camera world transform")?;
                        Ok::<_, anyhow::Error>((source.index(), camera))
                    })
                    .transpose()?;
                ensure!(node_count < options.node_limit, "node limit exceeded");
                node_count += 1;
                let mut node_primitives = Vec::new();
                if let Some(mesh) = source.mesh() {
                    ensure!(
                        mesh.weights().is_none(),
                        "mesh morph weights are unsupported"
                    );
                    for primitive in mesh.primitives() {
                        ensure!(
                            node_count < options.node_limit,
                            "primitive nodes exceed node limit"
                        );
                        node_count += 1;
                        let key = (mesh.index(), primitive.index());
                        let geometry_index = match primitives.get(&key) {
                            Some(&index) => index,
                            None => {
                                let material_index = primitive.material().index();
                                let material = match materials.get(&material_index) {
                                    Some(&index) => index,
                                    None => {
                                        let index = definition.materials.len();
                                        definition.materials.push(self.material(material_index)?);
                                        materials.insert(material_index, index);
                                        index
                                    }
                                };
                                let binding = &definition.materials[material];
                                let geometry = self.geometry(
                                    key.0,
                                    key.1,
                                    GeometryOptions {
                                        tex_coord_set: binding.tex_coord_set().unwrap_or_else(
                                            || {
                                                primitive
                                                    .attributes()
                                                    .filter_map(|(semantic, _)| {
                                                        if let gltf::Semantic::TexCoords(set) =
                                                            semantic
                                                        {
                                                            Some(set)
                                                        } else {
                                                            None
                                                        }
                                                    })
                                                    .min()
                                                    .unwrap_or(0)
                                            },
                                        ),
                                        generate_tangents: binding.requires_tangents()
                                            && (primitive.get(&gltf::Semantic::Tangents).is_none()
                                                || primitive
                                                    .get(&gltf::Semantic::Normals)
                                                    .is_none()),
                                        vertex_limit: vertices_left,
                                        index_limit: indices_left,
                                    },
                                )?;
                                binding.validate_geometry(&geometry)?;
                                vertices_left -= geometry.mesh().vertex_count();
                                indices_left -= geometry.mesh().index_count();
                                let index = definition.primitives.len();
                                definition
                                    .primitives
                                    .push(DefinitionPrimitive { geometry, material });
                                primitives.insert(key, index);
                                index
                            }
                        };
                        definition.primitives[geometry_index]
                            .geometry
                            .mesh()
                            .bounds()
                            .transformed(world)
                            .context("world mesh bounds")?;
                        node_primitives.push(geometry_index);
                    }
                }
                let node_index = definition.nodes.len();
                definition.nodes.push(DefinitionNode {
                    index: source_index,
                    name: source.name().map(str::to_owned),
                    parent,
                    local,
                    camera,
                    primitives: node_primitives,
                });
                let start = pending.len();
                for child in source.children() {
                    ensure!(
                        !discovered[child.index()],
                        "node {} has multiple references or a hierarchy cycle",
                        child.index()
                    );
                    discovered[child.index()] = true;
                    ensure!(
                        pending.len() < options.node_limit - node_count,
                        "pending nodes exceed node limit"
                    );
                    pending.push((child, Some(node_index), world));
                }
                pending[start..].reverse();
                Ok(())
            })();
            convert.with_context(|| format!("node {source_index}"))?;
        }
        Ok(SceneDefinition(Arc::new(definition)))
    }
}
