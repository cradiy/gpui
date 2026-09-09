use crate::{AffineTransform, Mesh, Object, Ray, math::Matrix};
use std::{collections::HashMap, ops::Range, sync::Arc};

const LEAF_SIZE: usize = 8;

#[derive(Clone, Copy, Debug)]
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
    bounds: Bounds,
    items: Range<usize>,
    children: Option<[usize; 2]>,
}

#[derive(Debug)]
pub(crate) struct Bvh {
    branches: Vec<Branch>,
    items: Vec<usize>,
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
        let mut tree = Self {
            branches: Vec::new(),
            items: (0..bounds.len()).collect(),
        };
        if !bounds.is_empty() {
            tree.split(0..bounds.len(), bounds);
        }
        tree
    }

    fn split(&mut self, range: Range<usize>, bounds: &[Bounds]) -> usize {
        let combined = self.items[range.clone()]
            .iter()
            .map(|&triangle| bounds[triangle])
            .reduce(Bounds::union)
            .unwrap();
        let index = self.branches.len();
        self.branches.push(Branch {
            bounds: combined,
            items: range.clone(),
            children: None,
        });
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
            let left = self.split(range.start..middle, bounds);
            let right = self.split(middle..range.end, bounds);
            self.branches[index].children = Some([left, right]);
        }
        index
    }

    fn visit(&self, model: Matrix, ray: Ray, mut visit: impl FnMut(usize)) {
        if self.branches.is_empty() {
            return;
        }
        let mut pending = vec![0];
        while let Some(index) = pending.pop() {
            let branch = &self.branches[index];
            if !branch.bounds.overlaps(model, ray) {
                continue;
            }
            if let Some([left, right]) = branch.children {
                pending.push(right);
                pending.push(left);
            } else {
                for &item in &self.items[branch.items.clone()] {
                    visit(item);
                }
            }
        }
    }
}

pub(crate) struct ObjectIndex {
    tree: Bvh,
    objects: Vec<usize>,
    unbounded: Vec<usize>,
}

impl ObjectIndex {
    pub(crate) fn build(objects: &[Object]) -> Self {
        let mut local_bounds = HashMap::new();
        let mut bounds = Vec::new();
        let mut indexed = Vec::new();
        let mut unbounded = Vec::new();
        for (index, object) in objects.iter().enumerate() {
            let local = *local_bounds
                .entry(Arc::as_ptr(&object.mesh.0))
                .or_insert_with(|| {
                    let bounds = object.mesh.bounds();
                    Bounds {
                        min: bounds.min().map(f64::from),
                        max: bounds.max().map(f64::from),
                    }
                });
            if let Some(world) = local.transformed(object.matrices().0) {
                bounds.push(world);
                indexed.push(index);
            } else {
                unbounded.push(index);
            }
        }
        Self {
            tree: Bvh::from_bounds(&bounds),
            objects: indexed,
            unbounded,
        }
    }

    pub(crate) fn visit(&self, ray: Ray, mut visit: impl FnMut(usize)) {
        self.tree
            .visit(AffineTransform::IDENTITY.matrix(), ray, |index| {
                visit(self.objects[index])
            });
        for &index in &self.unbounded {
            visit(index);
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
