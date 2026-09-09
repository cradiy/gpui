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

    fn visit(&self, model: Matrix, ray: Ray, mut visit: impl FnMut(usize)) {
        if self.topology.branches.is_empty() {
            return;
        }
        let mut pending = vec![0];
        while let Some(index) = pending.pop() {
            let branch = &self.topology.branches[index];
            if !self.bounds[index].is_some_and(|bounds| bounds.overlaps(model, ray)) {
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
    pub(crate) fn node(
        handle: NodeHandle,
        local: Aabb,
        world: AffineTransform,
        object: Option<usize>,
    ) -> Self {
        Self {
            key: ObjectKey::Node(handle),
            bounds: Bounds {
                min: local.min().map(f64::from),
                max: local.max().map(f64::from),
            }
            .transformed(world.matrix()),
            object,
        }
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
    objects: Vec<usize>,
    unbounded: Vec<usize>,
}

impl ObjectIndex {
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
            objects: indexed,
            unbounded,
        }
    }

    pub(crate) fn refit(&self, entries: &[IndexObject]) -> Self {
        let by_key: HashMap<_, _> = entries.iter().map(|entry| (entry.key, *entry)).collect();
        if entries.len() != self.entries.len()
            || by_key.len() != entries.len()
            || self.entries.iter().any(|old| {
                by_key
                    .get(&old.key)
                    .is_none_or(|new| old.bounds.is_some() != new.bounds.is_some())
            })
        {
            return Self::build_from(entries);
        }
        let entries: Vec<_> = self.entries.iter().map(|old| by_key[&old.key]).collect();
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
        for &index in &self.unbounded {
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
