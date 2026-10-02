/* Functions useful for debugging:

// A heat map color for debugging (blue -> cyan -> green -> yellow -> red).
fn heat_map_color(value: f32, minValue: f32, maxValue: f32, position: vec2<f32>) -> vec4<f32> {
    // Normalize value to 0-1 range
    let t = clamp((value - minValue) / (maxValue - minValue), 0.0, 1.0);

    // Heat map color calculation
    let r = t * t;
    let g = 4.0 * t * (1.0 - t);
    let b = (1.0 - t) * (1.0 - t);
    let heat_color = vec3<f32>(r, g, b);

    // Create a checkerboard pattern (black and white)
    let sum = floor(position.x / 3) + floor(position.y / 3);
    let is_odd = fract(sum * 0.5); // 0.0 for even, 0.5 for odd
    let checker_value = is_odd * 2.0; // 0.0 for even, 1.0 for odd
    let checker_color = vec3<f32>(checker_value);

    // Determine if value is in range (1.0 if in range, 0.0 if out of range)
    let in_range = step(minValue, value) * step(value, maxValue);

    // Mix checkerboard and heat map based on whether value is in range
    let final_color = mix(checker_color, heat_color, in_range);

    return vec4<f32>(final_color, 1.0);
}

*/

// Contrast and gamma correction adapted from https://github.com/microsoft/terminal/blob/1283c0f5b99a2961673249fa77c6b986efb5086c/src/renderer/atlas/dwrite.hlsl
// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.
fn color_brightness(color: vec3<f32>) -> f32 {
    // REC. 601 luminance coefficients for perceived brightness
    return dot(color, vec3<f32>(0.30, 0.59, 0.11));
}

fn light_on_dark_contrast(enhancedContrast: f32, color: vec3<f32>) -> f32 {
    let brightness = color_brightness(color);
    let multiplier = saturate(4.0 * (0.75 - brightness));
    return enhancedContrast * multiplier;
}

fn enhance_contrast(alpha: f32, k: f32) -> f32 {
    return alpha * (k + 1.0) / (alpha * k + 1.0);
}

fn enhance_contrast3(alpha: vec3<f32>, k: f32) -> vec3<f32> {
    return alpha * (k + 1.0) / (alpha * k + 1.0);
}

fn apply_alpha_correction(a: f32, b: f32, g: vec4<f32>) -> f32 {
    let brightness_adjustment = g.x * b + g.y;
    let correction = brightness_adjustment * a + (g.z * b + g.w);
    return a + a * (1.0 - a) * correction;
}

fn apply_alpha_correction3(a: vec3<f32>, b: vec3<f32>, g: vec4<f32>) -> vec3<f32> {
    let brightness_adjustment = g.x * b + g.y;
    let correction = brightness_adjustment * a + (g.z * b + g.w);
    return a + a * (1.0 - a) * correction;
}

fn apply_contrast_and_gamma_correction(sample: f32, color: vec3<f32>, enhanced_contrast_factor: f32, gamma_ratios: vec4<f32>) -> f32 {
    let enhanced_contrast = light_on_dark_contrast(enhanced_contrast_factor, color);
    let brightness = color_brightness(color);

    let contrasted = enhance_contrast(sample, enhanced_contrast);
    return apply_alpha_correction(contrasted, brightness, gamma_ratios);
}

fn apply_contrast_and_gamma_correction3(sample: vec3<f32>, color: vec3<f32>, enhanced_contrast_factor: f32, gamma_ratios: vec4<f32>) -> vec3<f32> {
    let enhanced_contrast = light_on_dark_contrast(enhanced_contrast_factor, color);

    let contrasted = enhance_contrast3(sample, enhanced_contrast);
    return apply_alpha_correction3(contrasted, color, gamma_ratios);
}

// Native targets blend in sRGB; WGPU retains its existing linear-color path.
const NATIVE_SRGB: bool = false;

struct GlobalParams {
    viewport_size: vec2<f32>,
    premultiplied_alpha: u32,
    pad: u32,
    viewport_origin: vec2<f32>,
    origin_pad: vec2<u32>,
}

