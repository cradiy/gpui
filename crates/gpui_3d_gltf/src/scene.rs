use std::{collections::HashMap, sync::Arc};

use anyhow::{Context, Result, ensure};
use gpui::RenderImage;
use gpui_3d::{
    AffineTransform, Camera, Node, NodeHandle, PunctualLight, SceneGraph, SceneSubtree, Skin,
};

use crate::{
    EncodedImage, GeometryOptions, MaterialDefinition, PreparedDocument, PrimitiveGeometry,
    SceneMorph, SceneSkin, SkinDefinition, SkinOptions,
};

/// Aggregate limits for one selected scene. Geometry is charged once per unique
/// mesh primitive; nodes include the synthetic root and primitive children.
#[derive(Clone, Copy, Debug)]
pub struct SceneOptions {
    pub node_limit: usize,
    /// Light-bearing nodes in the selected scene, including shared definitions.
    pub light_limit: usize,
    pub vertex_limit: usize,
    /// Retained coordinate pairs across unique primitives; also bounds each
    /// primitive's input and corner-expanded coordinate workspace.
    pub tex_coord_limit: usize,
    pub index_limit: usize,
    /// Total retained geometry influence slots and unique skin-binding slots.
    pub influence_limit: usize,
    /// Total joints in unique skin definitions and unique primitive bindings.
    pub joint_limit: usize,
    /// Initial Morph/Skin output vertices, charged per primitive occurrence.
    pub deformed_vertex_limit: usize,
    pub morph_target_limit: usize,
    pub morph_attribute_limit: usize,
}

