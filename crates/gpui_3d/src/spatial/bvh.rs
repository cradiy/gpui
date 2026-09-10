use crate::{Aabb, AffineTransform, Mesh, NodeHandle, Object, Ray, math::Matrix};
use std::{
    collections::{HashMap, HashSet},
    ops::Range,
    sync::Arc,
};

const LEAF_SIZE: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Bounds {
    min: [f64; 3],
    max: [f64; 3],
}

impl Bounds {
    fn intersects(self, region: Aabb) -> bool {
        (0..3).all(|i| {
            self.min[i] <= f64::from(region.max()[i]) && f64::from(region.min()[i]) <= self.max[i]
        })
    }

    fn union(self, other: Self) -> Self {
        Self {
            min: std::array::from_fn(|i| self.min[i].min(other.min[i])),
            max: std::array::from_fn(|i| self.max[i].max(other.max[i])),
        }
    }

    fn transformed(self, model: Matrix) -> Option<Self> {
        if !model.iter().flatten().all(|v| v.is_finite()) {
            return None;
        }
        let mut result = self;
        for axis in 0..3 {
            let translation = f64::from(model[3][axis]);
            let mut low = translation;
            let mut high = translation;
            let mut magnitude = translation.abs();
            for (i, column) in model.iter().enumerate().take(3) {
                let a = f64::from(column[axis]) * self.min[i];
                let b = f64::from(column[axis]) * self.max[i];
                low += a.min(b);
                high += a.max(b);
                magnitude += a.abs().max(b.abs());
            }
            let padding = 32. * f64::from(f32::EPSILON) * magnitude + f64::from(f32::MIN_POSITIVE);
            result.min[axis] = low - padding;
            result.max[axis] = high + padding;
        }
        Some(result)
    }

    fn overlaps(self, model: Matrix, ray: Ray) -> bool {
        let mut near = 0_f64;
        let mut far = f64::INFINITY;
        for axis in 0..3 {
            let translation = f64::from(model[3][axis]);
            let mut low = translation;
            let mut high = translation;
            let mut magnitude = translation.abs();
            for (i, column) in model.iter().enumerate().take(3) {
                let coefficient = f64::from(column[axis]);
                let a = coefficient * self.min[i];
                let b = coefficient * self.max[i];
                low += a.min(b);
                high += a.max(b);
                magnitude += a.abs().max(b.abs());
            }
            // Enclose rounding in the f32 world-space vertex and ray arithmetic.
            let padding =
                32. * f64::from(f32::EPSILON) * magnitude.max(f64::from(ray.origin()[axis]).abs())
                    + f64::from(f32::MIN_POSITIVE);
            low -= padding;
            high += padding;
            if !low.is_finite() || !high.is_finite() {
                return true;
            }
            let origin = f64::from(ray.origin()[axis]);
            let direction = f64::from(ray.direction()[axis]);
            if direction == 0. {
                if origin < low || origin > high {
                    return false;
                }
            } else {
                let a = (low - origin) / direction;
                let b = (high - origin) / direction;
                near = near.max(a.min(b));
                far = far.min(a.max(b));
                if near > far {
                    return false;
                }
            }
        }
        true
    }
}

#[derive(Debug)]
struct Branch {
    items: Range<usize>,
    children: Option<[usize; 2]>,
    parent: Option<usize>,
}

#[derive(Debug)]
struct Topology {
    branches: Vec<Branch>,
    items: Vec<usize>,
    leaves: Vec<usize>,
}

#[derive(Clone, Debug)]
pub(crate) struct Bvh {
    topology: Arc<Topology>,
    bounds: Arc<Vec<Option<Bounds>>>,
}

impl Bvh {
    fn build(mesh: &Mesh) -> Self {
        let bounds: Vec<_> = mesh
            .indices()
            .chunks_exact(3)
            .map(|indices| {
                let positions: [[f32; 3]; 3] =
                    std::array::from_fn(|i| mesh.vertices()[indices[i] as usize].position);
                Bounds {
                    min: std::array::from_fn(|axis| {
                        positions
                            .iter()
                            .map(|p| f64::from(p[axis]))
                            .fold(f64::INFINITY, f64::min)
                    }),
                    max: std::array::from_fn(|axis| {
                        positions
                            .iter()
                            .map(|p| f64::from(p[axis]))
                            .fold(f64::NEG_INFINITY, f64::max)
                    }),
                }
            })
            .collect();
        Self::from_bounds(&bounds)
    }

