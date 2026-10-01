struct GlobalParams {
    viewport_size: vec2<f32>,
    premultiplied_alpha: u32,
    pad: u32,
}

struct Bounds {
    origin: vec2<f32>,
    size: vec2<f32>,
}

struct BackdropParams {
    slots: array<vec4<f32>, 8>,
}

struct BackdropInput {
    uv: vec2<f32>,
    position: vec2<f32>,
    size: vec2<f32>,
    time: f32,
    pointer: vec2<f32>,
    pointer_active: f32,
}

struct BackdropInstance {
    bounds: Bounds,
    content_mask: Bounds,
    corner_radii: vec4<f32>,
    blur_radius: f32,
    opacity: f32,
    time: f32,
    pointer_active: f32,
    direction: vec2<f32>,
    pointer: vec2<f32>,
    uniforms: BackdropParams,
}

@group(0) @binding(0) var<uniform> globals: GlobalParams;
@group(1) @binding(0) var<storage, read> b_backdrops: array<BackdropInstance>;
@group(1) @binding(1) var t_raw_backdrop: texture_2d<f32>;
@group(1) @binding(3) var t_blurred_backdrop: texture_2d<f32>;

// __GPUI_BACKDROP_SAMPLING__

fn backdrop_straight_color(color: vec4<f32>) -> vec4<f32> {
    if (globals.premultiplied_alpha != 0u && color.a > 0.00001) {
        return vec4<f32>(color.rgb / color.a, color.a);
    }
    return color;
}

fn backdrop_sample_uv(input: BackdropInput, displacement_pixels: vec2<f32>) -> vec2<f32> {
    return clamp(
        (input.position + displacement_pixels) / max(globals.viewport_size, vec2<f32>(1.0)),
        vec2<f32>(0.0),
        vec2<f32>(1.0),
    );
}

fn sample_raw_backdrop(input: BackdropInput, displacement_pixels: vec2<f32>) -> vec4<f32> {
    return backdrop_straight_color(backdrop_sample_texture(
        t_raw_backdrop,
        backdrop_sample_uv(input, displacement_pixels),
    ));
}

fn sample_blurred_backdrop(input: BackdropInput, displacement_pixels: vec2<f32>) -> vec4<f32> {
    return backdrop_straight_color(backdrop_sample_texture(
        t_blurred_backdrop,
        backdrop_sample_uv(input, displacement_pixels),
    ));
}

// __GPUI_BACKDROP_SOURCE__

struct BackdropVarying {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) instance_id: u32,
}

fn backdrop_device_position(unit: vec2<f32>, bounds: Bounds) -> vec4<f32> {
    let position = unit * bounds.size + bounds.origin;
    let device = position / globals.viewport_size * vec2<f32>(2.0, -2.0)
        + vec2<f32>(-1.0, 1.0);
    return vec4<f32>(device, 0.0, 1.0);
}

@vertex
fn vs_backdrop(
    @builtin(vertex_index) vertex_id: u32,
    @builtin(instance_index) instance_id: u32,
) -> BackdropVarying {
    let unit = vec2<f32>(f32(vertex_id & 1u), 0.5 * f32(vertex_id & 2u));
    var out: BackdropVarying;
    out.position = backdrop_device_position(unit, b_backdrops[instance_id].bounds);
    out.uv = unit;
    out.instance_id = instance_id;
    return out;
}

fn backdrop_corner_radius(point: vec2<f32>, radii: vec4<f32>) -> f32 {
    if (point.x < 0.0) {
        return select(radii.w, radii.x, point.y < 0.0);
    }
    return select(radii.z, radii.y, point.y < 0.0);
}

fn backdrop_rounded_rect_distance(position: vec2<f32>, bounds: Bounds, radii: vec4<f32>) -> f32 {
    let half_size = bounds.size * 0.5;
    let centered = position - (bounds.origin + half_size);
    let radius = backdrop_corner_radius(centered, radii);
    let corner = abs(centered) - half_size + radius;
    return length(max(corner, vec2<f32>(0.0)))
        + min(max(corner.x, corner.y), 0.0)
        - radius;
}

@fragment
fn fs_backdrop(input: BackdropVarying) -> @location(0) vec4<f32> {
    let instance = b_backdrops[input.instance_id];
    let position = input.position.xy;
    let mask_end = instance.content_mask.origin + instance.content_mask.size;
    if (any(position < instance.content_mask.origin) || any(position > mask_end)) {
        discard;
    }

    let effect_input = BackdropInput(
        input.uv,
        position,
        instance.bounds.size,
        instance.time,
        instance.pointer,
        instance.pointer_active,
    );
    let raw = backdrop_effect(effect_input, instance.uniforms);
    let color = vec4<f32>(raw.rgb, clamp(raw.a, 0.0, 1.0));
    let distance = backdrop_rounded_rect_distance(
        position,
        instance.bounds,
        instance.corner_radii,
    );
    let coverage = 1.0 - smoothstep(-0.5, 0.5, distance);
    let alpha = color.a * coverage * instance.opacity;
    let multiplier = select(1.0, alpha, globals.premultiplied_alpha != 0u);
    return vec4<f32>(color.rgb * multiplier, alpha);
}
