struct Params {
    rays: array<vec4<f32>, 3>,
    bounds: vec4<f32>,
    settings: vec4<f32>,
};
@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var radiance: texture_2d<f32>;
@group(0) @binding(2) var radiance_sampler: sampler;

@vertex
fn vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let p = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    return vec4<f32>(p[index], 1.0, 1.0);
}

@fragment
fn fragment(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let local = (position.xy - params.bounds.xy) / params.bounds.zw;
    if (any(local < vec2<f32>(0.0)) || any(local > vec2<f32>(1.0))) { discard; }
    let ndc = vec2<f32>(local.x * 2.0 - 1.0, 1.0 - local.y * 2.0);
    let direction = normalize(params.rays[0].xyz + ndc.x * params.rays[1].xyz + ndc.y * params.rays[2].xyz);
    let rotated = vec3<f32>(
        params.settings.x * direction.x - params.settings.y * direction.z,
        direction.y,
        params.settings.y * direction.x + params.settings.x * direction.z,
    );
    let uv = vec2<f32>(atan2(rotated.z, rotated.x) / 6.28318530718 + 0.5,
        acos(clamp(rotated.y, -1.0, 1.0)) / 3.14159265359);
    let color = textureSampleLevel(radiance, radiance_sampler, uv, 0.0).rgb * params.settings.z;
    return vec4<f32>(min(color, vec3<f32>(65504.0)), 1.0);
}
