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

/// Returns a blurred, slowly moving atmosphere derived from album artwork.
///
/// Uniform slot 0 contains diffusion, saturation, brightness, and motion
/// strength. Slot 1 contains glow strength, ripple displacement, ripple light,
/// and ring definition. Callers may override either slot with
/// [`Effect::uniform`].
pub fn album_glow(source: impl Into<ImageSource>) -> Effect {
    image_effect(source, album_glow_shader())
        .uniform(0, [0.14, 1.65, 0.9, 1.0])
        .uniform(1, [0.36, 0.045, 0.06, 0.0])
        .bg(gpui::rgb(0x2a2834))
}

/// Returns a defined water-ripple treatment derived from album artwork.
///
/// Unlike [`album_glow`], this preset deliberately exposes the bright and dark
/// edges of each expanding ring.
pub fn album_ripples(source: impl Into<ImageSource>) -> Effect {
    image_effect(source, album_ripples_shader())
        .uniform(0, [0.14, 1.65, 0.9, 1.0])
        .uniform(1, [0.48, 0.075, 0.28, 1.0])
        .bg(gpui::rgb(0x2a2834))
}

/// Returns the image-sampling shader used by [`album_glow`] and [`album_ripples`].
pub fn album_glow_shader() -> EffectShader {
    EffectShader::wgsl_image(ALBUM_GLOW_WGSL)
}

/// Returns the image-sampling shader used by [`album_ripples`].
pub fn album_ripples_shader() -> EffectShader {
    album_glow_shader()
}

const AURORA_WGSL: &str = r#"
fn aurora_band(distance: f32, width: f32) -> f32 {
    let normalized = distance / max(width, 0.0001);
    return exp(-normalized * normalized);
}

fn aurora_palette(position: f32, params: EffectParams) -> vec4<f32> {
    let cursor = fract(position) * 4.0;
    let distance0 = min(abs(cursor), 4.0 - abs(cursor));
    let distance1 = min(abs(cursor - 1.0), 4.0 - abs(cursor - 1.0));
    let distance2 = min(abs(cursor - 2.0), 4.0 - abs(cursor - 2.0));
    let distance3 = min(abs(cursor - 3.0), 4.0 - abs(cursor - 3.0));
    let weights = vec4<f32>(
        1.0 - smoothstep(0.0, 1.0, distance0),
        1.0 - smoothstep(0.0, 1.0, distance1),
        1.0 - smoothstep(0.0, 1.0, distance2),
        1.0 - smoothstep(0.0, 1.0, distance3),
    );
    let total = max(dot(weights, vec4<f32>(1.0)), 0.0001);
    return (
        params.slots[0] * weights.x +
        params.slots[1] * weights.y +
        params.slots[2] * weights.z +
        params.slots[3] * weights.w
    ) / total;
}

fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> {
    let tau = 6.28318530718;
    let phase = input.time * tau;
    let scale = params.slots[4].x;
    let x = input.uv.x * scale;
    let y = input.uv.y;

    // Three independent curtains avoid the horizontal layers produced by a
    // conventional stacked gradient. Every displacement is analytic, so the
    // motion remains continuous without a noise grid becoming visible.
    let center0 = 0.42
        + sin(x * tau * 0.72 + phase) * 0.13
        + sin(x * tau * 1.85 - phase * 1.7) * 0.035;
    let center1 = 0.55
        + sin(x * tau * 0.58 - phase + 1.7) * 0.16
        + cos(x * tau * 2.2 + phase * 2.0) * 0.03;
    let center2 = 0.67
        + sin(x * tau * 0.91 + phase + 3.4) * 0.12
        + sin(x * tau * 2.65 - phase * 2.0) * 0.025;

    let folds0 = 0.72 + 0.28 * pow(0.5 + 0.5 * sin(x * tau * 8.0 + phase * 2.0), 2.0);
    let folds1 = 0.72 + 0.28 * pow(0.5 + 0.5 * sin(x * tau * 10.0 - phase), 2.0);
    let folds2 = 0.72 + 0.28 * pow(0.5 + 0.5 * cos(x * tau * 7.0 + phase), 2.0);
    let band0 = aurora_band(y - center0, 0.19) * folds0;
    let band1 = aurora_band(y - center1, 0.22) * folds1;
    let band2 = aurora_band(y - center2, 0.24) * folds2;

    // A wider, dimmer halo turns the ribbons into soft light curtains rather
    // than bright contour lines.
    let halo0 = aurora_band(y - center0, 0.38) * 0.22;
    let halo1 = aurora_band(y - center1, 0.43) * 0.19;
    let halo2 = aurora_band(y - center2, 0.46) * 0.17;
    let intensity0 = band0 + halo0;
    let intensity1 = band1 + halo1;
    let intensity2 = band2 + halo2;

    let color0 = aurora_palette(x * 0.23 + sin(phase) * 0.18, params);
    let color1 = aurora_palette(x * 0.19 + sin(-phase + 1.1) * 0.14 + 0.31, params);
    let color2 = aurora_palette(x * 0.16 + sin(phase + 0.7) * 0.11 + 0.62, params);
    let total = max(intensity0 + intensity1 + intensity2, 0.0001);
    let curtain = (
        color0 * intensity0 +
        color1 * intensity1 +
        color2 * intensity2
    ) / total;

    let background_color = aurora_palette(input.uv.x * 0.32 + sin(phase) * 0.06, params);
    let background = background_color.rgb * mix(0.16, 0.27, y);
    let strength = 1.0 - exp(-total * 0.92);
    let glow = 0.9 + 0.18 * clamp(total / 2.4, 0.0, 1.0);
    let rgb = pow(
        max(mix(background, curtain.rgb * glow, strength), vec3<f32>(0.0)),
        vec3<f32>(0.88),
    );
    let alpha = dot(vec4<f32>(
        params.slots[0].a,
        params.slots[1].a,
        params.slots[2].a,
        params.slots[3].a,
    ), vec4<f32>(0.25));
    return vec4<f32>(rgb, alpha);
}
"#;

