struct Params {
    model: mat4x4<f32>, normal: mat4x4<f32>, camera: mat4x4<f32>,
    bounds: vec4<f32>, viewport: vec4<f32>, direction: vec4<f32>, light: vec4<f32>,
    color: vec4<f32>, texture_rect: vec4<f32>, flags: vec4<f32>,
    ids: vec4<u32>,
    uv_u: vec4<f32>, uv_v: vec4<f32>, sampling: vec4<u32>,
};
@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var image: texture_2d<f32>;
@group(0) @binding(2) var image_sampler: sampler;
struct Output { @builtin(position) position: vec4<f32>, @location(0) normal: vec3<f32>, @location(1) uv: vec2<f32> };
@vertex
fn vertex(@location(0) position: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) uv: vec2<f32>) -> Output {
    var clip = params.camera * params.model * vec4<f32>(position, 1.0);
    let origin = params.bounds.xy / params.viewport.xy;
    let extent = params.bounds.zw / params.viewport.xy;
    clip.x = (origin.x * 2.0 - 1.0) * clip.w + (clip.x + clip.w) * extent.x;
    clip.y = (1.0 - origin.y * 2.0) * clip.w + (clip.y - clip.w) * extent.y;
    return Output(clip, (params.normal * vec4<f32>(normal, 0.0)).xyz, uv);
}
fn address_coordinate(value: f32, mode: u32) -> f32 {
    if (mode == 1u) { return value - floor(value); }
    if (mode == 2u) {
        let period = value - floor(value * 0.5) * 2.0;
        return min(period, 2.0 - period);
    }
    return clamp(value, 0.0, 1.0);
}
fn address_texel(value: i32, extent: i32, mode: u32) -> i32 {
    if (mode == 1u) { return ((value % extent) + extent) % extent; }
    return clamp(value, 0, extent - 1);
}
fn image_texel(pixel: vec2<i32>, extent: vec2<i32>) -> vec4<f32> {
    let addressed = vec2<i32>(address_texel(pixel.x, extent.x, params.sampling.x),
        address_texel(pixel.y, extent.y, params.sampling.y));
    return textureLoad(image, vec2<i32>(params.texture_rect.xy) + addressed, 0);
}
fn sample_image(uv: vec2<f32>) -> vec4<f32> {
    let mapped = vec2<f32>(dot(params.uv_u.xyz, vec3<f32>(uv, 1.0)),
        dot(params.uv_v.xyz, vec3<f32>(uv, 1.0)));
    if (!all(abs(mapped) <= vec2<f32>(3.402823466e+38))) { return vec4<f32>(0.0); }
    let addressed = vec2<f32>(address_coordinate(mapped.x, params.sampling.x),
        address_coordinate(mapped.y, params.sampling.y));
    let extent = max(vec2<i32>(params.texture_rect.zw), vec2<i32>(1));
    let pixel = addressed * vec2<f32>(extent) - vec2<f32>(0.5);
    if (params.sampling.z == 0u) {
        return image_texel(vec2<i32>(floor(pixel + vec2<f32>(0.5))), extent);
    }
    let low = vec2<i32>(floor(pixel));
    let weight = fract(pixel);
    return mix(mix(image_texel(low, extent), image_texel(low + vec2<i32>(1, 0), extent), weight.x),
        mix(image_texel(low + vec2<i32>(0, 1), extent), image_texel(low + vec2<i32>(1, 1), extent), weight.x), weight.y);
}
fn base_color(input: Output) -> vec4<f32> {
    var sampled: vec4<f32>;
    if (params.flags.w > 0.5) {
        sampled = sample_image(input.uv);
    } else {
        let uv = (params.texture_rect.xy + vec2<f32>(0.5) + clamp(input.uv, vec2<f32>(0.0), vec2<f32>(1.0)) * max(params.texture_rect.zw - 1.0, vec2<f32>(0.0))) / vec2<f32>(textureDimensions(image));
        sampled = textureSampleLevel(image, image_sampler, uv, 0.0);
    }
    if (params.flags.z > 0.5) { sampled = vec4<f32>(sampled.rgb / max(sampled.a, 0.00001), sampled.a); }
    let base = sampled * params.color;
    if (base.a < params.flags.x) { discard; }
    return base;
}
@fragment
fn object_id(input: Output) -> @location(0) u32 {
    let base = base_color(input);
    return params.ids.x;
}
@fragment
fn fragment(input: Output, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let base = base_color(input);
    var illumination = vec3<f32>(1.0);
    if (params.flags.y < 0.5) {
        let normal = input.normal / max(length(input.normal), 0.00001) * select(-1.0, 1.0, front);
        let light = params.direction.xyz / max(length(params.direction.xyz), 0.00001);
        illumination = vec3<f32>(params.direction.w) + params.light.rgb * params.light.a * max(dot(normal, light), 0.0);
    }
    return vec4<f32>(base.rgb * illumination, 1.0);
}