struct GammaParams {
    gamma_ratios: vec4<f32>,
    grayscale_enhanced_contrast: f32,
    subpixel_enhanced_contrast: f32,
    is_bgr: u32,
    pad: u32,
}

@group(0) @binding(0) var<uniform> globals: GlobalParams;
@group(0) @binding(1) var<uniform> gamma_params: GammaParams;
@group(1) @binding(1) var t_sprite: texture_2d<f32>;
@group(1) @binding(2) var s_sprite: sampler;

const M_PI_F: f32 = 3.1415926;
const GRAYSCALE_FACTORS: vec3<f32> = vec3<f32>(0.2126, 0.7152, 0.0722);

struct Bounds {
    origin: vec2<f32>,
    size: vec2<f32>,
}

struct Corners {
    top_left: f32,
    top_right: f32,
    bottom_right: f32,
    bottom_left: f32,
}

struct Edges {
    top: f32,
    right: f32,
    bottom: f32,
    left: f32,
}

struct Hsla {
    h: f32,
    s: f32,
    l: f32,
    a: f32,
}

struct BorderColors {
    top: Hsla,
    right: Hsla,
    bottom: Hsla,
    left: Hsla,
}

struct BorderColorStop {
    color: Hsla,
    position: f32,
}

struct BorderGradient {
    stops: array<BorderColorStop, 2>,
    stop_count: u32,
    color_space: u32,
    phase: f32,
    opacity: f32,
    stop_offset: u32,
    pad: u32,
}

struct LinearColorStop {
    color: Hsla,
    percentage: f32,
}

struct ExtendedColorStop {
    color: Hsla,
    position: f32,
    midpoint: f32,
}
@group(2) @binding(0) var<storage, read> gradient_stops: array<ExtendedColorStop>;

struct Background {
    // 0u is Solid
    // 1u is LinearGradient
    // 2u is PatternSlash
    // 3u is Checkerboard
    tag: u32,
    // 0u is sRGB linear color
    // 1u is Oklab color
    color_space: u32,
    solid: Hsla,
    gradient_angle_or_pattern_height: f32,
    colors: array<LinearColorStop, 2>,
    stop_count: u32,
    gradient_phase: f32,
    gradient_repeating: u32,
    gradient_midpoints: array<f32, 2>,
    angular_seam_width: f32,
    stop_offset: u32,
}

struct AtlasTextureId {
    index: u32,
    kind: u32,
}

struct AtlasBounds {
    origin: vec2<i32>,
    size: vec2<i32>,
}

struct AtlasTile {
    texture_id: AtlasTextureId,
    tile_id: u32,
    padding: u32,
    bounds: AtlasBounds,
}

struct TransformationMatrix {
    rotation_scale: mat2x2<f32>,
    translation: vec2<f32>,
}

fn to_device_position_impl(position: vec2<f32>) -> vec4<f32> {
    let device_position = (position - globals.viewport_origin) / globals.viewport_size * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0);
    return vec4<f32>(device_position, 0.0, 1.0);
}

fn to_device_position(unit_vertex: vec2<f32>, bounds: Bounds) -> vec4<f32> {
    let position = unit_vertex * vec2<f32>(bounds.size) + bounds.origin;
    return to_device_position_impl(position);
}

fn to_device_position_transformed(unit_vertex: vec2<f32>, bounds: Bounds, transform: TransformationMatrix) -> vec4<f32> {
    let position = unit_vertex * vec2<f32>(bounds.size) + bounds.origin;
    //Note: Rust side stores it as row-major, so transposing here
    let transformed = transpose(transform.rotation_scale) * position + transform.translation;
    return to_device_position_impl(transformed);
}

fn to_tile_position(unit_vertex: vec2<f32>, tile: AtlasTile) -> vec2<f32> {
  let atlas_size = vec2<f32>(textureDimensions(t_sprite, 0));
  return (vec2<f32>(tile.bounds.origin) + unit_vertex * vec2<f32>(tile.bounds.size)) / atlas_size;
}

