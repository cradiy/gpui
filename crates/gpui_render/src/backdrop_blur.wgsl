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
@group(1) @binding(1) var t_backdrop: texture_2d<f32>;

struct BackdropVarying {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) instance_id: u32,
}

fn unit_vertex(vertex_id: u32) -> vec2<f32> {
    return vec2<f32>(f32(vertex_id & 1u), 0.5 * f32(vertex_id & 2u));
}

fn to_device_position(unit: vec2<f32>, bounds: Bounds) -> vec4<f32> {
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
    var out: BackdropVarying;
    out.position = to_device_position(unit_vertex(vertex_id), b_backdrops[instance_id].bounds);
    out.instance_id = instance_id;
    return out;
}

@fragment
fn fs_blur(input: BackdropVarying) -> @location(0) vec4<f32> {
    let instance = b_backdrops[input.instance_id];
    let uv = input.position.xy / max(instance.content_mask.size, vec2<f32>(1.0));
    let step = instance.direction * instance.blur_radius
        / max(globals.viewport_size, vec2<f32>(1.0)) * 0.25;

    var color = sample_backdrop(uv) * 0.227027;
    color += sample_backdrop(uv + step * 1.0) * 0.1945946;
    color += sample_backdrop(uv - step * 1.0) * 0.1945946;
    color += sample_backdrop(uv + step * 2.0) * 0.1216216;
    color += sample_backdrop(uv - step * 2.0) * 0.1216216;
    color += sample_backdrop(uv + step * 3.0) * 0.054054;
    color += sample_backdrop(uv - step * 3.0) * 0.054054;
    color += sample_backdrop(uv + step * 4.0) * 0.016216;
    color += sample_backdrop(uv - step * 4.0) * 0.016216;
    return color;
}

fn corner_radius(point: vec2<f32>, radii: vec4<f32>) -> f32 {
    if (point.x < 0.0) {
        return select(radii.w, radii.x, point.y < 0.0);
    }
    return select(radii.z, radii.y, point.y < 0.0);
}

fn rounded_rect_distance(position: vec2<f32>, bounds: Bounds, radii: vec4<f32>) -> f32 {
    let half_size = bounds.size * 0.5;
    let centered = position - (bounds.origin + half_size);
    let radius = corner_radius(centered, radii);
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

    let distance = rounded_rect_distance(position, instance.bounds, instance.corner_radii);
    let coverage = 1.0 - smoothstep(-0.5, 0.5, distance);
    let factor = coverage * instance.opacity;
    let sampled = sample_backdrop(
        position / max(globals.viewport_size, vec2<f32>(1.0)),
    );

    if (globals.premultiplied_alpha != 0u) {
        return vec4<f32>(sampled.rgb * factor, sampled.a * factor);
    }
    return vec4<f32>(sampled.rgb, sampled.a * factor);
}
