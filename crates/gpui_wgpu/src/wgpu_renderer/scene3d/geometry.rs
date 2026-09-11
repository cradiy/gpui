use collections::HashMap;
use gpui::Mesh3d;
use std::sync::Arc;

mod upload;
pub(super) use upload::Upload;

struct Entry<T> {
    mesh: Arc<Mesh3d>,
    resource: T,
}

pub(super) struct GeometryCache<T> {
    entries: HashMap<(usize, [u32; 5]), Arc<Entry<T>>>,
}

impl<T> Default for GeometryCache<T> {
    fn default() -> Self {
        Self {
            entries: HashMap::default(),
        }
    }
}

pub(super) fn key(mesh: &Arc<Mesh3d>, uv_sets: [u32; 5]) -> (usize, [u32; 5]) {
    (Arc::as_ptr(mesh) as usize, uv_sets)
}

fn topology(mesh: &Mesh3d) -> (usize, usize) {
    (mesh.indices().as_ptr() as usize, mesh.vertices().len())
}

impl<T> GeometryCache<T> {
    pub(super) fn prepare_shared<'a>(
        &mut self,
        meshes: Vec<(Arc<Mesh3d>, [u32; 5])>,
        peers: impl IntoIterator<Item = &'a mut Self>,
        upload: impl FnMut(Option<T>, &Mesh3d, [u32; 5]) -> T,
    ) where
        T: 'a,
    {
        let mut peers: Vec<_> = peers.into_iter().collect();
        for peer in &mut peers {
            peer.retain(meshes.iter().cloned());
        }
        self.prepare(meshes, upload);
        for peer in peers {
            peer.reuse_from(self);
        }
    }

    pub(super) fn prepare(
        &mut self,
        meshes: impl IntoIterator<Item = (Arc<Mesh3d>, [u32; 5])>,
        mut upload: impl FnMut(Option<T>, &Mesh3d, [u32; 5]) -> T,
    ) {
        let required: HashMap<_, _> = meshes
            .into_iter()
            .map(|(mesh, sets)| (key(&mesh, sets), mesh))
            .collect();
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
                    resource: upload(resource, &mesh, key.1),
                    mesh,
                })
            });
        }
    }

    pub(super) fn get(&self, mesh: &Arc<Mesh3d>, uv_sets: [u32; 5]) -> &T {
        &self.entries[&key(mesh, uv_sets)].resource
    }

    pub(super) fn resources(&self) -> impl Iterator<Item = &T> {
        self.entries.values().map(|entry| &entry.resource)
    }

    pub(super) fn reuse_from(&mut self, other: &Self) {
        self.entries.clone_from(&other.entries);
    }

    pub(super) fn retain(&mut self, meshes: impl IntoIterator<Item = (Arc<Mesh3d>, [u32; 5])>) {
        let required: collections::HashSet<_> = meshes
            .into_iter()
            .map(|(mesh, sets)| key(&mesh, sets))
            .collect();
        self.entries.retain(|key, _| required.contains(key));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scene3d_geometry_separates_uv_combinations_and_reuses_unchanged_snapshots() {
        let mesh = mesh(0.).with_uv_set(7, vec![[0.75, 0.25]; 3]).unwrap();
        let first = [0; 5];
        let second = [7, 0, 0, 0, 0];
        let mut cache = GeometryCache::default();
        let mut uploads = 0;
        cache.prepare(
            [(mesh.clone(), first), (mesh.clone(), second)],
            |_, mesh, sets| {
                uploads += 1;
                mesh.uv_at(sets[0], 0).unwrap()
            },
        );
        assert_eq!(cache.get(&mesh, first), &[0., 0.]);
        assert_eq!(cache.get(&mesh, second), &[0.75, 0.25]);
        cache.prepare([(mesh.clone(), second)], |_, _, _| {
            panic!("unchanged UV combination")
        });
        assert_eq!(uploads, 2);
        assert_eq!(cache.entries.len(), 1);
        cache.prepare([(mesh.clone(), first)], |old, mesh, sets| {
            assert_eq!(old, Some([0.75, 0.25]));
            mesh.uv_at(sets[0], 0).unwrap()
        });
        assert_eq!(cache.get(&mesh, first), &[0., 0.]);
    }

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
        cache.prepare(
            [original.clone()].into_iter().map(|mesh| (mesh, [0; 5])),
            |old, mesh, _| upload(&mut allocations, old, mesh),
        );
        let first = cache.get(&original, [0; 5]).id;
        cache.prepare(
            [a.clone(), a.clone()]
                .into_iter()
                .map(|mesh| (mesh, [0; 5])),
            |old, mesh, _| upload(&mut allocations, old, mesh),
        );
        assert_eq!(cache.get(&a, [0; 5]).id, first);
        assert_eq!(cache.get(&a, [0; 5]).x, 1.);
        assert_eq!(allocations, 1);
        cache.prepare(
            [b.clone(), a.clone()]
                .into_iter()
                .map(|mesh| (mesh, [0; 5])),
            |old, mesh, _| upload(&mut allocations, old, mesh),
        );
        assert_ne!(cache.get(&a, [0; 5]).id, cache.get(&b, [0; 5]).id);
        assert_eq!(cache.get(&a, [0; 5]).x, 1.);
        assert_eq!(cache.get(&b, [0; 5]).x, 2.);
        let second = cache.get(&b, [0; 5]).id;
        cache.prepare(
            [a, c.clone()].into_iter().map(|mesh| (mesh, [0; 5])),
            |old, mesh, _| upload(&mut allocations, old, mesh),
        );
        assert_eq!(cache.get(&c, [0; 5]).id, second);
        assert_eq!(allocations, 2);
        cache.prepare(
            [original.clone(), c.clone()]
                .into_iter()
                .map(|mesh| (mesh, [0; 5])),
            |old, mesh, _| upload(&mut allocations, old, mesh),
        );
        assert_eq!(cache.get(&original, [0; 5]).id, first);
        assert_eq!(cache.get(&original, [0; 5]).x, 0.);
        assert_eq!(cache.get(&c, [0; 5]).x, 3.);
    }

    #[test]
    fn scene3d_geometry_shared_output_caches_release_retired_owners_before_reuse() {
        let a = mesh(0.);
        let b = update(&a, 2.);
        let c = update(&a, 4.);
        let mut color = GeometryCache::default();
        let mut ids = GeometryCache::default();
        let mut allocations = 0;
        color.prepare(
            [a.clone()].into_iter().map(|mesh| (mesh, [0; 5])),
            |old, mesh, _| upload(&mut allocations, old, mesh),
        );
        ids.reuse_from(&color);
        color.prepare(
            [b.clone()].into_iter().map(|mesh| (mesh, [0; 5])),
            |old, mesh, _| upload(&mut allocations, old, mesh),
        );
        assert_ne!(color.get(&b, [0; 5]).id, ids.get(&a, [0; 5]).id);
        assert_eq!(ids.get(&a, [0; 5]).x, 0.);
        ids.reuse_from(&color);
        ids.retain([(c.clone(), [0; 5])]);
        let previous = color.get(&b, [0; 5]).id;
        color.prepare(
            [c.clone()].into_iter().map(|mesh| (mesh, [0; 5])),
            |old, mesh, _| upload(&mut allocations, old, mesh),
        );
        assert_eq!(color.get(&c, [0; 5]).id, previous);
        ids.reuse_from(&color);
        assert_eq!(ids.get(&c, [0; 5]).id, previous);
        assert_eq!(ids.get(&c, [0; 5]).x, 4.);
        assert_eq!(allocations, 2);
    }

    #[test]
    fn scene3d_geometry_pool_shares_consumers_and_preserves_live_snapshots() {
        let original = mesh(0.);
        let a = update(&original, 1.);
        let b = update(&original, 2.);
        let c = update(&original, 3.);
        let mut pool = GeometryCache::default();
        let mut low = GeometryCache::default();
        let mut high = GeometryCache::default();
        let mut allocations = 0;
        pool.prepare_shared(
            vec![(original.clone(), [0; 5]), (original.clone(), [0; 5])],
            [&mut low, &mut high],
            |old, mesh, _| upload(&mut allocations, old, mesh),
        );
        assert_eq!(allocations, 1);
        assert_eq!(
            low.get(&original, [0; 5]).id,
            high.get(&original, [0; 5]).id
        );
        let original_entry = Arc::downgrade(&pool.entries[&key(&original, [0; 5])]);
        pool.prepare_shared(
            vec![(original.clone(), [0; 5]), (a.clone(), [0; 5])],
            [&mut low, &mut high],
            |old, mesh, _| upload(&mut allocations, old, mesh),
        );
        low.prepare([(original.clone(), [0; 5])], |_, _, _| {
            panic!("shared mesh uploaded again")
        });
        high.prepare([(a.clone(), [0; 5])], |_, _, _| {
            panic!("shared mesh uploaded again")
        });
        assert_eq!(allocations, 2);
        assert_eq!(low.get(&original, [0; 5]).x, 0.);
        assert_eq!(high.get(&a, [0; 5]).x, 1.);
        assert_ne!(low.get(&original, [0; 5]).id, high.get(&a, [0; 5]).id);
        let a_entry = Arc::downgrade(&pool.entries[&key(&a, [0; 5])]);
        pool.prepare_shared(
            vec![(b.clone(), [0; 5])],
            [&mut low, &mut high],
            |old, mesh, _| upload(&mut allocations, old, mesh),
        );
        assert_eq!(allocations, 2);
        assert!(original_entry.upgrade().is_none());
        assert!(a_entry.upgrade().is_none());
        assert_eq!(low.get(&b, [0; 5]).id, high.get(&b, [0; 5]).id);
        assert_eq!(high.get(&b, [0; 5]).x, 2.);
        assert_eq!(pool.entries.len(), 1);

        let mut retained = GeometryCache::default();
        retained.reuse_from(&high);
        let b_entry = Arc::downgrade(&pool.entries[&key(&b, [0; 5])]);
        pool.prepare_shared(
            vec![(c.clone(), [0; 5])],
            [&mut low, &mut high],
            |old, mesh, _| upload(&mut allocations, old, mesh),
        );
        assert_eq!(allocations, 3);
        assert_eq!(retained.get(&b, [0; 5]).x, 2.);
        assert_eq!(low.get(&c, [0; 5]).x, 3.);
        assert_ne!(retained.get(&b, [0; 5]).id, low.get(&c, [0; 5]).id);
        let c_entry = Arc::downgrade(&pool.entries[&key(&c, [0; 5])]);
        pool.prepare_shared(Vec::new(), [&mut low, &mut high], |_, _, _| unreachable!());
        assert!(pool.entries.is_empty() && low.entries.is_empty() && high.entries.is_empty());
        assert!(c_entry.upgrade().is_none());
        assert!(b_entry.upgrade().is_some());
        drop(retained);
        assert!(b_entry.upgrade().is_none());
    }

    #[test]
    fn scene3d_geometry_topology_changes_allocate_and_empty_frames_release_resources() {
        let a = mesh(0.);
        let b = mesh(0.);
        let mut cache = GeometryCache::default();
        let mut allocations = 0;
        cache.prepare(
            [a.clone()].into_iter().map(|mesh| (mesh, [0; 5])),
            |old, mesh, _| upload(&mut allocations, old, mesh),
        );
        let first = cache.get(&a, [0; 5]).id;
        cache.prepare(
            [b.clone()].into_iter().map(|mesh| (mesh, [0; 5])),
            |old, mesh, _| upload(&mut allocations, old, mesh),
        );
        assert_ne!(cache.get(&b, [0; 5]).id, first);
        let resource = Arc::downgrade(&cache.entries[&key(&b, [0; 5])]);
        cache.prepare([].into_iter().map(|mesh| (mesh, [0; 5])), |old, mesh, _| {
            upload(&mut allocations, old, mesh)
        });
        assert!(resource.upgrade().is_none());
        assert!(cache.entries.is_empty());
    }
}