fn distance_from_clip_rect_impl(position: vec2<f32>, clip_bounds: Bounds) -> vec4<f32> {
    let tl = position - clip_bounds.origin;
    let br = clip_bounds.origin + clip_bounds.size - position;
    return vec4<f32>(tl.x, br.x, tl.y, br.y);
}

fn distance_from_clip_rect(unit_vertex: vec2<f32>, bounds: Bounds, clip_bounds: Bounds) -> vec4<f32> {
    let position = unit_vertex * vec2<f32>(bounds.size) + bounds.origin;
    return distance_from_clip_rect_impl(position, clip_bounds);
}

fn distance_from_clip_rect_transformed(unit_vertex: vec2<f32>, bounds: Bounds, clip_bounds: Bounds, transform: TransformationMatrix) -> vec4<f32> {
    let position = unit_vertex * vec2<f32>(bounds.size) + bounds.origin;
    let transformed = transpose(transform.rotation_scale) * position + transform.translation;
    return distance_from_clip_rect_impl(transformed, clip_bounds);
}

// https://gamedev.stackexchange.com/questions/92015/optimized-linear-to-srgb-glsl
fn srgb_to_linear(srgb: vec3<f32>) -> vec3<f32> {
    let cutoff = srgb < vec3<f32>(0.04045);
    let higher = pow((srgb + vec3<f32>(0.055)) / vec3<f32>(1.055), vec3<f32>(2.4));
    let lower = srgb / vec3<f32>(12.92);
    return select(higher, lower, cutoff);
}

fn srgb_to_linear_component(a: f32) -> f32 {
    let cutoff = a < 0.04045;
    let higher = pow((a + 0.055) / 1.055, 2.4);
    let lower = a / 12.92;
    return select(higher, lower, cutoff);
}

fn linear_to_srgb(linear: vec3<f32>) -> vec3<f32> {
    let cutoff = linear < vec3<f32>(0.0031308);
    let higher = vec3<f32>(1.055) * pow(linear, vec3<f32>(1.0 / 2.4)) - vec3<f32>(0.055);
    let lower = linear * vec3<f32>(12.92);
    return select(higher, lower, cutoff);
}

/// Convert a linear color to sRGBA space.
fn linear_to_srgba(color: vec4<f32>) -> vec4<f32> {
    if NATIVE_SRGB { return color; }
    return vec4<f32>(linear_to_srgb(color.rgb), color.a);
}

/// Convert a sRGBA color to linear space.
fn srgba_to_linear(color: vec4<f32>) -> vec4<f32> {
    if NATIVE_SRGB { return color; }
    return vec4<f32>(srgb_to_linear(color.rgb), color.a);
}

/// Hsla to linear RGBA conversion.
fn hsla_to_rgba(hsla: Hsla) -> vec4<f32> {
    let h = hsla.h * 6.0; // Now, it's an angle but scaled in [0, 6) range
    let s = hsla.s;
    let l = hsla.l;
    let a = hsla.a;

    let c = (1.0 - abs(2.0 * l - 1.0)) * s;
    let x = c * (1.0 - abs(h % 2.0 - 1.0));
    let m = l - c / 2.0;
    var color = vec3<f32>(m);

    if (h >= 0.0 && h < 1.0) {
        color.r += c;
        color.g += x;
    } else if (h >= 1.0 && h < 2.0) {
        color.r += x;
        color.g += c;
    } else if (h >= 2.0 && h < 3.0) {
        color.g += c;
        color.b += x;
    } else if (h >= 3.0 && h < 4.0) {
        color.g += x;
        color.b += c;
    } else if (h >= 4.0 && h < 5.0) {
        color.r += x;
        color.b += c;
    } else {
        color.r += c;
        color.b += x;
    }

    return vec4<f32>(color, a);
}

