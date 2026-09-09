struct Params {
    model: mat4x4<f32>, normal: mat4x4<f32>, camera: mat4x4<f32>,
    bounds: vec4<f32>, viewport: vec4<f32>, direction: vec4<f32>, light: vec4<f32>,
    color: vec4<f32>, texture_rect: vec4<f32>, flags: vec4<f32>,
    ids: vec4<u32>,
    uv_u: vec4<f32>, uv_v: vec4<f32>, sampling: vec4<u32>,
    view: vec4<f32>, pbr: vec4<f32>, emissive: vec4<f32>,
};
@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var image: texture_2d<f32>;
@group(0) @binding(2) var image_sampler: sampler;
struct Output { @builtin(position) position: vec4<f32>, @location(0) normal: vec3<f32>, @location(1) uv: vec2<f32>, @location(2) world: vec3<f32> };
@vertex
fn vertex(@location(0) position: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) uv: vec2<f32>) -> Output {
    var clip = params.camera * params.model * vec4<f32>(position, 1.0);
    let origin = params.bounds.xy / params.viewport.xy;
    let extent = params.bounds.zw / params.viewport.xy;
    clip.x = (origin.x * 2.0 - 1.0) * clip.w + (clip.x + clip.w) * extent.x;
    clip.y = (1.0 - origin.y * 2.0) * clip.w + (clip.y - clip.w) * extent.y;
    return Output(clip, (params.normal * vec4<f32>(normal, 0.0)).xyz, uv, (params.model * vec4<f32>(position, 1.0)).xyz);
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
fn srgb_to_linear(value: vec3<f32>) -> vec3<f32> {
    let c = clamp(value, vec3<f32>(0.0), vec3<f32>(1.0));
    return select(pow((c + 0.055) / 1.055, vec3<f32>(2.4)), c / 12.92, c <= vec3<f32>(0.04045));
}
fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    return select(1.055 * pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055, 12.92 * c, c <= vec3<f32>(0.0031308));
}
fn image_texel(pixel: vec2<i32>, extent: vec2<i32>) -> vec4<f32> {
    let addressed = vec2<i32>(address_texel(pixel.x, extent.x, params.sampling.x),
        address_texel(pixel.y, extent.y, params.sampling.y));
    let texel = textureLoad(image, vec2<i32>(params.texture_rect.xy) + addressed, 0);
    if (params.sampling.w == 0u) { return vec4<f32>(srgb_to_linear(texel.rgb), texel.a); }
    return texel;
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
        if (params.flags.z > 1.5) { sampled = vec4<f32>(linear_to_srgb(sampled.rgb), sampled.a); }
        if (params.flags.z > 0.5) { sampled = vec4<f32>(sampled.rgb / max(sampled.a, 0.00001), sampled.a); }
        sampled = vec4<f32>(srgb_to_linear(sampled.rgb), sampled.a);
    }
    let base = sampled * vec4<f32>(srgb_to_linear(params.color.rgb), params.color.a);
    if (base.a < params.flags.x) { discard; }
    return base;
}
@fragment
fn object_id(input: Output) -> @location(0) u32 {
    let base = base_color(input);
    return params.ids.x;
}

fn unit_vector(value: vec3<f32>) -> vec3<f32> {
    let scaled = value / max(max(max(abs(value.x), abs(value.y)), abs(value.z)), 0.000001);
    return scaled / max(length(scaled), 0.000001);
}

fn pbr_lighting(base: vec3<f32>, normal: vec3<f32>, world: vec3<f32>) -> vec3<f32> {
    let metal = params.pbr.x;
    let roughness = max(params.pbr.y, 0.045);
    let view = unit_vector(params.view.xyz - world * params.view.w);
    let light = unit_vector(params.direction.xyz);
    let nv = clamp(dot(normal, view), 0.0, 1.0);
    let nl = clamp(dot(normal, light), 0.0, 1.0);
    let diffuse = base * (1.0 - metal);
    var result = diffuse * params.direction.w + params.emissive.rgb;
    if (nv <= 0.0 || nl <= 0.0) { return result; }
    let half_vector = unit_vector(view + light);
    let nh = clamp(dot(normal, half_vector), 0.0, 1.0);
    let vh = clamp(dot(view, half_vector), 0.0, 1.0);
    let a = roughness * roughness;
    let a2 = a * a;
    let d = (1.0 - nh * nh) + a2 * nh * nh;
    let distribution = a2 / max(3.14159265359 * d * d, 1e-12);
    let visibility = 0.5 / max(nl * sqrt(nv * nv * (1.0 - a2) + a2)
        + nv * sqrt(nl * nl * (1.0 - a2) + a2), 1e-6);
    let f0 = mix(vec3<f32>(0.04), base, metal);
    let fresnel = f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - vh, 5.0);
    let specular = fresnel * distribution * visibility;
    let reflected = (vec3<f32>(1.0) - fresnel) * diffuse / 3.14159265359 + specular;
    result += reflected * srgb_to_linear(params.light.rgb) * params.light.a * nl;
    return result;
}

@fragment
fn fragment(input: Output, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let base = base_color(input);
    var illumination = vec3<f32>(1.0);
    if (params.flags.y < 0.5) {
        if (params.pbr.z > 0.5) {
            let normal = unit_vector(input.normal) * select(-1.0, 1.0, front);
            return vec4<f32>(clamp(pbr_lighting(base.rgb, normal, input.world), vec3<f32>(0.0), vec3<f32>(65504.0)), 1.0);
        }
        let normal = input.normal / max(length(input.normal), 0.00001) * select(-1.0, 1.0, front);
        let light = params.direction.xyz / max(length(params.direction.xyz), 0.00001);
        illumination = vec3<f32>(params.direction.w) + srgb_to_linear(params.light.rgb) * params.light.a * max(dot(normal, light), 0.0);
    }
    return vec4<f32>(clamp(base.rgb * illumination, vec3<f32>(0.0), vec3<f32>(65504.0)), 1.0);
}
