use anyhow::{Context, Result, ensure};
use gltf::khr_lights_punctual::Kind;
use gpui::Rgba;
use gpui_3d::{AffineTransform, PunctualLight};

use crate::PreparedDocument;

impl PreparedDocument {
    /// Converts a punctual light in local coordinates. Point and spot sources
    /// are at the origin; directional and spot emission travels along -Z.
    pub fn light(&self, index: usize) -> Result<PunctualLight> {
        self.convert_light(index)
            .with_context(|| format!("light {index}"))
    }

    fn convert_light(&self, index: usize) -> Result<PunctualLight> {
        crate::validation::supported_extensions(self.gltf())?;
        let source = self
            .gltf()
            .lights()
            .and_then(|mut lights| lights.nth(index))
            .context("light index out of range")?;
        let color = source.color();
        ensure!(
            color
                .iter()
                .all(|v| v.is_finite() && (0. ..=1.).contains(v)),
            "linear light color must be finite and in [0, 1]"
        );
        let light = match source.kind() {
            Kind::Directional => {
                ensure!(
                    source.range().is_none(),
                    "directional light cannot have a range"
                );
                PunctualLight::directional([0., 0., 1.])
            }
            Kind::Point => PunctualLight::point([0.; 3]),
            Kind::Spot {
                inner_cone_angle,
                outer_cone_angle,
            } => PunctualLight::spot([0.; 3], [0., 0., -1.])
                .cone_angles(inner_cone_angle, outer_cone_angle),
        };
        let [r, g, b] = color.map(crate::material::srgb);
        light
            .color(Rgba { r, g, b, a: 1. })
            .intensity(source.intensity())
            .range(source.range())
            .transformed(AffineTransform::IDENTITY)
            .context("invalid light parameters")
    }
}