/// Convert a linear sRGB to Oklab space.
/// Reference: https://bottosson.github.io/posts/oklab/#converting-from-linear-srgb-to-oklab
fn linear_srgb_to_oklab(input_color: vec4<f32>) -> vec4<f32> {
    var color = input_color;
    if NATIVE_SRGB { color = vec4<f32>(srgb_to_linear(color.rgb), color.a); }
	let l = 0.4122214708 * color.r + 0.5363325363 * color.g + 0.0514459929 * color.b;
	let m = 0.2119034982 * color.r + 0.6806995451 * color.g + 0.1073969566 * color.b;
	let s = 0.0883024619 * color.r + 0.2817188376 * color.g + 0.6299787005 * color.b;

	let l_ = pow(l, 1.0 / 3.0);
	let m_ = pow(m, 1.0 / 3.0);
	let s_ = pow(s, 1.0 / 3.0);

	return vec4<f32>(
		0.2104542553 * l_ + 0.7936177850 * m_ - 0.0040720468 * s_,
		1.9779984951 * l_ - 2.4285922050 * m_ + 0.4505937099 * s_,
		0.0259040371 * l_ + 0.7827717662 * m_ - 0.8086757660 * s_,
		color.a
	);
}

/// Convert an Oklab color to linear sRGB space.
fn oklab_to_linear_srgb(color: vec4<f32>) -> vec4<f32> {
	let l_ = color.r + 0.3963377774 * color.g + 0.2158037573 * color.b;
	let m_ = color.r - 0.1055613458 * color.g - 0.0638541728 * color.b;
	let s_ = color.r - 0.0894841775 * color.g - 1.2914855480 * color.b;

	let l = l_ * l_ * l_;
	let m = m_ * m_ * m_;
	let s = s_ * s_ * s_;

	let output = vec4<f32>(
		4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
		-1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
		-0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s,
		color.a
	);
    if NATIVE_SRGB { return vec4<f32>(linear_to_srgb(output.rgb), output.a); }
    return output;
}

fn over(below: vec4<f32>, above: vec4<f32>) -> vec4<f32> {
    let alpha = above.a + below.a * (1.0 - above.a);
    let color = (above.rgb * above.a + below.rgb * below.a * (1.0 - above.a)) / alpha;
    return vec4<f32>(color, alpha);
}

// A standard gaussian function, used for weighting samples
fn gaussian(x: f32, sigma: f32) -> f32{
    return exp(-(x * x) / (2.0 * sigma * sigma)) / (sqrt(2.0 * M_PI_F) * sigma);
}

// This approximates the error function, needed for the gaussian integral
fn erf(v: vec2<f32>) -> vec2<f32> {
    let s = sign(v);
    let a = abs(v);
    let r1 = 1.0 + (0.278393 + (0.230389 + (0.000972 + 0.078108 * a) * a) * a) * a;
    let r2 = r1 * r1;
    return s - s / (r2 * r2);
}

fn blur_along_x(x: f32, y: f32, sigma: f32, corner: f32, half_size: vec2<f32>) -> f32 {
  let delta = min(half_size.y - corner - abs(y), 0.0);
  let curved = half_size.x - corner + sqrt(max(0.0, corner * corner - delta * delta));
  let integral = 0.5 + 0.5 * erf((x + vec2<f32>(-curved, curved)) * (sqrt(0.5) / sigma));
  return integral.y - integral.x;
}

// Selects corner radius based on quadrant.
fn pick_corner_radius(center_to_point: vec2<f32>, radii: Corners) -> f32 {
    if (center_to_point.x < 0.0) {
        if (center_to_point.y < 0.0) {
            return radii.top_left;
        } else {
            return radii.bottom_left;
        }
    } else {
        if (center_to_point.y < 0.0) {
            return radii.top_right;
        } else {
            return radii.bottom_right;
        }
    }
}

// Signed distance of the point to the quad's border - positive outside the
// border, and negative inside.
//
// See comments on similar code using `quad_sdf_impl` in `fs_quad` for
// explanation.
fn quad_sdf(point: vec2<f32>, bounds: Bounds, corner_radii: Corners) -> f32 {
    let half_size = bounds.size / 2.0;
    let center = bounds.origin + half_size;
    let center_to_point = point - center;
    let corner_radius = pick_corner_radius(center_to_point, corner_radii);
    let corner_to_point = abs(center_to_point) - half_size;
    let corner_center_to_point = corner_to_point + corner_radius;
    return quad_sdf_impl(corner_center_to_point, corner_radius);
}

