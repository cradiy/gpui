use gpui::{LightKind3d, PunctualLight3d, Rgba};

/// A world-space directional, point, or spot light.
/// Rendering rejects non-finite values and unsupported ranges.
#[derive(Clone, Copy, Debug)]
pub struct PunctualLight(pub(crate) PunctualLight3d);

impl PunctualLight {
    /// Infinite source. Direction points from the surface toward the light.
    pub fn directional(direction: [f32; 3]) -> Self {
        Self(PunctualLight3d {
            kind: LightKind3d::Directional,
            position: [0.; 3],
            direction,
            color: gpui::white().into(),
            intensity: 1.,
            range: None,
            minimum_distance: 0.01,
            inner_angle: 0.,
            outer_angle: std::f32::consts::FRAC_PI_4,
        })
    }
    /// Omnidirectional source with inverse-square attenuation and no cutoff.
    pub fn point(position: [f32; 3]) -> Self {
        Self(PunctualLight3d {
            kind: LightKind3d::Point,
            position,
            ..Self::directional([0., 0., -1.]).0
        })
    }
    /// Cone source. Direction points outward from the light, unlike directional lights.
    /// Defaults to inner half-angle zero and outer half-angle pi/4.
    pub fn spot(position: [f32; 3], direction: [f32; 3]) -> Self {
        Self(PunctualLight3d {
            kind: LightKind3d::Spot,
            direction,
            ..Self::point(position).0
        })
    }
    /// sRGB color in [0, 1]. Alpha is ignored; defaults to white.
    pub fn color(mut self, color: impl Into<Rgba>) -> Self {
        self.0.color = color.into();
        self
    }
    /// Linear source multiplier in [0, 65504]. Defaults to 1.
    pub fn intensity(mut self, intensity: f32) -> Self {
        self.0.intensity = intensity;
        self
    }
    /// Positive world-space cutoff for point/spot lights, or None for unlimited range.
    pub fn range(mut self, range: Option<f32>) -> Self {
        self.0.range = range;
        self
    }
    /// Clamps attenuation distance, not source geometry. Defaults to 0.01 world units.
    /// Accepts [0.0001, 65504]; does not create an area light.
    pub fn minimum_distance(mut self, distance: f32) -> Self {
        self.0.minimum_distance = distance;
        self
    }
    /// Spot half-angles in radians: 0 <= inner < outer <= pi/2.
    pub fn cone_angles(mut self, inner: f32, outer: f32) -> Self {
        self.0.inner_angle = inner;
        self.0.outer_angle = outer;
        self
    }
}