    fn from_bounds(bounds: &[Bounds]) -> Self {
        let mut topology = Topology {
            branches: Vec::new(),
            items: (0..bounds.len()).collect(),
            leaves: vec![0; bounds.len()],
        };
        let mut branch_bounds = Vec::new();
        if !bounds.is_empty() {
            topology.split(0..bounds.len(), bounds, None, &mut branch_bounds);
        }
        Self {
            topology: Arc::new(topology),
            bounds: Arc::new(branch_bounds),
        }
    }

    fn refit(&self, changed: &[usize], bounds: impl Fn(usize) -> Option<Bounds>) -> Self {
        let mut result = self.clone();
        let mut dirty = HashSet::new();
        for &item in changed {
            let mut branch = Some(self.topology.leaves[item]);
            while let Some(index) = branch {
                if !dirty.insert(index) {
                    break;
                }
                branch = self.topology.branches[index].parent;
            }
        }
        if !dirty.is_empty() {
            let mut dirty: Vec<_> = dirty.into_iter().collect();
            dirty.sort_unstable_by(|a, b| b.cmp(a));
            let updated = Arc::make_mut(&mut result.bounds);
            for index in dirty {
                let branch = &self.topology.branches[index];
                updated[index] = if let Some([left, right]) = branch.children {
                    match (updated[left], updated[right]) {
                        (Some(a), Some(b)) => Some(a.union(b)),
                        (a, b) => a.or(b),
                    }
                } else {
                    self.topology.items[branch.items.clone()]
                        .iter()
                        .filter_map(|&item| bounds(item))
                        .reduce(Bounds::union)
                };
            }
        }
        result
    }

    fn visit(&self, model: Matrix, ray: Ray, visit: impl FnMut(usize)) {
        self.visit_matching(|bounds| bounds.overlaps(model, ray), visit);
    }

    fn visit_matching(&self, overlaps: impl Fn(Bounds) -> bool, mut visit: impl FnMut(usize)) {
        if self.topology.branches.is_empty() {
            return;
        }
        let mut pending = vec![0];
        while let Some(index) = pending.pop() {
            let branch = &self.topology.branches[index];
            if !self.bounds[index].is_some_and(&overlaps) {
                continue;
            }
            if let Some([left, right]) = branch.children {
                pending.push(right);
                pending.push(left);
            } else {
                for &item in &self.topology.items[branch.items.clone()] {
                    visit(item);
                }
            }
        }
    }
}