fn quad_sdf_impl(corner_center_to_point: vec2<f32>, corner_radius: f32) -> f32 {
    if (corner_radius == 0.0) {
        // Fast path for unrounded corners.
        return max(corner_center_to_point.x, corner_center_to_point.y);
    } else {
        // Signed distance of the point from a quad that is inset by corner_radius.
        // It is negative inside this quad, and positive outside.
        let signed_distance_to_inset_quad =
            // 0 inside the inset quad, and positive outside.
            length(max(vec2<f32>(0.0), corner_center_to_point)) +
            // 0 outside the inset quad, and negative inside.
            min(0.0, max(corner_center_to_point.x, corner_center_to_point.y));

        return signed_distance_to_inset_quad - corner_radius;
    }
}

// Abstract away the final color transformation based on the
// target alpha compositing mode.
fn blend_color(color: vec4<f32>, alpha_factor: f32) -> vec4<f32> {
    let alpha = color.a * alpha_factor;
    let multiplier = select(1.0, alpha, globals.premultiplied_alpha != 0u);
    return vec4<f32>(color.rgb * multiplier, alpha);
}


struct GradientColor {
    solid: vec4<f32>,
    colors: array<vec4<f32>, 2>,
}

fn prepare_gradient_color(background: Background) -> GradientColor {
    var result = GradientColor();
    if (background.stop_offset != 0u) { return result; }
    let tag = background.tag;
    let color_space = background.color_space;
    let solid = background.solid;
    let colors = background.colors;

    if (tag == 0u || tag == 2u || tag == 3u) {
        result.solid = hsla_to_rgba(solid);
    } else if (tag == 1u || (tag >= 4u && tag <= 6u)) {
        for (var ix = 0u; ix < 2u; ix += 1u) {
            // hsla_to_rgba returns a linear sRGB color.
            let color = hsla_to_rgba(colors[ix].color);
            if (color_space == 0u) {
                result.colors[ix] = linear_to_srgba(color);
            } else if (color_space == 1u) {
                result.colors[ix] = linear_srgb_to_oklab(color);
            }
        }
    }

    return result;
}

