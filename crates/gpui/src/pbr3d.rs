/// Metallic-roughness material parameters for direct microfacet lighting.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PbrMaterial3d {
    /// Metal fraction in [0, 1]. Zero is dielectric; one has no diffuse response.
    pub metallic: f32,
    /// Perceptual roughness in [0, 1]; shading uses a minimum of 0.045.
    pub roughness: f32,
    /// Additive linear RGB radiance in [0, 65504], independent of scene lights.
    /// Does not illuminate other objects.
    pub emissive: [f32; 3],
}

impl Default for PbrMaterial3d {
    fn default() -> Self {
        Self {
            metallic: 0.,
            roughness: 0.5,
            emissive: [0.; 3],
        }
    }
}

impl PbrMaterial3d {
    /// Whether every parameter is finite and within its supported range.
    pub fn is_valid(self) -> bool {
        self.metallic.is_finite()
            && (0.0..=1.0).contains(&self.metallic)
            && self.roughness.is_finite()
            && (0.0..=1.0).contains(&self.roughness)
            && self
                .emissive
                .iter()
                .all(|v| v.is_finite() && (0.0..=65504.0).contains(v))
    }
}