impl Topology {
    fn split(
        &mut self,
        range: Range<usize>,
        bounds: &[Bounds],
        parent: Option<usize>,
        branch_bounds: &mut Vec<Option<Bounds>>,
    ) -> usize {
        let combined = self.items[range.clone()]
            .iter()
            .map(|&triangle| bounds[triangle])
            .reduce(Bounds::union)
            .unwrap();
        let index = self.branches.len();
        self.branches.push(Branch {
            items: range.clone(),
            children: None,
            parent,
        });
        branch_bounds.push(Some(combined));
        if range.len() > LEAF_SIZE {
            let axis = (0..3)
                .max_by(|&a, &b| {
                    (combined.max[a] - combined.min[a])
                        .total_cmp(&(combined.max[b] - combined.min[b]))
                })
                .unwrap();
            let middle = range.len() / 2;
            self.items[range.clone()].select_nth_unstable_by(middle, |&a, &b| {
                (bounds[a].min[axis] + bounds[a].max[axis])
                    .total_cmp(&(bounds[b].min[axis] + bounds[b].max[axis]))
                    .then_with(|| a.cmp(&b))
            });
            let middle = range.start + middle;
            let left = self.split(range.start..middle, bounds, Some(index), branch_bounds);
            let right = self.split(middle..range.end, bounds, Some(index), branch_bounds);
            self.branches[index].children = Some([left, right]);
        } else {
            for &item in &self.items[range] {
                self.leaves[item] = index;
            }
        }
        index
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum ObjectKey {
    Node(NodeHandle),
    Flat(usize),
}

#[derive(Clone, Copy)]
pub(crate) struct IndexObject {
    key: ObjectKey,
    bounds: Option<Bounds>,
    object: Option<usize>,
}

impl IndexObject {
    pub(crate) fn node_handle(self) -> Option<NodeHandle> {
        match self.key {
            ObjectKey::Node(handle) => Some(handle),
            ObjectKey::Flat(_) => None,
        }
    }

    pub(crate) fn with_bounds(self, local: Aabb, world: AffineTransform) -> Self {
        Self {
            bounds: Bounds {
                min: local.min().map(f64::from),
                max: local.max().map(f64::from),
            }
            .transformed(world.matrix()),
            ..self
        }
    }

    pub(crate) fn node(
        handle: NodeHandle,
        local: Aabb,
        world: AffineTransform,
        object: Option<usize>,
    ) -> Self {
        Self {
            key: ObjectKey::Node(handle),
            bounds: None,
            object,
        }
        .with_bounds(local, world)
    }

    pub(crate) fn flat(objects: &[Object]) -> Vec<Self> {
        let mut local_bounds = HashMap::new();
        objects
            .iter()
            .enumerate()
            .map(|(index, object)| {
                let local = *local_bounds
                    .entry(Arc::as_ptr(&object.mesh.0))
                    .or_insert_with(|| {
                        let bounds = object.mesh.bounds();
                        Bounds {
                            min: bounds.min().map(f64::from),
                            max: bounds.max().map(f64::from),
                        }
                    });
                Self {
                    key: ObjectKey::Flat(index),
                    bounds: local.transformed(object.matrices().0),
                    object: Some(index),
                }
            })
            .collect()
    }

    fn active_bounds(self) -> Option<Bounds> {
        self.object.and(self.bounds)
    }
}

pub(crate) struct ObjectIndex {
    tree: Bvh,
    entries: Vec<IndexObject>,
    objects: Arc<[usize]>,
    unbounded: Arc<[usize]>,
}

impl ObjectIndex {
    pub(crate) fn visit_bounds(&self, region: Aabb, mut visit: impl FnMut(usize)) {
        self.tree.visit_matching(
            |bounds| bounds.intersects(region),
            |index| {
                let entry = self.entries[self.objects[index]];
                if let Some(object) = entry.object
                    && entry.bounds.is_some_and(|bounds| bounds.intersects(region))
                {
                    visit(object);
                }
            },
        );
        for &index in self.unbounded.iter() {
            if let Some(object) = self.entries[index].object {
                visit(object);
            }
        }
    }

    pub(crate) fn build_from(entries: &[IndexObject]) -> Self {
        let mut bounds = Vec::new();
        let mut indexed = Vec::new();
        let mut unbounded = Vec::new();
        for (index, entry) in entries.iter().enumerate() {
            if let Some(world) = entry.bounds {
                bounds.push(world);
                indexed.push(index);
            } else {
                unbounded.push(index);
            }
        }
        let tree = Bvh::from_bounds(&bounds);
        let hidden: Vec<_> = indexed
            .iter()
            .enumerate()
            .filter_map(|(item, &index)| entries[index].object.is_none().then_some(item))
            .collect();
        Self {
            tree: tree.refit(&hidden, |item| entries[indexed[item]].active_bounds()),
            entries: entries.to_vec(),
            objects: indexed.into(),
            unbounded: unbounded.into(),
        }
    }

    pub(crate) fn refit(&self, entries: &[IndexObject]) -> Self {
        if entries.len() != self.entries.len() {
            return Self::build_from(entries);
        }
        let ordered = self
            .entries
            .iter()
            .zip(entries)
            .all(|(old, new)| old.key == new.key);
        let entries: Vec<_> = if ordered {
            if self
                .entries
                .iter()
                .zip(entries)
                .any(|(old, new)| old.bounds.is_some() != new.bounds.is_some())
            {
                return Self::build_from(entries);
            }
            entries.to_vec()
        } else {
            let by_key: HashMap<_, _> = entries.iter().map(|entry| (entry.key, *entry)).collect();
            if by_key.len() != entries.len()
                || self.entries.iter().any(|old| {
                    by_key
                        .get(&old.key)
                        .is_none_or(|new| old.bounds.is_some() != new.bounds.is_some())
                })
            {
                return Self::build_from(entries);
            }
            self.entries.iter().map(|old| by_key[&old.key]).collect()
        };
        let changed: Vec<_> = self
            .objects
            .iter()
            .enumerate()
            .filter_map(|(item, &index)| {
                (self.entries[index].active_bounds() != entries[index].active_bounds())
                    .then_some(item)
            })
            .collect();
        Self {
            tree: self
                .tree
                .refit(&changed, |item| entries[self.objects[item]].active_bounds()),
            entries,
            objects: self.objects.clone(),
            unbounded: self.unbounded.clone(),
        }
    }

    pub(crate) fn visit(&self, ray: Ray, mut visit: impl FnMut(usize)) {
        self.tree
            .visit(AffineTransform::IDENTITY.matrix(), ray, |index| {
                if let Some(object) = self.entries[self.objects[index]].object {
                    visit(object);
                }
            });
        for &index in self.unbounded.iter() {
            if let Some(object) = self.entries[index].object {
                visit(object);
            }
        }
    }
}

impl Mesh {
    /// Builds the shared CPU spatial index if needed. Queries also build it
    /// lazily. This synchronous operation needs no window or GPU and can run on
    /// a worker using a mesh clone before interactive queries begin.
    pub fn prepare_spatial_index(&self) {
        self.1.get_or_init(|| Bvh::build(self));
    }

    pub(crate) fn visit_triangles(&self, model: Matrix, ray: Ray, visit: impl FnMut(usize)) {
        self.1
            .get_or_init(|| Bvh::build(self))
            .visit(model, ray, visit);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Camera, Material, Node, ReparentMode, Scene, SceneGraph};

    #[test]
    fn volume_traversal_prunes_distant_branches_and_keeps_closed_contact() {
        let bounds: Vec<_> = (0..1024)
            .map(|i| Bounds {
                min: [f64::from(i) * 4., -0.5, 0.],
                max: [f64::from(i) * 4. + 1., 0.5, 0.],
            })
            .collect();
        let tree = Bvh::from_bounds(&bounds);
        let region = Aabb::new([401., 0., 0.], [401., 0., 0.]).unwrap();
        let checks = std::cell::Cell::new(0);
        let mut candidates = Vec::new();
        tree.visit_matching(
            |bounds| {
                checks.set(checks.get() + 1);
                bounds.intersects(region)
            },
            |index| candidates.push(index),
        );
        assert!(checks.get() < 32, "visited {} branches", checks.get());
        assert!(candidates.len() <= LEAF_SIZE);
        assert!(candidates.contains(&100));
        assert_eq!(
            candidates
                .into_iter()
                .filter(|&i| bounds[i].intersects(region))
                .collect::<Vec<_>>(),
            vec![100]
        );
    }

    fn plane() -> Object {
        Object::new(Mesh::plane(), Material::color(gpui::rgb(0xffffff)))
    }

    fn ray(x: f32) -> Ray {
        Ray::new([x, 0.1, 5.], [0., 0., -1.]).unwrap()
    }

    #[test]
    fn refitted_graph_preserves_hidden_slots_and_remaps_reordered_objects() {
        let mut graph = SceneGraph::new();
        let parent = graph.insert(None, Node::new()).unwrap();
        let nodes: Vec<_> = (0..32)
            .map(|i| {
                graph
                    .insert(
                        Some(parent),
                        Node::new()
                            .mesh(Mesh::plane(), Material::color(gpui::rgb(0xffffff)))
                            .transform(
                                AffineTransform::from_translation([i as f32 * 2., 0., 0.]).unwrap(),
                            )
                            .visible(i != 0),
                    )
                    .unwrap()
            })
            .collect();
        let initial = graph.evaluate().unwrap();
        initial.prepare_spatial_index();
        let old = initial.scene(Camera::default());
        let old_index = old.spatial_index.get().unwrap();
        assert!(old_index.tree.topology.branches.len() > 1);
        assert!(old.raycast(ray(0.1)).is_none());
        assert_eq!(old.raycast(ray(2.1)).unwrap().object_index, 0);

        graph.set_visible(nodes[0], true).unwrap();
        let shown = graph.evaluate().unwrap();
        shown.prepare_spatial_index_from(&initial);
        let shown_scene = shown.scene(Camera::default());
        let shown_index = shown_scene.spatial_index.get().unwrap();
        assert!(Arc::ptr_eq(
            &old_index.tree.topology,
            &shown_index.tree.topology
        ));
        assert!(!Arc::ptr_eq(
            &old_index.tree.bounds,
            &shown_index.tree.bounds
        ));
        assert_eq!(shown_scene.raycast(ray(0.1)).unwrap().node, Some(nodes[0]));
        assert_eq!(shown_scene.raycast(ray(2.1)).unwrap().object_index, 1);

        graph
            .reparent(nodes[0], None, ReparentMode::KeepWorld)
            .unwrap();
        let reordered = graph.evaluate().unwrap();
        reordered.prepare_spatial_index_from(&shown);
        let reordered_scene = reordered.scene(Camera::default());
        let reordered_index = reordered_scene.spatial_index.get().unwrap();
        assert!(Arc::ptr_eq(
            &shown_index.tree.bounds,
            &reordered_index.tree.bounds
        ));
        let hit = reordered_scene.raycast(ray(0.1)).unwrap();
        assert_eq!(hit.node, Some(nodes[0]));
        assert_eq!(hit.object_index, 31);
        assert_eq!(reordered_scene.raycast(ray(2.1)).unwrap().object_index, 0);

        graph.set_visible(parent, false).unwrap();
        graph.set_visible(nodes[0], false).unwrap();
        let hidden = graph.evaluate().unwrap();
        hidden.prepare_spatial_index_from(&reordered);
        let hidden_scene = hidden.scene(Camera::default());
        let hidden_index = hidden_scene.spatial_index.get().unwrap();
        assert!(Arc::ptr_eq(
            &old_index.tree.topology,
            &hidden_index.tree.topology
        ));
        assert!(hidden_index.tree.bounds[0].is_none());
        hidden_scene.visit_objects(ray(2.1), |_| panic!("hidden object visited"));

        graph.set_visible(parent, true).unwrap();
        graph
            .set_transform(
                parent,
                AffineTransform::from_translation([100., 0., 0.]).unwrap(),
            )
            .unwrap();
        let moved = graph.evaluate().unwrap();
        moved.prepare_spatial_index_from(&hidden);
        let moved_scene = moved.scene(Camera::default());
        assert!(Arc::ptr_eq(
            &old_index.tree.topology,
            &moved_scene.spatial_index.get().unwrap().tree.topology
        ));
        for (i, node) in nodes.iter().enumerate().skip(1) {
            let hit = moved_scene.raycast(ray(100.1 + i as f32 * 2.)).unwrap();
            assert_eq!(hit.node, Some(*node));
            assert_eq!(hit.object_index, i - 1);
        }
        let mut candidates = 0;
        moved_scene.visit_objects(ray(102.1), |_| candidates += 1);
        assert!(candidates <= LEAF_SIZE);
        assert!(moved_scene.raycast(ray(2.1)).is_none());
        assert_eq!(old.raycast(ray(2.1)).unwrap().node, Some(nodes[1]));
        assert!(old.raycast(ray(0.1)).is_none());
    }

    #[test]
    fn refit_rebuilds_for_changed_node_sets_and_updates_replaced_geometry() {
        let mut graph = SceneGraph::new();
        let node = graph
            .insert(
                None,
                Node::new().mesh(Mesh::plane(), Material::color(gpui::rgb(0xffffff))),
            )
            .unwrap();
        let previous = graph.evaluate().unwrap().scene(Camera::default());
        previous.prepare_spatial_index();
        let old_index = previous.spatial_index.get().unwrap();
        let source = Mesh::plane();
        let mut vertices = source.vertices().to_vec();
        for vertex in &mut vertices {
            vertex.position[0] += 10.;
        }
        graph
            .set_mesh(node, Mesh::new(vertices, source.indices().to_vec()))
            .unwrap();
        let deformed = graph.evaluate().unwrap().scene(Camera::default());
        deformed.prepare_spatial_index_from(&previous);
        assert!(Arc::ptr_eq(
            &old_index.tree.topology,
            &deformed.spatial_index.get().unwrap().tree.topology
        ));
        assert!(deformed.raycast(ray(0.1)).is_none());
        assert_eq!(deformed.raycast(ray(10.1)).unwrap().node, Some(node));

        graph.remove_subtree(node).unwrap();
        let replacement = graph
            .insert(
                None,
                Node::new().mesh(Mesh::plane(), Material::color(gpui::rgb(0xffffff))),
            )
            .unwrap();
        let replaced = graph.evaluate().unwrap().scene(Camera::default());
        replaced.prepare_spatial_index_from(&deformed);
        assert!(!Arc::ptr_eq(
            &old_index.tree.topology,
            &replaced.spatial_index.get().unwrap().tree.topology
        ));
        assert_eq!(replaced.raycast(ray(0.1)).unwrap().node, Some(replacement));
        assert_ne!(replacement, node);
        assert_eq!(previous.raycast(ray(0.1)).unwrap().node, Some(node));

        let mut other = SceneGraph::new();
        let other_node = other
            .insert(
                None,
                Node::new().mesh(Mesh::plane(), Material::color(gpui::rgb(0xffffff))),
            )
            .unwrap();
        let foreign = other.evaluate().unwrap().scene(Camera::default());
        foreign.prepare_spatial_index_from(&previous);
        assert!(!Arc::ptr_eq(
            &old_index.tree.topology,
            &foreign.spatial_index.get().unwrap().tree.topology
        ));
        assert_eq!(foreign.raycast(ray(0.1)).unwrap().node, Some(other_node));

        let appended = previous.clone().object(plane().position([0., 0., 1.]));
        appended.prepare_spatial_index_from(&previous);
        assert_eq!(appended.raycast(ray(0.1)).unwrap().object_index, 1);
        assert!(!Arc::ptr_eq(
            &old_index.tree.topology,
            &appended.spatial_index.get().unwrap().tree.topology
        ));
    }

    #[test]
    fn flat_refits_handle_empty_and_unprepared_sources() {
        let previous = Scene::new().object(plane());
        let moved = Scene::new().object(plane().position([10., 0., 0.]));
        moved.prepare_spatial_index_from(&previous);
        assert!(previous.spatial_index.get().is_none());
        assert!(moved.raycast(ray(0.1)).is_none());
        assert_eq!(moved.raycast(ray(10.1)).unwrap().object_index, 0);
        previous.prepare_spatial_index_from(&moved);
        let original_index = previous.spatial_index.get().unwrap();
        assert!(Arc::ptr_eq(
            &original_index.tree.topology,
            &moved.spatial_index.get().unwrap().tree.topology
        ));
        previous.prepare_spatial_index_from(&Scene::new());
        assert!(std::ptr::eq(
            original_index,
            previous.spatial_index.get().unwrap()
        ));
        assert!(previous.raycast(ray(0.1)).is_some());
        assert!(moved.raycast(ray(0.1)).is_none());

        let empty = Scene::new();
        empty.prepare_spatial_index_from(&previous);
        assert!(empty.raycast(ray(0.1)).is_none());
        assert!(previous.raycast(ray(0.1)).is_some());
    }

    #[test]
    fn boundedness_changes_rebuild_conservative_candidate_storage() {
        let mut entries = IndexObject::flat(&[plane()]);
        let finite = ObjectIndex::build_from(&entries);
        entries[0].bounds = None;
        let unbounded = finite.refit(&entries);
        assert!(!Arc::ptr_eq(
            &finite.tree.topology,
            &unbounded.tree.topology
        ));
        let mut candidates = Vec::new();
        unbounded.visit(ray(100.), |i| candidates.push(i));
        assert_eq!(candidates, [0]);
        let distant_region = Aabb::new([100.; 3], [101.; 3]).unwrap();
        let mut candidates = Vec::new();
        unbounded.visit_bounds(distant_region, |i| candidates.push(i));
        assert_eq!(candidates, [0]);
        finite.visit_bounds(distant_region, |_| panic!("distant finite bound visited"));
        finite.visit(ray(100.), |_| panic!("distant finite object visited"));

        entries[0].object = None;
        let hidden = unbounded.refit(&entries);
        hidden.visit(ray(0.1), |_| panic!("hidden unbounded object visited"));
        hidden.visit_bounds(distant_region, |_| {
            panic!("hidden unbounded object visited")
        });
        let restored = hidden.refit(&IndexObject::flat(&[plane()]));
        let mut candidates = Vec::new();
        restored.visit(ray(0.1), |i| candidates.push(i));
        assert_eq!(candidates, [0]);
        restored.visit(ray(100.), |_| panic!("distant restored object visited"));
    }
}