fn sample_gradient_stops(
    background: Background,
    position: f32,
    colors: array<vec4<f32>, 2>,
) -> vec4<f32> {
    let count = max(background.stop_count, 2u);
    var sample_position = clamp(position, 0.0, 1.0);
    var left_ix = 0u;
    var right_ix = 0u;
    var left_position = gradient_stop_position(background, 0u);
    var right_position = left_position;

    if (background.gradient_repeating != 0u) {
        sample_position = fract(position + background.gradient_phase);
        left_ix = count - 1u;
        right_ix = 0u;
        left_position = gradient_stop_position(background, left_ix);
        right_position = gradient_stop_position(background, 0) + 1.0;

        if (sample_position >= gradient_stop_position(background, 0u)
            && sample_position < left_position) {
            left_ix = gradient_segment_index(background, sample_position, count);
            right_ix = left_ix + 1u;
            left_position = gradient_stop_position(background, left_ix);
            right_position = gradient_stop_position(background, right_ix);
        }
        if (right_ix == 0u && sample_position < gradient_stop_position(background, 0)) {
            sample_position += 1.0;
        }
    } else if (sample_position <= gradient_stop_position(background, 0)) {
        left_ix = 0u;
        right_ix = 0u;
    } else if (sample_position >= gradient_stop_position(background, count - 1u)) {
        left_ix = count - 1u;
        right_ix = count - 1u;
        left_position = gradient_stop_position(background, left_ix);
        right_position = left_position;
    } else {
        left_ix = gradient_segment_index(background, sample_position, count);
        right_ix = left_ix + 1u;
        left_position = gradient_stop_position(background, left_ix);
        right_position = gradient_stop_position(background, right_ix);
    }

    let segment = max(right_position - left_position, 0.000001);
    let t = clamp((sample_position - left_position) / segment, 0.0, 1.0);
    // CSS color-hint interpolation: the segment midpoint is a 50% mix.
    var midpoint = background.gradient_midpoints[min(left_ix, 1u)];
    if (background.stop_offset != 0u) {
        midpoint = gradient_stops[background.stop_offset - 1u + left_ix].midpoint;
    }
    var weight = t;
    if (midpoint != 0.5 && t > 0.0 && t < 1.0) {
        weight = pow(t, log(0.5) / log(midpoint));
    }
    var color = mix(gradient_stop_color(background, colors, left_ix), gradient_stop_color(background, colors, right_ix), weight);
    if (background.gradient_repeating != 0u) {
        var before_ix = count - 1u;
        if (left_ix > 0u) {
            before_ix = left_ix - 1u;
        }
        var after_ix = 0u;
        if (right_ix + 1u < count) {
            after_ix = right_ix + 1u;
        }

        // Treat the stops as periodic cubic B-spline control points. Four
        // neighboring colors influence every sample, avoiding visible bands
        // centered on exact stop colors while keeping the curve C2-continuous.
        let t2 = t * t;
        let t3 = t2 * t;
        let one_minus_t = 1.0 - t;
        let weight0 = one_minus_t * one_minus_t * one_minus_t / 6.0;
        let weight1 = (3.0 * t3 - 6.0 * t2 + 4.0) / 6.0;
        let weight2 = (-3.0 * t3 + 3.0 * t2 + 3.0 * t + 1.0) / 6.0;
        let weight3 = t3 / 6.0;
        color = weight0 * gradient_stop_color(background, colors, before_ix)
            + weight1 * gradient_stop_color(background, colors, left_ix)
            + weight2 * gradient_stop_color(background, colors, right_ix)
            + weight3 * gradient_stop_color(background, colors, after_ix);
    }
    return color;
}

// Upper-bound search preserves equal-position hard edges without a linear scan.
fn gradient_segment_index(background: Background, position: f32, count: u32) -> u32 {
    var low = 0u;
    var high = count - 1u;
    while (low + 1u < high) {
        let middle = low + (high - low) / 2u;
        if (position < gradient_stop_position(background, middle)) {
            high = middle;
        } else {
            low = middle;
        }
    }
    return low;
}

fn gradient_stop_position(background: Background, index: u32) -> f32 {
    if (background.stop_offset != 0u) {
        return gradient_stops[background.stop_offset - 1u + index].position;
    }
    return background.colors[index].percentage;
}

fn gradient_stop_color(background: Background, colors: array<vec4<f32>, 2>, index: u32) -> vec4<f32> {
    if (background.stop_offset == 0u) { return colors[index]; }
    var color = hsla_to_rgba(gradient_stops[background.stop_offset - 1u + index].color);
    color.a *= background.solid.a;
    if (background.color_space == 1u) { return linear_srgb_to_oklab(color); }
    return linear_to_srgba(color);
}

fn sample_linear_gradient(background: Background, position: f32,
    colors: array<vec4<f32>, 2>) -> vec4<f32> {
    var color = sample_gradient_stops(background, position, colors);
    let width = background.angular_seam_width;
    let half_width = width * 0.5;
    if (background.tag == 5u && background.gradient_repeating == 0u && width > 0.0
        && (position < half_width || position > 1.0 - half_width)) {
        let seam_start = sample_gradient_stops(background, 1.0 - half_width, colors);
        let seam_end = sample_gradient_stops(background, half_width, colors);
        let wrapped = select(position - 1.0, position, position < half_width);
        color = mix(seam_start, seam_end, smoothstep(-half_width, half_width, wrapped));
    }
    if (background.color_space == 1u) {
        return oklab_to_linear_srgb(color);
    }
    return srgba_to_linear(color);
}