const PLASMA_WGSL: &str = r#"
fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> {
    let tau = 6.28318530718;
    let phase = input.time * tau;
    let point = (input.uv - vec2<f32>(0.5)) * params.slots[4].x;
    let a = sin((point.x * 5.0 + phase) + sin(point.y * 4.0 - phase));
    let b = sin((point.y * 6.0 - phase) + cos(point.x * 3.0 + phase));
    let c = sin(length(point + vec2<f32>(cos(phase), sin(phase)) * 0.35) * 9.0 - phase);
    let first = mix(params.slots[0], params.slots[1], 0.5 + 0.5 * a);
    let second = mix(params.slots[2], params.slots[3], 0.5 + 0.5 * b);
    return mix(first, second, 0.5 + 0.5 * c);
}
"#;

const COLOR_ORBS_WGSL: &str = r#"
fn effect_orb_weight(point: vec2<f32>, center: vec2<f32>, sharpness: f32) -> f32 {
    let delta = point - center;
    return exp(-dot(delta, delta) * sharpness);
}

fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> {
    let tau = 6.28318530718;
    let phase = input.time * tau;
    let aspect = input.size.x / max(input.size.y, 1.0);
    let point = (input.uv - vec2<f32>(0.5)) * vec2<f32>(aspect, 1.0);
    let radius = vec2<f32>(0.32 * aspect, 0.3);
    let center0 = vec2<f32>(cos(phase), sin(phase)) * radius;
    let center1 = vec2<f32>(cos(phase + 1.5707963), sin(phase * 2.0 + 1.2)) * radius;
    let center2 = vec2<f32>(cos(-phase + 3.0), sin(phase + 2.8)) * radius;
    let center3 = vec2<f32>(cos(phase * 2.0 + 4.5), sin(-phase + 4.0)) * radius;
    let sharpness = params.slots[4].x;
    let weights = vec4<f32>(
        effect_orb_weight(point, center0, sharpness),
        effect_orb_weight(point, center1, sharpness),
        effect_orb_weight(point, center2, sharpness),
        effect_orb_weight(point, center3, sharpness),
    );
    let total = max(dot(weights, vec4<f32>(1.0)), 0.0001);
    let color = (
        params.slots[0] * weights.x +
        params.slots[1] * weights.y +
        params.slots[2] * weights.z +
        params.slots[3] * weights.w
    ) / total;
    let glow = clamp(total * 0.42, 0.0, 1.0);
    let rgb = color.rgb * (0.72 + glow * 0.45);
    return vec4<f32>(rgb, clamp(color.a, 0.0, 1.0));
}
"#;

const ALBUM_GLOW_WGSL: &str = r#"
fn album_glow_weight(point: vec2<f32>, center: vec2<f32>, softness: f32) -> f32 {
    let delta = point - center;
    return exp(-dot(delta, delta) * softness);
}

fn album_glow_color(input: EffectInput, uv: vec2<f32>) -> vec3<f32> {
    return sample_effect_image(input, clamp(uv, vec2<f32>(0.03), vec2<f32>(0.97))).rgb;
}

