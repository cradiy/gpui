use collections::HashMap;
use gpui::Mesh3d;
use std::sync::Arc;

struct Entry<T> {
    mesh: Arc<Mesh3d>,
    resource: T,
}

pub(super) struct GeometryCache<T> {
    entries: HashMap<usize, Arc<Entry<T>>>,
}

impl<T> Default for GeometryCache<T> {
    fn default() -> Self {
        Self {
            entries: HashMap::default(),
        }
    }
}

fn key(mesh: &Arc<Mesh3d>) -> usize {
    Arc::as_ptr(mesh) as usize
}

fn topology(mesh: &Mesh3d) -> (usize, usize) {
    (mesh.indices().as_ptr() as usize, mesh.vertices().len())
}

impl<T> GeometryCache<T> {
    pub(super) fn prepare(
        &mut self,
        meshes: impl IntoIterator<Item = Arc<Mesh3d>>,
        mut upload: impl FnMut(Option<T>, &Mesh3d) -> T,
    ) {
        let required: HashMap<_, _> = meshes.into_iter().map(|mesh| (key(&mesh), mesh)).collect();
        let mut retired: HashMap<_, Vec<T>> = HashMap::default();
        for (key, entry) in std::mem::take(&mut self.entries) {
            if required.contains_key(&key) {
                self.entries.insert(key, entry);
            } else if let Ok(entry) = Arc::try_unwrap(entry) {
                retired
                    .entry(topology(&entry.mesh))
                    .or_default()
                    .push(entry.resource);
            }
        }
        for (key, mesh) in required {
            self.entries.entry(key).or_insert_with(|| {
                let resource = retired.get_mut(&topology(&mesh)).and_then(Vec::pop);
                Arc::new(Entry {
                    resource: upload(resource, &mesh),
                    mesh,
                })
            });
        }
    }

    pub(super) fn get(&self, mesh: &Arc<Mesh3d>) -> &T {
        &self.entries[&key(mesh)].resource
    }

    pub(super) fn reuse_from(&mut self, other: &Self) {
        self.entries.clone_from(&other.entries);
    }

    pub(super) fn retain(&mut self, meshes: impl IntoIterator<Item = Arc<Mesh3d>>) {
        let required: collections::HashSet<_> = meshes.into_iter().map(|mesh| key(&mesh)).collect();
        self.entries.retain(|key, _| required.contains(key));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct Buffer {
        id: usize,
        x: f32,
    }

    fn mesh(x: f32) -> Arc<Mesh3d> {
        Mesh3d::new(
            vec![
                gpui::MeshVertex3d {
                    position: [x, 0., 0.],
                    normal: [0., 0., 1.],
                    uv: [0.; 2],
                };
                3
            ],
            vec![0, 1, 2],
        )
    }

    fn update(mesh: &Mesh3d, x: f32) -> Arc<Mesh3d> {
        let mut vertices = mesh.vertices().to_vec();
        vertices[0].position[0] = x;
        mesh.with_vertices(vertices, None).unwrap()
    }

    fn upload(allocations: &mut usize, old: Option<Buffer>, mesh: &Mesh3d) -> Buffer {
        let id = old.map_or_else(
            || {
                *allocations += 1;
                *allocations
            },
            |buffer| buffer.id,
        );
        Buffer {
            id,
            x: mesh.vertices()[0].position[0],
        }
    }

    #[test]
    fn scene3d_geometry_updates_reuse_slots_without_overwriting_live_snapshots() {
        let original = mesh(0.);
        let a = update(&original, 1.);
        let b = update(&original, 2.);
        let c = update(&original, 3.);
        let mut cache = GeometryCache::default();
        let mut allocations = 0;
        cache.prepare([original.clone()], |old, mesh| {
            upload(&mut allocations, old, mesh)
        });
        let first = cache.get(&original).id;
        cache.prepare([a.clone(), a.clone()], |old, mesh| {
            upload(&mut allocations, old, mesh)
        });
        assert_eq!(cache.get(&a).id, first);
        assert_eq!(cache.get(&a).x, 1.);
        assert_eq!(allocations, 1);
        cache.prepare([b.clone(), a.clone()], |old, mesh| {
            upload(&mut allocations, old, mesh)
        });
        assert_ne!(cache.get(&a).id, cache.get(&b).id);
        assert_eq!(cache.get(&a).x, 1.);
        assert_eq!(cache.get(&b).x, 2.);
        let second = cache.get(&b).id;
        cache.prepare([a, c.clone()], |old, mesh| {
            upload(&mut allocations, old, mesh)
        });
        assert_eq!(cache.get(&c).id, second);
        assert_eq!(allocations, 2);
        cache.prepare([original.clone(), c.clone()], |old, mesh| {
            upload(&mut allocations, old, mesh)
        });
        assert_eq!(cache.get(&original).id, first);
        assert_eq!(cache.get(&original).x, 0.);
        assert_eq!(cache.get(&c).x, 3.);
    }

    #[test]
    fn scene3d_geometry_shared_output_caches_release_retired_owners_before_reuse() {
        let a = mesh(0.);
        let b = update(&a, 2.);
        let c = update(&a, 4.);
        let mut color = GeometryCache::default();
        let mut ids = GeometryCache::default();
        let mut allocations = 0;
        color.prepare([a.clone()], |old, mesh| upload(&mut allocations, old, mesh));
        ids.reuse_from(&color);
        color.prepare([b.clone()], |old, mesh| upload(&mut allocations, old, mesh));
        assert_ne!(color.get(&b).id, ids.get(&a).id);
        assert_eq!(ids.get(&a).x, 0.);
        ids.reuse_from(&color);
        ids.retain([c.clone()]);
        let previous = color.get(&b).id;
        color.prepare([c.clone()], |old, mesh| upload(&mut allocations, old, mesh));
        assert_eq!(color.get(&c).id, previous);
        ids.reuse_from(&color);
        assert_eq!(ids.get(&c).id, previous);
        assert_eq!(ids.get(&c).x, 4.);
        assert_eq!(allocations, 2);
    }

    #[test]
    fn scene3d_geometry_topology_changes_allocate_and_empty_frames_release_resources() {
        let a = mesh(0.);
        let b = mesh(0.);
        let mut cache = GeometryCache::default();
        let mut allocations = 0;
        cache.prepare([a.clone()], |old, mesh| upload(&mut allocations, old, mesh));
        let first = cache.get(&a).id;
        cache.prepare([b.clone()], |old, mesh| upload(&mut allocations, old, mesh));
        assert_ne!(cache.get(&b).id, first);
        let resource = Arc::downgrade(&cache.entries[&key(&b)]);
        cache.prepare([], |old, mesh| upload(&mut allocations, old, mesh));
        assert!(resource.upgrade().is_none());
        assert!(cache.entries.is_empty());
    }
}
