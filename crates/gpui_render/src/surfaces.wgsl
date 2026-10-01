// --- surfaces --- //

struct SurfaceParams {
    bounds: Bounds,
    clip_bounds: Bounds,
    content_mask: Bounds,
    uv_bounds: Bounds,
    corner_radii: Corners,
    color_rows: array<vec4<f32>, 3>,
    opacity: f32,
    _pad: array<f32, 3>,
}

@group(1) @binding(0) var<storage, read> surface_locals: SurfaceParams;
@group(1) @binding(1) var t_surface_0: texture_2d<f32>;
@group(1) @binding(2) var t_surface_1: texture_2d<f32>;
@group(1) @binding(3) var s_surface: sampler;

struct SurfaceVarying {
    @builtin(position) position: vec4<f32>,
    @location(0) texture_position: vec2<f32>,
    @location(3) clip_distances: vec4<f32>,
}

@vertex
fn vs_surface(@builtin(vertex_index) vertex_id: u32) -> SurfaceVarying {
    let unit_vertex = vec2<f32>(f32(vertex_id & 1u), 0.5 * f32(vertex_id & 2u));

    var out = SurfaceVarying();
    out.position = to_device_position(unit_vertex, surface_locals.bounds);
    out.texture_position = surface_locals.uv_bounds.origin + unit_vertex * surface_locals.uv_bounds.size;
    out.clip_distances = distance_from_clip_rect(unit_vertex, surface_locals.bounds, surface_locals.content_mask);
    return out;
}

@fragment
fn fs_surface_rgba(input: SurfaceVarying) -> @location(0) vec4<f32> {
    let sample = textureSample(t_surface_0, s_surface, input.texture_position);
    if (any(input.clip_distances < vec4<f32>(0.0))) {
        return vec4<f32>(0.0);
    }
    let distance = quad_sdf((input.position.xy + globals.viewport_origin), surface_locals.clip_bounds, surface_locals.corner_radii);
    return blend_color(sample, surface_locals.opacity * saturate(0.5 - distance));
}

@fragment
fn fs_surface_yuv(input: SurfaceVarying) -> @location(0) vec4<f32> {
    let yuv = vec4<f32>(
        textureSample(t_surface_0, s_surface, input.texture_position).r,
        textureSample(t_surface_1, s_surface, input.texture_position).rg,
        1.0);
    if (any(input.clip_distances < vec4<f32>(0.0))) {
        return vec4<f32>(0.0);
    }
    let rgb = vec3<f32>(
        dot(surface_locals.color_rows[0], yuv),
        dot(surface_locals.color_rows[1], yuv),
        dot(surface_locals.color_rows[2], yuv),
    );
    let distance = quad_sdf((input.position.xy + globals.viewport_origin), surface_locals.clip_bounds, surface_locals.corner_radii);
    return blend_color(vec4<f32>(rgb, 1.0), surface_locals.opacity * saturate(0.5 - distance));
}