fn album_drop_wave(
    point: vec2<f32>,
    origin: vec2<f32>,
    aspect: f32,
    progress: f32,
    max_radius: f32,
    width: f32,
) -> vec4<f32> {
    let delta = point - origin;
    let metric_delta = delta * vec2<f32>(aspect, 1.0);
    let distance = max(length(metric_delta), 0.0001);
    let radius = progress * max_radius;
    let signed_distance = (distance - radius) / max(width, 0.0001);
    let life = sin(progress * 3.14159265359);
    let crest = exp(-signed_distance * signed_distance) * life;
    let trough_distance = signed_distance + 1.25;
    let trough = exp(-trough_distance * trough_distance * 1.25) * life;
    let displacement = signed_distance * crest;
    let direction = metric_delta / distance / vec2<f32>(aspect, 1.0);
    let halo = exp(-signed_distance * signed_distance * 0.28) * life;
    return vec4<f32>(direction * displacement, crest - trough * 0.42, halo);
}

fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> {
    let tau = 6.28318530718;
    let phase = input.time * tau;
    let blur = max(params.slots[0].x, 0.0);
    let saturation = max(params.slots[0].y, 0.0);
    let brightness = max(params.slots[0].z, 0.0);
    let motion = max(params.slots[0].w, 0.0);
    let glow_strength = max(params.slots[1].x, 0.0);
    let ripple_displacement = max(params.slots[1].y, 0.0);
    let ripple_light = max(params.slots[1].z, 0.0);
    let ring_definition = clamp(params.slots[1].w, 0.0, 1.0);
    let diffusion = clamp(blur / 0.18, 0.0, 1.0);

    // Pull a compact palette from across the complete artwork. The palette
    // drifts slightly over the image, but the output deliberately does not
    // preserve the cover's contours: each color becomes a very wide light blob.
    let palette_drift = vec2<f32>(cos(phase), sin(phase)) * 0.025 * motion;
    let color0 = album_glow_color(input, vec2<f32>(0.34, 0.24) + palette_drift);
    let color1 = album_glow_color(
        input,
        vec2<f32>(0.84, 0.22) + vec2<f32>(-palette_drift.y, palette_drift.x),
    );
    let color2 = album_glow_color(input, vec2<f32>(0.12, 0.58) - palette_drift);
    let color3 = album_glow_color(
        input,
        vec2<f32>(0.82, 0.55) + vec2<f32>(palette_drift.y, -palette_drift.x),
    );
    let color4 = album_glow_color(input, vec2<f32>(0.25, 0.82) + palette_drift.yx * 0.6);
    let color5 = album_glow_color(input, vec2<f32>(0.72, 0.84) - palette_drift.yx * 0.7);

    // Three staggered drops produce one local wavefront each. Distances are
    // measured in element-height units, preserving a circular wave on a wide card.
    let aspect = input.size.x / max(input.size.y, 1.0);
    let ripple_origin0 = vec2<f32>(0.20, 0.44) +
        vec2<f32>(cos(phase), sin(phase)) * vec2<f32>(0.025, 0.02) * motion;
    let ripple_origin1 = vec2<f32>(0.78, 0.58) +
        vec2<f32>(cos(-phase + 1.8), sin(-phase + 1.8)) * vec2<f32>(0.022, 0.018) * motion;
    let ripple_origin2 = vec2<f32>(0.50, 0.34) +
        vec2<f32>(cos(phase + 3.6), sin(phase + 3.6)) * vec2<f32>(0.024, 0.018) * motion;
    let max_ripple_radius = mix(1.55, 1.2 + diffusion * 0.25, ring_definition);
    let diffuse_width = 0.22 + diffusion * 0.16;
    let defined_width = 0.10 + diffusion * 0.08;
    let ripple_width = mix(diffuse_width, defined_width, ring_definition);
    let ripple0 = album_drop_wave(
        input.uv,
        ripple_origin0,
        aspect,
        fract(input.time),
        max_ripple_radius,
        ripple_width,
    );
    let ripple1 = album_drop_wave(
        input.uv,
        ripple_origin1,
        aspect,
        fract(input.time + 0.3333333),
        max_ripple_radius,
        ripple_width * 1.06,
    );
    let ripple2 = album_drop_wave(
        input.uv,
        ripple_origin2,
        aspect,
        fract(input.time + 0.6666667),
        max_ripple_radius,
        ripple_width * 0.96,
    );
    let point = input.uv + (ripple0.xy + ripple1.xy + ripple2.xy)
        * ripple_displacement
        * motion;
    let center0 = vec2<f32>(-0.04, 0.08) +
        vec2<f32>(cos(phase + 0.2), sin(phase + 0.2)) * vec2<f32>(0.13, 0.09) * motion;
    let center1 = vec2<f32>(0.82, -0.08) +
        vec2<f32>(cos(phase + 1.4), sin(phase + 1.4)) * vec2<f32>(0.12, 0.10) * motion;
    let center2 = vec2<f32>(-0.06, 0.78) +
        vec2<f32>(cos(phase + 2.6), sin(phase + 2.6)) * vec2<f32>(0.14, 0.09) * motion;
    let center3 = vec2<f32>(1.04, 0.62) +
        vec2<f32>(cos(phase + 3.8), sin(phase + 3.8)) * vec2<f32>(0.13, 0.11) * motion;
    let center4 = vec2<f32>(0.38, 0.42) +
        vec2<f32>(cos(-phase + 0.8), sin(-phase + 0.8)) * vec2<f32>(0.17, 0.13) * motion;
    let center5 = vec2<f32>(0.68, 1.04) +
        vec2<f32>(cos(-phase + 2.2), sin(-phase + 2.2)) * vec2<f32>(0.15, 0.10) * motion;

    // A larger diffusion value lowers the falloff, so every output pixel mixes
    // several cover colors instead of showing isolated bands or blobs.
    let softness = mix(5.2, 2.4, diffusion);
    let weight0 = album_glow_weight(point, center0, softness);
    let weight1 = album_glow_weight(point, center1, softness);
    let weight2 = album_glow_weight(point, center2, softness);
    let weight3 = album_glow_weight(point, center3, softness);
    let weight4 = album_glow_weight(point, center4, softness);
    let weight5 = album_glow_weight(point, center5, softness);
    let total = max(weight0 + weight1 + weight2 + weight3 + weight4 + weight5, 0.0001);
    var rgb = (
        color0 * weight0 +
        color1 * weight1 +
        color2 * weight2 +
        color3 * weight3 +
        color4 * weight4 +
        color5 * weight5
    ) / total;

    // Re-evaluate the moving sources with a tighter falloff to create broad
    // emissive halos on top of the already diffused color field.
    let glow_softness = softness * 2.5;
    let glow0 = album_glow_weight(point, center0, glow_softness);
    let glow1 = album_glow_weight(point, center1, glow_softness);
    let glow2 = album_glow_weight(point, center2, glow_softness);
    let glow3 = album_glow_weight(point, center3, glow_softness);
    let glow4 = album_glow_weight(point, center4, glow_softness);
    let glow5 = album_glow_weight(point, center5, glow_softness);
    let glow_total = max(glow0 + glow1 + glow2 + glow3 + glow4 + glow5, 0.0001);
    let glow_color = (
        color0 * glow0 +
        color1 * glow1 +
        color2 * glow2 +
        color3 * glow3 +
        color4 * glow4 +
        color5 * glow5
    ) / glow_total;
    let glow_mask = clamp(glow_total * 0.22, 0.0, 0.5);
    rgb += glow_color * glow_mask * glow_strength;

    // The crest is a single soft ring, while the wider halo gives it the diffuse
    // light of a drop spreading across calm water.
    let ripple_crest = ripple0.z + ripple1.z + ripple2.z;
    let ripple_halo = ripple0.w + ripple1.w + ripple2.w;
    rgb *= 1.0
        + ripple_crest * ripple_light * ring_definition * motion
        + ripple_halo
            * ripple_light
            * (1.0 - ring_definition)
            * 0.14
            * motion;

    let luminance = dot(rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
    rgb = mix(vec3<f32>(luminance), rgb, saturation) * brightness;
    let centered = input.uv - vec2<f32>(0.5);
    let vignette_point = centered * vec2<f32>(0.85, 1.15);
    let vignette = 1.0 - smoothstep(0.18, 0.82, dot(vignette_point, vignette_point));
    let lower_shade = mix(1.0, 0.72, smoothstep(0.35, 1.0, input.uv.y));
    let pulse = 0.97 + 0.03 * sin(phase);
    let light_center = vec2<f32>(0.5) + vec2<f32>(
        cos(phase + 2.0) * 0.36,
        sin(phase + 2.0) * 0.24,
    );
    let light_delta = (input.uv - light_center) * vec2<f32>(1.0, 1.35);
    let moving_light = exp(-dot(light_delta, light_delta) * 5.0);
    rgb *= mix(0.72, 1.0, vignette)
        * lower_shade
        * pulse
        * (0.92 + moving_light * 0.16 * motion);
    return vec4<f32>(max(rgb, vec3<f32>(0.0)), 1.0);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(album_glow_shader().uses_image());
    }
}