impl Default for SceneOptions {
    fn default() -> Self {
        Self {
            node_limit: 100_000,
            light_limit: gpui_3d::MAX_PUNCTUAL_LIGHTS,
            vertex_limit: 4_194_304,
            tex_coord_limit: 16_777_216,
            index_limit: 12_582_912,
            influence_limit: 33_554_432,
            joint_limit: 65_536,
            deformed_vertex_limit: 4_194_304,
            morph_target_limit: 8192,
            morph_attribute_limit: 16_777_216,
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
    light: Option<(usize, PunctualLight)>,
    skin: Option<usize>,
    weights: Arc<[f32]>,
    primitives: Vec<usize>,
}

#[derive(Clone)]
struct DefinitionPrimitive {
    geometry: PrimitiveGeometry,
    material: usize,
}

/// A selected scene with converted geometry, skin bindings, and encoded material inputs.
/// Clones share the definition. Image decoding and GPU work are not performed.
#[derive(Clone)]
pub struct SceneDefinition(Arc<Definition>);

struct Definition {
    source: Arc<()>,
    index: usize,
    nodes: Vec<DefinitionNode>,
    primitives: Vec<DefinitionPrimitive>,
    materials: Vec<MaterialDefinition>,
    skins: HashMap<usize, SkinDefinition>,
    bindings: HashMap<(usize, usize), Skin>,
}

/// Original glTF node identity and its source-subtree handle. Names are metadata,
/// not application IDs, and may be duplicated.
#[derive(Clone, Debug)]
pub struct SceneNode {
    pub index: usize,
    pub name: Option<String>,
    pub handle: NodeHandle,
    pub camera_index: Option<usize>,
    pub light_index: Option<usize>,
    pub skin_index: Option<usize>,
}

/// One primitive occurrence beneath an original glTF node.
#[derive(Clone, Copy, Debug)]
pub struct ScenePrimitive {
    pub node_index: usize,
    pub mesh_index: usize,
    pub primitive_index: usize,
    pub material_index: Option<usize>,
    pub skin_index: Option<usize>,
    pub handle: NodeHandle,
}

/// Immutable scene subtree ready for instantiation through `SceneGraph`.
/// Geometry and decoded images are shared; instances own their node properties.
#[derive(Clone)]
pub struct SceneAsset {
    pub(crate) source: Arc<()>,
    index: usize,
    subtree: SceneSubtree,
    nodes: Arc<[SceneNode]>,
    primitives: Arc<[ScenePrimitive]>,
    skins: Arc<[SceneSkin]>,
    morphs: Arc<[SceneMorph]>,
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
    /// Primitive bindings in scene order, retaining undeformed base geometry.
    pub fn skins(&self) -> &[SceneSkin] {
        &self.skins
    }
    pub fn morphs(&self) -> &[SceneMorph] {
        &self.morphs
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

    /// Unique primitive geometries in first-use order, including source mappings
    /// and base-mesh tangent repair diagnostics.
    pub fn geometries(&self) -> impl ExactSizeIterator<Item = &PrimitiveGeometry> {
        self.0
            .primitives
            .iter()
            .map(|primitive| &primitive.geometry)
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
            if let Some((_, light)) = source.light {
                node = node.light(light);
            }
            let handle = graph.insert(Some(parent), node)?;
            nodes.push(SceneNode {
                index: source.index,
                name: source.name.clone(),
                handle,
                camera_index: source.camera.map(|(index, _)| index),
                light_index: source.light.map(|(index, _)| index),
                skin_index: source.skin,
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
                    skin_index: source.skin,
                    handle,
                });
            }
        }
        let node_handles: HashMap<_, _> =
            nodes.iter().map(|node| (node.index, node.handle)).collect();
        let joint_handles: HashMap<_, Arc<[NodeHandle]>> = self
            .0
            .skins
            .iter()
            .map(|(&index, skin)| {
                (
                    index,
                    skin.joints()
                        .iter()
                        .map(|joint| node_handles[joint])
                        .collect(),
                )
            })
            .collect();
        let mut skins = Vec::new();
        let mut morphs = Vec::new();
        let mut primitive_cursor = 0;
        for source in &self.0.nodes {
            for &geometry_index in &source.primitives {
                let primitive = &primitives[primitive_cursor];
                primitive_cursor += 1;
                if let Some(geometry) = self.0.primitives[geometry_index].geometry.morph() {
                    morphs.push(SceneMorph {
                        node_index: source.index,
                        node: node_handles[&source.index],
                        primitive: primitive.handle,
                        geometry: geometry.clone(),
                        weights: source.weights.clone(),
                    });
                }
                if let Some(index) = source.skin {
                    skins.push(SceneSkin {
                        index,
                        primitive: primitive.handle,
                        joints: joint_handles[&index].clone(),
                        binding: self.0.bindings[&(index, geometry_index)].clone(),
                        base: self.0.primitives[geometry_index].geometry.mesh().clone(),
                    });
                }
            }
        }
        if !skins.is_empty() || !morphs.is_empty() {
            let poses = graph.evaluate()?;
            let meshes = crate::morph::deform(&primitives, &skins, &morphs, Some, &poses, &[])?;
            for (handle, mesh) in meshes {
                graph.set_mesh(handle, mesh)?;
            }
        }
        Ok(SceneAsset {
            source: self.0.source.clone(),
            index: self.index(),
            subtree: graph.snapshot_subtree(root)?,
            nodes: nodes.into(),
            primitives: primitives.into(),
            skins: skins.into(),
            morphs: morphs.into(),
        })
    }
}

impl PreparedDocument {
    /// Converts a selected scene's mesh hierarchy and skin bindings. None selects the declared
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
            source: self.source_identity(),
            index: scene.index(),
            nodes: Vec::new(),
            primitives: Vec::new(),
            materials: Vec::new(),
            skins: HashMap::new(),
            bindings: HashMap::new(),
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
        let mut light_count = 0usize;
        let mut vertices_left = options.vertex_limit;
        let mut tex_coords_left = options.tex_coord_limit;
        let mut indices_left = options.index_limit;
        let mut influences_left = options.influence_limit;
        let mut morph_targets_left = options.morph_target_limit;
        let mut morph_attributes_left = options.morph_attribute_limit;
        let mut mesh_weights = HashMap::<usize, Arc<[f32]>>::new();
        while let Some((source, parent, parent_world)) = pending.pop() {
            let source_index = source.index();
            let convert = (|| -> Result<()> {
                let raw = &self.gltf().as_json().nodes[source_index];
                let skin = source.skin().map(|skin| skin.index());
                ensure!(
                    skin.is_none() || source.mesh().is_some(),
                    "skin node requires a mesh"
                );
                ensure!(
                    raw.weights.is_none() || source.mesh().is_some(),
                    "morph weight node requires a mesh"
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
                let light = source
                    .light()
                    .map(|source| {
                        ensure!(light_count < options.light_limit, "light limit exceeded");
                        let light = self.light(source.index())?;
                        light.transformed(world).context("light world transform")?;
                        light_count += 1;
                        Ok::<_, anyhow::Error>((source.index(), light))
                    })
                    .transpose()?;
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
                let mut weights = Arc::from([]);
                if let Some(mesh) = source.mesh() {
                    weights = if let Some(weights) = source.weights() {
                        crate::morph::default_weights(
                            &mesh,
                            Some(weights),
                            options.morph_target_limit,
                        )?
                    } else if let Some(weights) = mesh_weights.get(&mesh.index()) {
                        weights.clone()
                    } else {
                        let weights =
                            crate::morph::default_weights(&mesh, None, options.morph_target_limit)?;
                        mesh_weights.insert(mesh.index(), weights.clone());
                        weights
                    };
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
                                        tangent_uv_set: binding.normal_tex_coord_set().unwrap_or(0),
                                        tex_coord_limit: tex_coords_left,
                                        generate_tangents: binding.requires_tangents()
                                            && (primitive.get(&gltf::Semantic::Tangents).is_none()
                                                || primitive
                                                    .get(&gltf::Semantic::Normals)
                                                    .is_none()),
                                        vertex_limit: vertices_left,
                                        index_limit: indices_left,
                                        influence_limit: influences_left,
                                        morph_target_limit: morph_targets_left,
                                        morph_attribute_limit: morph_attributes_left,
                                    },
                                )?;
                                binding.validate_geometry(&geometry)?;
                                vertices_left -= geometry.mesh().vertex_count();
                                tex_coords_left -= geometry.tex_coord_count();
                                indices_left -= geometry.mesh().index_count();
                                influences_left -= geometry.influence_count();
                                if let Some(morph) = geometry.morph() {
                                    morph_targets_left -= morph.targets().len();
                                    morph_attributes_left -= morph.attribute_vertex_count();
                                }
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
                    light,
                    skin,
                    weights,
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
        let lookup: HashMap<_, _> = definition
            .nodes
            .iter()
            .enumerate()
            .map(|(i, node)| (node.index, i))
            .collect();
        let mut roots = Vec::with_capacity(definition.nodes.len());
        let mut ends: Vec<_> = (1..=definition.nodes.len()).collect();
        for (i, node) in definition.nodes.iter().enumerate() {
            roots.push(node.parent.map_or(i, |parent| roots[parent]));
        }
        for (i, node) in definition.nodes.iter().enumerate().rev() {
            if let Some(parent) = node.parent {
                ends[parent] = ends[parent].max(ends[i]);
            }
        }
        let mut joints_left = options.joint_limit;
        let mut deformed_vertices_left = options.deformed_vertex_limit;
        for node in &definition.nodes {
            for &geometry_index in &node.primitives {
                let geometry = &definition.primitives[geometry_index].geometry;
                if node.skin.is_some() || geometry.morph().is_some() {
                    deformed_vertices_left = deformed_vertices_left
                        .checked_sub(geometry.mesh().vertex_count())
                        .context("initial deformed vertices exceed deformed vertex limit")?;
                }
            }
            let Some(index) = node.skin else {
                continue;
            };
            let bind = (|| -> Result<()> {
                if let std::collections::hash_map::Entry::Vacant(entry) =
                    definition.skins.entry(index)
                {
                    let skin = self.skin(
                        index,
                        SkinOptions {
                            joint_limit: joints_left,
                        },
                    )?;
                    let joints = skin
                        .joints()
                        .iter()
                        .map(|joint| {
                            lookup.get(joint).copied().with_context(|| {
                                format!("joint node {joint} is outside the selected scene")
                            })
                        })
                        .collect::<Result<Vec<_>>>()?;
                    let root = roots[joints[0]];
                    ensure!(
                        joints.iter().all(|&joint| roots[joint] == root),
                        "skin joints have no common scene root"
                    );
                    if let Some(skeleton) = skin.skeleton() {
                        let skeleton = *lookup
                            .get(&skeleton)
                            .context("skeleton is outside the selected scene")?;
                        ensure!(
                            joints
                                .iter()
                                .all(|&joint| skeleton <= joint && joint < ends[skeleton]),
                            "skeleton must be an ancestor of every joint"
                        );
                    }
                    joints_left -= skin.joints().len();
                    entry.insert(skin);
                }
                let skin = &definition.skins[&index];
                for &geometry_index in &node.primitives {
                    if let std::collections::hash_map::Entry::Vacant(entry) =
                        definition.bindings.entry((index, geometry_index))
                    {
                        let geometry = &definition.primitives[geometry_index].geometry;
                        joints_left = joints_left
                            .checked_sub(skin.joints().len())
                            .context("skin bindings exceed joint limit")?;
                        influences_left = influences_left
                            .checked_sub(geometry.influence_count())
                            .context("skin bindings exceed influence limit")?;
                        entry.insert(skin.bind(geometry)?);
                    }
                }
                Ok(())
            })();
            bind.with_context(|| format!("node {} skin {index}", node.index))?;
        }
        Ok(SceneDefinition(Arc::new(definition)))
    }
}
