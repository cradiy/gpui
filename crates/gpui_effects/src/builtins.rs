use crate::{Effect, effect, image_effect};
use gpui::prelude::*;
use gpui::{EffectShader, ImageSource, Rgba};

fn color_slot(color: impl Into<Rgba>) -> [f32; 4] {
    let color = color.into();
    [color.r, color.g, color.b, color.a]
}

/// Returns an animated aurora-style effect with four customizable colors.
pub fn aurora<C>(colors: [C; 4]) -> Effect
where
    C: Copy + Into<Rgba>,
{
    effect(aurora_shader())
        .uniform(0, color_slot(colors[0]))
        .uniform(1, color_slot(colors[1]))
        .uniform(2, color_slot(colors[2]))
        .uniform(3, color_slot(colors[3]))
        .uniform(4, [1.15, 0.8, 1.0, 0.0])
}

/// Returns the portable shader used by [`aurora`].
pub fn aurora_shader() -> EffectShader {
    EffectShader::wgsl(AURORA_WGSL)
}

/// Returns a smoothly looping plasma effect with four customizable colors.
pub fn plasma<C>(colors: [C; 4]) -> Effect
where
    C: Copy + Into<Rgba>,
{
    effect(plasma_shader())
        .uniform(0, color_slot(colors[0]))
        .uniform(1, color_slot(colors[1]))
        .uniform(2, color_slot(colors[2]))
        .uniform(3, color_slot(colors[3]))
        .uniform(4, [1.0, 1.0, 1.0, 0.0])
}

/// Returns the portable shader used by [`plasma`].
pub fn plasma_shader() -> EffectShader {
    EffectShader::wgsl(PLASMA_WGSL)
}

/// Returns a smoothly looping color-orb fusion effect.
pub fn color_orbs<C>(colors: [C; 4]) -> Effect
where
    C: Copy + Into<Rgba>,
{
    effect(color_orbs_shader())
        .uniform(0, color_slot(colors[0]))
        .uniform(1, color_slot(colors[1]))
        .uniform(2, color_slot(colors[2]))
        .uniform(3, color_slot(colors[3]))
        .uniform(4, [4.5, 1.0, 1.0, 0.0])
}

/// Returns the portable shader used by [`color_orbs`].
pub fn color_orbs_shader() -> EffectShader {
    EffectShader::wgsl(COLOR_ORBS_WGSL)
}

/// Returns a defined water-ripple treatment derived from album artwork.
///
/// Unlike [`crate::color_flow`], this preset deliberately exposes the bright and dark
/// edges of each expanding ring.
pub fn album_ripples(source: impl Into<ImageSource>) -> Effect {
    image_effect(source, album_ripples_shader())
        .uniform(0, [0.14, 1.65, 0.9, 1.0])
        .uniform(1, [0.48, 0.075, 0.28, 1.0])
        .bg(gpui::rgb(0x2a2834))
}

/// Returns the image-sampling shader used by [`album_ripples`].
pub fn album_ripples_shader() -> EffectShader {
    EffectShader::wgsl_image(ALBUM_RIPPLES_WGSL)
}

const AURORA_WGSL: &str = include_str!("shaders/aurora.wgsl");

const PLASMA_WGSL: &str = include_str!("shaders/plasma.wgsl");

const COLOR_ORBS_WGSL: &str = include_str!("shaders/color_orbs.wgsl");

const ALBUM_RIPPLES_WGSL: &str = include_str!("shaders/album_ripples.wgsl");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color_flow_shader;

    #[test]
    fn color_flow_shader_is_valid_wgsl() {
        let source = gpui::compose_effect_shader_wgsl(&color_flow_shader());
        let module = naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|error| panic!("{}", error.emit_to_string(&source)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("color flow shader must validate");
    }

    #[test]
    fn built_in_shaders_have_stable_distinct_ids() {
        let colors = [
            gpui::rgb(0xff1744),
            gpui::rgb(0x2979ff),
            gpui::rgb(0x00e676),
            gpui::rgb(0xffea00),
        ];
        let aurora_id = aurora(colors).shader().id();
        assert_eq!(aurora_id, aurora(colors).shader().id());
        assert_ne!(aurora_id, plasma(colors).shader().id());
        assert_ne!(aurora_id, color_orbs(colors).shader().id());
        assert!(color_flow_shader().uses_image());
        assert!(album_ripples_shader().uses_image());
        assert_ne!(color_flow_shader().id(), album_ripples_shader().id());
    }
}