fn gradient_color(background: Background, position: vec2<f32>, bounds: Bounds,
    solid_color: vec4<f32>, colors: array<vec4<f32>, 2>) -> vec4<f32> {
    var background_color = vec4<f32>(0.0);

    switch (background.tag) {
        default: {
            return solid_color;
        }
        case 1u, 4u, 5u, 6u: {
            // Linear gradient background.
            // -90 degrees to match the CSS gradient angle.
            let angle = background.gradient_angle_or_pattern_height;
            let radians = (angle % 360.0 - 90.0) * M_PI_F / 180.0;
            var direction = vec2<f32>(cos(radians), sin(radians));
            // Expand the short side to be the same as the long side
            if (bounds.size.x > bounds.size.y) {
                direction.y *= bounds.size.y / bounds.size.x;
            } else {
                direction.x *= bounds.size.x / bounds.size.y;
            }

            // Get the t value for the linear gradient with the color stop percentages.
            let half_size = bounds.size / 2.0;
            let center = bounds.origin + half_size;
            let center_to_point = position - center;
            var t = dot(center_to_point, direction) / length(direction);
            // Check the direct to determine the use x or y
            if (abs(direction.x) > abs(direction.y)) {
                t = (t + half_size.x) / bounds.size.x;
            } else {
                t = (t + half_size.y) / bounds.size.y;
            }

            if (background.tag != 1u) {
                let q = (position - center) / max(half_size, vec2<f32>(0.0001));
                let c = cos(radians);
                let s = sin(radians);
                let r = vec2<f32>(q.x * c + q.y * s, -q.x * s + q.y * c);
                if (background.tag == 4u) {
                    t = length(q);
                } else if (background.tag == 5u) {
                    t = fract((atan2(q.y, q.x) - radians) / (2.0 * M_PI_F) + 1.0);
                } else {
                    t = abs(r.x) + abs(r.y);
                }
            }
            background_color = sample_linear_gradient(background, t, colors);

            // Dither to reduce visible 8-bit color banding on large gradients.
            // Two decorrelated samples form triangular-distributed noise.
            let seed = position * 0.6180339887;
            let r1 = fract(sin(dot(seed, vec2<f32>(12.9898, 78.233))) * 43758.5453);
            let r2 = fract(sin(dot(seed, vec2<f32>(39.3460, 11.135))) * 24634.6345);
            let triangular_noise = r1 + r2 - 1.0;
            background_color = vec4<f32>(
                background_color.rgb + vec3<f32>(triangular_noise * 2.0 / 255.0),
                background_color.a + triangular_noise * 3.0 / 255.0,
            );
        }
        case 2u: {
            // pattern slash
            let gradient_angle_or_pattern_height = background.gradient_angle_or_pattern_height;
            let pattern_width = (gradient_angle_or_pattern_height / 65535.0f) / 255.0f;
            let pattern_interval = (gradient_angle_or_pattern_height % 65535.0f) / 255.0f;
            let pattern_height = pattern_width + pattern_interval;
            let stripe_angle = M_PI_F / 4.0;
            let pattern_period = pattern_height * sin(stripe_angle);
            let rotation = mat2x2<f32>(
                cos(stripe_angle), -sin(stripe_angle),
                sin(stripe_angle), cos(stripe_angle)
            );
            let relative_position = position - bounds.origin;
            let rotated_point = rotation * relative_position;
            let pattern = rotated_point.x % pattern_period;
            let distance = min(pattern, pattern_period - pattern) - pattern_period * (pattern_width / pattern_height) /  2.0f;
            background_color = solid_color;
            background_color.a *= saturate(0.5 - distance);
        }
        case 3u: {
            // checkerboard
            let size = background.gradient_angle_or_pattern_height;
            let relative_position = position - bounds.origin;

            let x_index = floor(relative_position.x / size);
            let y_index = floor(relative_position.y / size);
            let should_be_colored = (x_index + y_index) % 2.0;

            background_color = solid_color;
            background_color.a *= saturate(should_be_colored);
        }
    }

    return background_color;
}
