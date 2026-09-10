use crate::{Aabb, QueryObject, Scene};

impl Scene {
    /// Returns conservative world-AABB overlap candidates in scene object order.
    /// Contact is included. Uses the shared object BVH without traversing triangles;
    /// padded transformed bounds can include false positives. Objects without
    /// computable finite bounds remain candidates rather than being culled.
    /// Camera clipping, occlusion, material alpha, and picking behavior are ignored.
    /// Evaluated scenes contain only meshes with inherited visibility enabled.
    /// Results are not exact mesh intersections or confirmed visible surfaces.
    pub fn bounds_candidates(&self, region: Aabb) -> Vec<QueryObject<'_>> {
        self.bounds_candidates_where(region, |_| true)
    }

    /// Filters bounds candidates by node/application identity or caller policy.
    /// The predicate runs once per overlapping conservative bound, including
    /// unbounded objects, in scene object order. Rejected candidates have no effect
    /// on other results. Queries leave rendering and retained snapshots unchanged.
    pub fn bounds_candidates_where(
        &self,
        region: Aabb,
        mut filter: impl FnMut(QueryObject<'_>) -> bool,
    ) -> Vec<QueryObject<'_>> {
        let mut indices = Vec::new();
        self.visit_bounds(region, |index| indices.push(index));
        indices.sort_unstable();
        indices
            .into_iter()
            .map(|index| self.query_object(index))
            .filter(|object| filter(*object))
            .collect()
    }
}
