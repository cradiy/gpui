struct Params {
    model: mat4x4<f32>, normal: mat4x4<f32>, camera: mat4x4<f32>,
    bounds: vec4<f32>, viewport: vec4<f32>, direction: vec4<f32>, light: vec4<f32>,
    color: vec4<f32>, texture_rect: vec4<f32>, flags: vec4<f32>,
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
@fragment
fn fragment(input: Output, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let uv = (params.texture_rect.xy + vec2<f32>(0.5) + clamp(input.uv, vec2<f32>(0.0), vec2<f32>(1.0)) * max(params.texture_rect.zw - 1.0, vec2<f32>(0.0))) / vec2<f32>(textureDimensions(image));
    var sampled = textureSample(image, image_sampler, uv);
    if (params.flags.z > 0.5) { sampled = vec4<f32>(sampled.rgb / max(sampled.a, 0.00001), sampled.a); }
    let base = sampled * params.color;
    if (base.a < params.flags.x) { discard; }
    var illumination = vec3<f32>(1.0);
    if (params.flags.y < 0.5) {
        let normal = input.normal / max(length(input.normal), 0.00001) * select(-1.0, 1.0, front);
        let light = params.direction.xyz / max(length(params.direction.xyz), 0.00001);
        illumination = vec3<f32>(params.direction.w) + params.light.rgb * params.light.a * max(dot(normal, light), 0.0);
    }
    return vec4<f32>(base.rgb * illumination, 1.0);
}
