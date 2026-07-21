struct GlobalParams {
    viewport_size: vec2<f32>,
    premultiplied_alpha: u32,
    pad: u32,
}

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

struct TransformationMatrix {
    rotation_scale: mat2x2<f32>,
    translation: vec2<f32>,
}

struct EffectParams {
    slots: array<vec4<f32>, 8>,
}

struct EffectInput {
    uv: vec2<f32>,
    position: vec2<f32>,
    size: vec2<f32>,
    time: f32,
    image_origin: vec2<f32>,
    image_size: vec2<f32>,
    second_image_origin: vec2<f32>,
    second_image_size: vec2<f32>,
    third_image_origin: vec2<f32>,
    third_image_size: vec2<f32>,
    fourth_image_origin: vec2<f32>,
    fourth_image_size: vec2<f32>,
    mask_uv: vec2<f32>,
}

@group(0) @binding(0) var<uniform> globals: GlobalParams;

// __GPUI_EFFECT_IMAGE_SOURCE__
// __GPUI_EFFECT_SECOND_IMAGE_SOURCE__
// __GPUI_EFFECT_ADDITIONAL_IMAGE_SOURCE__
// __GPUI_EFFECT_MASK_SOURCE__

fn effect_to_device_position(
    unit_vertex: vec2<f32>,
    bounds: Bounds,
    transform: TransformationMatrix,
) -> vec4<f32> {
    let position = unit_vertex * bounds.size + bounds.origin;
    let transformed = transpose(transform.rotation_scale) * position + transform.translation;
    let device_position = transformed / globals.viewport_size * vec2<f32>(2.0, -2.0)
        + vec2<f32>(-1.0, 1.0);
    return vec4<f32>(device_position, 0.0, 1.0);
}

fn effect_clip_distances(
    unit_vertex: vec2<f32>,
    bounds: Bounds,
    clip: Bounds,
    transform: TransformationMatrix,
) -> vec4<f32> {
    let position = unit_vertex * bounds.size + bounds.origin;
    let transformed = transpose(transform.rotation_scale) * position + transform.translation;
    let top_left = transformed - clip.origin;
    let bottom_right = clip.origin + clip.size - transformed;
    return vec4<f32>(top_left.x, bottom_right.x, top_left.y, bottom_right.y);
}

fn effect_corner_radius(point: vec2<f32>, radii: Corners) -> f32 {
    if (point.x < 0.0) {
        return select(radii.bottom_left, radii.top_left, point.y < 0.0);
    }
    return select(radii.bottom_right, radii.top_right, point.y < 0.0);
}

fn effect_quad_sdf(point: vec2<f32>, bounds: Bounds, radii: Corners) -> f32 {
    let half_size = bounds.size * 0.5;
    let center_to_point = point - (bounds.origin + half_size);
    let radius = effect_corner_radius(center_to_point, radii);
    let corner = abs(center_to_point) - half_size + radius;
    return length(max(corner, vec2<f32>(0.0)))
        + min(max(corner.x, corner.y), 0.0)
        - radius;
}

fn effect_blend_color(color: vec4<f32>, alpha_factor: f32) -> vec4<f32> {
    let alpha = color.a * alpha_factor;
    let multiplier = select(1.0, alpha, globals.premultiplied_alpha != 0u);
    return vec4<f32>(color.rgb * multiplier, alpha);
}

// __GPUI_EFFECT_SOURCE__

struct EffectInstance {
    bounds: Bounds,
    effect_bounds: Bounds,
    transformation: TransformationMatrix,
    content_mask: Bounds,
    corner_radii: Corners,
    image_bounds: Bounds,
    second_image_bounds: Bounds,
    third_image_bounds: Bounds,
    fourth_image_bounds: Bounds,
    opacity: f32,
    time: f32,
    pad: vec2<f32>,
    alignment_pad: vec2<f32>,
    uniforms: EffectParams,
}

@group(1) @binding(0) var<storage, read> b_effects: array<EffectInstance>;

struct EffectVarying {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) effect_id: u32,
    @location(2) clip_distances: vec4<f32>,
    @location(3) local_position: vec2<f32>,
}

@vertex
fn vs_effect(
    @builtin(vertex_index) vertex_id: u32,
    @builtin(instance_index) instance_id: u32,
) -> EffectVarying {
    let unit_vertex = vec2<f32>(f32(vertex_id & 1u), 0.5 * f32(vertex_id & 2u));
    let instance = b_effects[instance_id];
    var out: EffectVarying;
    out.position = effect_to_device_position(unit_vertex, instance.bounds, instance.transformation);
    out.uv = unit_vertex;
    out.local_position = unit_vertex * instance.bounds.size + instance.bounds.origin;
    out.effect_id = instance_id;
    out.clip_distances = effect_clip_distances(
        unit_vertex,
        instance.bounds,
        instance.content_mask,
        instance.transformation,
    );
    return out;
}

@fragment
fn fs_effect(input: EffectVarying) -> @location(0) vec4<f32> {
    if (any(input.clip_distances < vec4<f32>(0.0))) {
        return vec4<f32>(0.0);
    }

    let instance = b_effects[input.effect_id];
    let effect_uv = (input.local_position - instance.effect_bounds.origin)
        / max(instance.effect_bounds.size, vec2<f32>(0.0001));
    let effect_input = EffectInput(
        effect_uv,
        input.position.xy,
        instance.effect_bounds.size,
        instance.time,
        instance.image_bounds.origin,
        instance.image_bounds.size,
        instance.second_image_bounds.origin,
        instance.second_image_bounds.size,
        instance.third_image_bounds.origin,
        instance.third_image_bounds.size,
        instance.fourth_image_bounds.origin,
        instance.fourth_image_bounds.size,
        input.uv,
    );
    let raw_color = effect(effect_input, instance.uniforms);
    let color = vec4<f32>(raw_color.rgb, clamp(raw_color.a, 0.0, 1.0));

    let distance = effect_quad_sdf(input.position.xy, instance.bounds, instance.corner_radii);
    let coverage = 1.0 - smoothstep(-0.5, 0.5, distance);
    let mask_coverage = effect_mask_coverage(effect_input);
    return effect_blend_color(color, coverage * mask_coverage * instance.opacity);
}
