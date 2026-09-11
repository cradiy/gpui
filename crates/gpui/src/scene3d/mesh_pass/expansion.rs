use super::*;

/// Units for displacement along a vertex's geometric normal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum MeshPassSpace3d {
    /// World units, independent of object scale.
    World = 1,
    /// Render-target pixels along the projected normal; clip Z and W are unchanged.
    Pixels = 2,
}

/// Bounded normal displacement for one additional color pass.
#[derive(Clone, Debug, PartialEq)]
pub struct MeshPassExpansion3d {
    /// Displacement coordinate system.
    pub space: MeshPassSpace3d,
    /// Signed finite displacement. Negative values move inward.
    pub amount: f32,
    /// Optional declared Float32 custom attribute, sampled per vertex.
    pub weight_attribute: Option<crate::SharedString>,
    /// Finite nonnegative clamp for weights; ignored when no attribute is selected.
    pub weight_limit: f32,
}

impl MeshPassExpansion3d {
    /// Constant normal displacement in the selected coordinate system.
    pub fn new(space: MeshPassSpace3d, amount: f32) -> Self {
        Self {
            space,
            amount,
            weight_attribute: None,
            weight_limit: 1.,
        }
    }

    /// Multiplies displacement by a Float32 stream clamped to [0, limit].
    pub fn weight(mut self, attribute: impl Into<crate::SharedString>, limit: f32) -> Self {
        self.weight_attribute = Some(attribute.into());
        self.weight_limit = limit;
        self
    }

    /// Whether controls and their maximum product fit finite shader arithmetic.
    pub fn is_valid(&self) -> bool {
        self.amount.is_finite()
            && self.weight_limit.is_finite()
            && self.weight_limit >= 0.
            && self.maximum_displacement() <= f64::from(f32::MAX)
    }

    /// Conservative displacement magnitude before normal projection.
    pub fn maximum_displacement(&self) -> f64 {
        f64::from(self.amount).abs()
            * if self.weight_attribute.is_some() {
                f64::from(self.weight_limit)
            } else {
                1.
            }
    }
}

impl MeshPass3d {
    /// Conservative camera visibility, preserving primary mesh/query bounds.
    /// Pixel expansion retains side-plane candidates until raster clipping, because
    /// target pixel dimensions are not part of scene preparation. Near/far clipping
    /// remains active. World expansion includes its maximum displacement at each plane.
    pub fn intersects_clip_volume(
        &self,
        bounds: [[f32; 3]; 2],
        model: [[f32; 4]; 4],
        camera: [[f32; 4]; 4],
    ) -> bool {
        let (radius, screen) = self.expansion.as_ref().map_or((0., false), |expansion| {
            let maximum = expansion.maximum_displacement();
            match expansion.space {
                MeshPassSpace3d::World => (maximum, false),
                MeshPassSpace3d::Pixels => (0., maximum > 0.),
            }
        });
        super::super::Mesh3d::expanded_bounds_intersect_clip_volume(
            bounds, model, camera, radius, screen,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const IDENTITY: [[f32; 4]; 4] = [
        [1., 0., 0., 0.],
        [0., 1., 0., 0.],
        [0., 0., 1., 0.],
        [0., 0., 0., 1.],
    ];

    #[test]
    fn expanded_visibility_preserves_world_units_and_screen_depth_planes() {
        let mut pass = MeshPass3d {
            material: MeshMaterial3d::new(std::sync::Arc::new(())),
            state: Default::default(),
            expansion: Some(MeshPassExpansion3d::new(MeshPassSpace3d::World, 0.2)),
        };
        let outside = [[1.1, 0., 0.5]; 2];
        assert!(pass.intersects_clip_volume(outside, IDENTITY, IDENTITY));
        pass.expansion.as_mut().unwrap().amount = -0.2;
        assert!(pass.intersects_clip_volume(outside, IDENTITY, IDENTITY));
        pass.expansion.as_mut().unwrap().amount = 0.05;
        assert!(!pass.intersects_clip_volume(outside, IDENTITY, IDENTITY));
        pass.expansion =
            Some(MeshPassExpansion3d::new(MeshPassSpace3d::World, 0.1).weight("width", 2.));
        assert!(pass.intersects_clip_volume(outside, IDENTITY, IDENTITY));
        let mut reflected = IDENTITY;
        reflected[0][0] = -2.;
        reflected[3][0] = 3.3;
        assert!(pass.intersects_clip_volume(outside, reflected, IDENTITY));
        pass.expansion.as_mut().unwrap().weight_limit = 0.25;
        assert!(!pass.intersects_clip_volume(outside, reflected, IDENTITY));
        let behind_near = [[0., 0., -0.1]; 2];
        pass.expansion = Some(MeshPassExpansion3d::new(MeshPassSpace3d::World, 0.2));
        assert!(pass.intersects_clip_volume(behind_near, IDENTITY, IDENTITY));
        pass.expansion = Some(MeshPassExpansion3d::new(MeshPassSpace3d::Pixels, 8.));
        assert!(pass.intersects_clip_volume(outside, IDENTITY, IDENTITY));
        assert!(!pass.intersects_clip_volume(behind_near, IDENTITY, IDENTITY));
        assert!(!pass.intersects_clip_volume([[0., 0., 1.1]; 2], IDENTITY, IDENTITY));
        pass.expansion.as_mut().unwrap().amount = 0.;
        assert!(!pass.intersects_clip_volume(outside, IDENTITY, IDENTITY));
        pass.expansion = None;
        assert!(!pass.intersects_clip_volume(outside, IDENTITY, IDENTITY));
    }
}
