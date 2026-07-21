use crate::{MaskedEffect, effect_svg, effect_text};
use gpui::{Div, EffectShader, Element, Rgba, SharedString, Svg};

fn color_slot(color: impl Into<Rgba>) -> [f32; 4] {
    let color = color.into();
    [color.r, color.g, color.b, color.a]
}

fn configure_spectrum<E, C>(effect: MaskedEffect<E>, colors: [C; 4]) -> MaskedEffect<E>
where
    E: Element,
    C: Copy + Into<Rgba>,
{
    effect
        .uniform(0, color_slot(colors[0]))
        .uniform(1, color_slot(colors[1]))
        .uniform(2, color_slot(colors[2]))
        .uniform(3, color_slot(colors[3]))
        .uniform(4, [1.0, 0.16, 0.08, 0.0])
}

/// Creates animated spectrum text with four customizable colors.
pub fn spectrum_text<C>(text: impl Into<SharedString>, colors: [C; 4]) -> MaskedEffect<Div>
where
    C: Copy + Into<Rgba>,
{
    configure_spectrum(effect_text(text, spectrum_mask_shader()), colors)
}

/// Creates an animated spectrum-filled monochrome SVG.
pub fn spectrum_svg<C>(path: impl Into<SharedString>, colors: [C; 4]) -> MaskedEffect<Svg>
where
    C: Copy + Into<Rgba>,
{
    configure_spectrum(effect_svg(path, spectrum_mask_shader()), colors)
}

/// Returns the portable mask shader used by [`spectrum_text`] and [`spectrum_svg`].
pub fn spectrum_mask_shader() -> EffectShader {
    EffectShader::wgsl_mask(SPECTRUM_MASK_WGSL)
}

const SPECTRUM_MASK_WGSL: &str = r#"
fn spectrum_palette(position: f32, params: EffectParams) -> vec4<f32> {
    let cursor = fract(position) * 4.0;
    let index = u32(floor(cursor));
    let next_index = (index + 1u) % 4u;
    let t = fract(cursor);
    let smooth_t = t * t * (3.0 - 2.0 * t);
    return mix(params.slots[index], params.slots[next_index], smooth_t);
}

fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> {
    let tau = 6.28318530718;
    let phase = input.time * tau;
    let scale = max(params.slots[4].x, 0.1);
    let bend = params.slots[4].y;
    let shimmer = params.slots[4].z;
    let wave = sin(input.uv.y * tau * 1.35 + phase) * bend
        + sin((input.uv.x + input.uv.y) * tau * 0.72 - phase * 2.0) * bend * 0.45;
    let cursor = input.uv.x * scale + wave - input.time;
    var color = spectrum_palette(cursor, params);
    let light = 1.0 + sin((input.uv.x * 1.7 - input.uv.y) * tau + phase * 2.0) * shimmer;
    color = vec4<f32>(max(color.rgb * light, vec3<f32>(0.0)), color.a);
    return color;
}
"#;
