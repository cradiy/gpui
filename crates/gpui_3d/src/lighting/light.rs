use crate::AffineTransform;
use gpui::{LightKind3d, PunctualLight3d, Rgba};
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LightError {
    InvalidParameters,
    Unrepresentable,
}

impl fmt::Display for LightError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidParameters => "light parameters are nonfinite or outside supported ranges",
            Self::Unrepresentable => "transformed light exceeds finite coordinate precision",
        })
    }
}
impl std::error::Error for LightError {}

/// A directional, point, or spot light. `Scene` consumes world-space sources;
/// `Node` consumes local-space sources. Invalid values fail evaluation or rendering.
#[derive(Clone, Copy, Debug)]
pub struct PunctualLight(pub(crate) PunctualLight3d);

impl PunctualLight {
    pub fn kind(self) -> LightKind3d {
        self.0.kind
    }
    /// Source position, ignored for directional lights.
    pub fn position(self) -> [f32; 3] {
        self.0.position
    }
    /// Direction toward a directional source or outward from a spot; ignored for points.
    pub fn direction(self) -> [f32; 3] {
        self.0.direction
    }
    /// Transforms local position and direction. Directions use the linear part
    /// and are normalized. Range, distance clamp, cone angles, color and intensity
    /// remain unchanged, including under nonuniform scale or shear.
    pub fn transformed(mut self, transform: AffineTransform) -> Result<Self, LightError> {
        if !self.0.is_valid() {
            return Err(LightError::InvalidParameters);
        }
        let matrix = transform.matrix();
        if self.0.kind != LightKind3d::Directional {
            self.0.position = std::array::from_fn(|r| {
                ((0..3)
                    .map(|c| f64::from(matrix[c][r]) * f64::from(self.0.position[c]))
                    .sum::<f64>()
                    + f64::from(matrix[3][r])) as f32
            });
        }
        if self.0.kind != LightKind3d::Point {
            let direction: [f64; 3] = std::array::from_fn(|r| {
                (0..3)
                    .map(|c| f64::from(matrix[c][r]) * f64::from(self.0.direction[c]))
                    .sum()
            });
            let length = direction.iter().map(|v| v * v).sum::<f64>().sqrt();
            self.0.direction = direction.map(|v| (v / length) as f32);
        }
        if !self.0.is_valid() {
            return Err(LightError::Unrepresentable);
        }
        Ok(self)
    }

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

/// One directional light plus uniform ambient illumination.
#[derive(Clone, Copy, Debug)]
pub struct Light {
    /// Direction toward the light in world space.
    pub direction: [f32; 3],
    /// sRGB light color; alpha is ignored.
    pub color: Rgba,
    /// Direct light multiplier.
    pub intensity: f32,
    /// Ambient light multiplier.
    pub ambient: f32,
}
impl Default for Light {
    fn default() -> Self {
        Self {
            direction: [-0.5, 0.8, 0.7],
            color: gpui::rgb(0xe7efff),
            intensity: 0.75,
            ambient: 0.3,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transformed_lights_preserve_photometric_parameters_and_ignore_unused_coordinates() {
        let transform = AffineTransform::from_matrix([
            [-2., 0., 0., 0.],
            [1., 3., 0., 0.],
            [0., 0., 4., 0.],
            [5., 6., 7., 1.],
        ])
        .unwrap();
        let spot = PunctualLight::spot([1., 2., 3.], [1., 1., 0.])
            .range(Some(10.))
            .intensity(12.)
            .minimum_distance(0.5)
            .cone_angles(0.2, 0.7);
        let result = spot.transformed(transform).unwrap();
        assert_eq!(result.kind(), LightKind3d::Spot);
        assert_eq!(result.position(), [5., 12., 19.]);
        let k = 1. / 10_f32.sqrt();
        for (a, b) in result.direction().into_iter().zip([-k, 3. * k, 0.]) {
            assert!((a - b).abs() < 1e-6);
        }
        assert_eq!(result.0.range, spot.0.range);
        assert_eq!(result.0.intensity, spot.0.intensity);
        assert_eq!(result.0.minimum_distance, spot.0.minimum_distance);
        assert_eq!((result.0.inner_angle, result.0.outer_angle), (0.2, 0.7));
        assert_eq!(result.0.color, spot.0.color);
        let large = AffineTransform::from_translation([f32::MAX; 3]).unwrap();
        let sun = PunctualLight::directional([f32::MAX, 0., 0.]);
        assert_eq!(sun.transformed(large).unwrap().direction(), [1., 0., 0.]);
        assert_eq!(
            sun.transformed(transform).unwrap().direction(),
            [-1., 0., 0.]
        );
        assert_eq!(
            PunctualLight::point([0.; 3])
                .transformed(transform)
                .unwrap()
                .position(),
            [5., 6., 7.]
        );
        assert_eq!(
            PunctualLight::point([f32::MAX; 3])
                .transformed(large)
                .unwrap_err(),
            LightError::Unrepresentable
        );
        assert_eq!(
            PunctualLight::spot([0.; 3], [0.; 3])
                .transformed(transform)
                .unwrap_err(),
            LightError::InvalidParameters
        );
    }
}
