struct ImageParams {
    rect: vec4<f32>, uv_u: vec4<f32>, uv_v: vec4<f32>, sampling: vec4<u32>,
};
struct DirectLight {
    position_kind: vec4<f32>, direction_range: vec4<f32>, color_intensity: vec4<f32>, cone: vec4<f32>,
};
struct Params {
    specular_environment: vec4<f32>,
    model: mat4x4<f32>, normal: mat4x4<f32>, camera: mat4x4<f32>,
    bounds: vec4<f32>, viewport: vec4<f32>, ambient: vec4<f32>,
    color: vec4<f32>, texture_rect: vec4<f32>, flags: vec4<f32>,
    ids: vec4<u32>,
    uv_u: vec4<f32>, uv_v: vec4<f32>, sampling: vec4<u32>,
    view: vec4<f32>, pbr: vec4<f32>, emissive: vec4<f32>,
    metallic_roughness_map: ImageParams, emissive_map: ImageParams,
    normal_map: ImageParams, normal_settings: vec4<f32>,
    depth_plane: vec4<f32>,
    environment_sh: array<vec4<f32>, 9>, environment: vec4<f32>,
    occlusion_map: ImageParams, occlusion_settings: vec4<f32>,
    lights: array<DirectLight, 8>, light_count: vec4<u32>,
    shadow_camera: mat4x4<f32>, shadow_settings: vec4<f32>, shadow_flags: vec4<u32>,
};
@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var image: texture_2d<f32>;
@group(0) @binding(2) var image_sampler: sampler;
@group(0) @binding(3) var metallic_roughness_image: texture_2d<f32>;
@group(0) @binding(4) var emissive_image: texture_2d<f32>;
@group(0) @binding(5) var normal_image: texture_2d<f32>;
@group(0) @binding(6) var occlusion_image: texture_2d<f32>;
@group(0) @binding(7) var shadow_image: texture_depth_2d;
@group(0) @binding(8) var shadow_sampler: sampler_comparison;
@group(0) @binding(9) var specular_image: texture_cube<f32>;
@group(0) @binding(10) var specular_brdf: texture_2d<f32>;
@group(0) @binding(11) var specular_sampler: sampler;
struct Output { @builtin(position) position: vec4<f32>, @location(0) normal: vec3<f32>, @location(1) uv: vec2<f32>, @location(2) world: vec3<f32>, @location(3) tangent: vec4<f32>, @location(4) @interpolate(flat) orientation: f32 };
@vertex
fn vertex(@location(0) position: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) uv: vec2<f32>, @location(3) tangent: vec4<f32>) -> Output {
    var clip = params.camera * params.model * vec4<f32>(position, 1.0);
    let origin = params.bounds.xy / params.viewport.xy;
    let extent = params.bounds.zw / params.viewport.xy;
    clip.x = (origin.x * 2.0 - 1.0) * clip.w + (clip.x + clip.w) * extent.x;
    clip.y = (1.0 - origin.y * 2.0) * clip.w + (clip.y - clip.w) * extent.y;
    let handedness = sign(dot(cross(unit_vector(params.model[0].xyz), unit_vector(params.model[1].xyz)), unit_vector(params.model[2].xyz)));
    let world_tangent = vec4<f32>((params.model * vec4<f32>(tangent.xyz, 0.0)).xyz, tangent.w * handedness);
    return Output(clip, (params.normal * vec4<f32>(normal, 0.0)).xyz, uv, (params.model * vec4<f32>(position, 1.0)).xyz, world_tangent, handedness);
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
fn image_texel(source: texture_2d<f32>, config: ImageParams, pixel: vec2<i32>, extent: vec2<i32>) -> vec4<f32> {
    let addressed = vec2<i32>(address_texel(pixel.x, extent.x, config.sampling.x),
        address_texel(pixel.y, extent.y, config.sampling.y));
    let texel = textureLoad(source, vec2<i32>(config.rect.xy) + addressed, 0);
    if (config.sampling.w == 0u) { return vec4<f32>(srgb_to_linear(texel.rgb), texel.a); }
    return texel;
}
fn sample_image(source: texture_2d<f32>, config: ImageParams, uv: vec2<f32>) -> vec4<f32> {
    let mapped = vec2<f32>(dot(config.uv_u.xyz, vec3<f32>(uv, 1.0)),
        dot(config.uv_v.xyz, vec3<f32>(uv, 1.0)));
    if (!all(abs(mapped) <= vec2<f32>(3.402823466e+38))) { return vec4<f32>(0.0); }
    let addressed = vec2<f32>(address_coordinate(mapped.x, config.sampling.x),
        address_coordinate(mapped.y, config.sampling.y));
    let extent = max(vec2<i32>(config.rect.zw), vec2<i32>(1));
    let pixel = addressed * vec2<f32>(extent) - vec2<f32>(0.5);
    if (config.sampling.z == 0u) {
        return image_texel(source, config, vec2<i32>(floor(pixel + vec2<f32>(0.5))), extent);
    }
    let low = vec2<i32>(floor(pixel));
    let weight = fract(pixel);
    return mix(mix(image_texel(source, config, low, extent), image_texel(source, config, low + vec2<i32>(1, 0), extent), weight.x),
        mix(image_texel(source, config, low + vec2<i32>(0, 1), extent), image_texel(source, config, low + vec2<i32>(1, 1), extent), weight.x), weight.y);
}
fn base_color(input: Output) -> vec4<f32> {
    var sampled: vec4<f32>;
    if (params.flags.w > 0.5) {
        sampled = sample_image(image, ImageParams(params.texture_rect, params.uv_u, params.uv_v, params.sampling), input.uv);
    } else {
        let uv = (params.texture_rect.xy + vec2<f32>(0.5) + clamp(input.uv, vec2<f32>(0.0), vec2<f32>(1.0)) * max(params.texture_rect.zw - 1.0, vec2<f32>(0.0))) / vec2<f32>(textureDimensions(image));
        sampled = textureSampleLevel(image, image_sampler, uv, 0.0);
        if (params.flags.z > 1.5) { sampled = vec4<f32>(linear_to_srgb(sampled.rgb), sampled.a); }
        if (params.flags.z > 0.5) { sampled = vec4<f32>(sampled.rgb / max(sampled.a, 0.00001), sampled.a); }
        sampled = vec4<f32>(srgb_to_linear(sampled.rgb), sampled.a);
    }
    let base = sampled * vec4<f32>(srgb_to_linear(params.color.rgb), params.color.a);
    let alpha = clamp(base.a, 0.0, 1.0);
    if (params.ids.y == 1u && alpha < params.flags.x) { discard; }
    if (params.ids.y == 2u && alpha <= 0.0) { discard; }
    return vec4<f32>(base.rgb, select(1.0, alpha, params.ids.y == 2u));
}
@vertex
fn shadow_vertex(@location(0) position: vec3<f32>, @location(2) uv: vec2<f32>) -> Output {
    let world = params.model * vec4<f32>(position, 1.0);
    return Output(params.shadow_camera * world, vec3<f32>(0.0), uv, world.xyz, vec4<f32>(0.0), 1.0);
}
@fragment
fn shadow_fragment(input: Output) {
    let base = base_color(input);
}

fn shadow_visibility(index: u32, world: vec3<f32>, geometric_normal: vec3<f32>) -> f32 {
    if (params.shadow_flags.x == 0u || index != params.shadow_flags.y) { return 1.0; }
    let light_direction = unit_vector(params.lights[index].direction_range.xyz);
    let n = unit_vector(geometric_normal);
    let slope = 1.0 - clamp(dot(n, light_direction), 0.0, 1.0);
    let clip = params.shadow_camera * vec4<f32>(world + n * params.shadow_settings.y * slope, 1.0);
    let p = clip.xyz / clip.w;
    if (!all(abs(p.xy) <= vec2<f32>(1.0)) || !(p.z >= 0.0 && p.z <= 1.0)) { return 1.0; }
    let uv = p.xy * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5);
    let reference = p.z - params.shadow_settings.x;
    let extent = vec2<f32>(textureDimensions(shadow_image));
    if (params.shadow_settings.z == 0.0) {
        let pixel = clamp(vec2<i32>(uv * extent), vec2<i32>(0), vec2<i32>(extent) - vec2<i32>(1));
        return select(0.0, 1.0, reference <= textureLoad(shadow_image, pixel, 0));
    }
    var visibility = 0.0;
    for (var y = -1; y <= 1; y += 1) {
        for (var x = -1; x <= 1; x += 1) {
            let sample_uv = uv + vec2<f32>(f32(x), f32(y)) * params.shadow_settings.z / extent;
            if (any(sample_uv < vec2<f32>(0.0)) || any(sample_uv > vec2<f32>(1.0))) {
                visibility += 1.0;
            } else {
                visibility += textureSampleCompareLevel(shadow_image, shadow_sampler, sample_uv, reference);
            }
        }
    }
    return visibility / 9.0;
}
@fragment
fn object_id(input: Output) -> @location(0) u32 {
    let base = base_color(input);
    return params.ids.x;
}

@fragment
fn linear_depth(input: Output) -> @location(0) f32 {
    let base = base_color(input);
    return dot(params.depth_plane, vec4<f32>(input.world, 1.0));
}

@fragment
fn world_normal(input: Output, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let base = base_color(input);
    let normal = unit_vector(input.normal) * select(-1.0, 1.0, front) * input.orientation;
    return vec4<f32>(normal, 1.0);
}

fn unit_vector(value: vec3<f32>) -> vec3<f32> {
    let magnitude = max(max(abs(value.x), abs(value.y)), abs(value.z));
    if (magnitude == 0.0 || !(magnitude <= 3.402823466e+38)) { return vec3<f32>(0.0); }
    let scaled = value / magnitude;
    return scaled / length(scaled);
}

fn surface_normal(input: Output) -> vec3<f32> {
    let n = unit_vector(input.normal);
    if (params.normal_settings.y < 0.5) { return n; }
    let t0 = unit_vector(input.tangent.xyz);
    let t = unit_vector(t0 - n * dot(n, t0));
    if (dot(t, t) < 0.5 || abs(input.tangent.w) < 0.5) { return n; }
    let b = cross(n, t) * sign(input.tangent.w);
    let decoded = sample_image(normal_image, params.normal_map, input.uv).xyz * 2.0 - 1.0;
    let mapped = unit_vector(decoded * vec3<f32>(params.normal_settings.x, params.normal_settings.x, 1.0));
    if (dot(mapped, mapped) < 0.5) { return n; }
    return unit_vector(t * mapped.x + b * mapped.y + n * mapped.z);
}

fn diffuse_environment(normal: vec3<f32>) -> vec3<f32> {
    if (params.environment.z == 0.0) { return vec3<f32>(0.0); }
    let n = unit_vector(normal);
    let x = params.environment.x * n.x - params.environment.y * n.z;
    let y = n.y;
    let z = params.environment.y * n.x + params.environment.x * n.z;
    let basis = array<f32, 9>(0.2820947918, 0.4886025119 * y, 0.4886025119 * z, 0.4886025119 * x,
        1.0925484306 * x * y, 1.0925484306 * y * z, 0.3153915653 * (3.0 * z * z - 1.0),
        1.0925484306 * x * z, 0.5462742153 * (x * x - y * y));
    var value = vec3<f32>(0.0);
    for (var i = 0u; i < 9u; i += 1u) { value += params.environment_sh[i].rgb * basis[i]; }
    return max(value, vec3<f32>(0.0)) * params.environment.z;
}

fn occlusion(uv: vec2<f32>) -> f32 {
    if (params.occlusion_settings.x == 0.0) { return 1.0; }
    return mix(1.0, sample_image(occlusion_image, params.occlusion_map, uv).r, params.occlusion_settings.x);
}

struct LightSample { direction: vec3<f32>, energy: vec3<f32> };
fn sample_light(source: DirectLight, world: vec3<f32>) -> LightSample {
    var direction = unit_vector(source.direction_range.xyz);
    var attenuation = 1.0;
    if (source.position_kind.w > 0.5) {
        let delta = source.position_kind.xyz - world;
        let distance = length(delta);
        direction = unit_vector(delta);
        let clamped_distance = max(distance, source.cone.z);
        attenuation = 1.0 / (clamped_distance * clamped_distance);
        if (source.direction_range.w > 0.0) {
            let ratio = min(distance / source.direction_range.w, 1.0);
            let window = 1.0 - ratio * ratio * ratio * ratio;
            attenuation *= window * window;
        }
        if (source.position_kind.w > 1.5) {
            let cosine = dot(-direction, unit_vector(source.direction_range.xyz));
            let angular = clamp((cosine - source.cone.y) / (source.cone.x - source.cone.y), 0.0, 1.0);
            attenuation *= angular * angular;
        }
    }
    return LightSample(direction, srgb_to_linear(source.color_intensity.rgb) * source.color_intensity.a * attenuation);
}

fn pbr_direct(diffuse: vec3<f32>, f0: vec3<f32>, roughness: f32, normal: vec3<f32>, view: vec3<f32>, light: LightSample) -> vec3<f32> {
    let nv = clamp(dot(normal, view), 0.0, 1.0);
    let nl = clamp(dot(normal, light.direction), 0.0, 1.0);
    if (nv <= 0.0 || nl <= 0.0) { return vec3<f32>(0.0); }
    let half_vector = unit_vector(view + light.direction);
    let nh = clamp(dot(normal, half_vector), 0.0, 1.0);
    let vh = clamp(dot(view, half_vector), 0.0, 1.0);
    let a = roughness * roughness;
    let a2 = a * a;
    let d = (1.0 - nh * nh) + a2 * nh * nh;
    let distribution = a2 / max(3.14159265359 * d * d, 1e-12);
    let visibility = 0.5 / max(nl * sqrt(nv * nv * (1.0 - a2) + a2)
        + nv * sqrt(nl * nl * (1.0 - a2) + a2), 1e-6);
    let fresnel = f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - vh, 5.0);
    let specular = fresnel * distribution * visibility;
    let reflected = (vec3<f32>(1.0) - fresnel) * diffuse / 3.14159265359 + specular;
    return reflected * light.energy * nl;
}

fn pbr_lighting(base: vec3<f32>, normal: vec3<f32>, geometric_normal: vec3<f32>, world: vec3<f32>, uv: vec2<f32>) -> vec3<f32> {
    let factors = sample_image(metallic_roughness_image, params.metallic_roughness_map, uv);
    let metal = params.pbr.x * factors.b;
    let roughness = max(params.pbr.y * factors.g, 0.045);
    let emission = params.emissive.rgb * sample_image(emissive_image, params.emissive_map, uv).rgb;
    let view = unit_vector(params.view.xyz - world * params.view.w);
    let diffuse = base * (1.0 - metal);
    let f0 = mix(vec3<f32>(0.04), base, metal);
    var result = diffuse * (vec3<f32>(params.ambient.x) + (vec3<f32>(1.0) - f0) * diffuse_environment(normal)) * occlusion(uv) + emission;
    let nv = clamp(dot(normal, view), 0.0, 1.0);
    if (params.specular_environment.z > 0.0 && nv > 0.0) {
        let direction = reflect(-view, normal);
        let settings = params.specular_environment;
        let rotated = vec3<f32>(settings.x * direction.x - settings.y * direction.z, direction.y,
            settings.y * direction.x + settings.x * direction.z);
        let radiance = textureSampleLevel(specular_image, specular_sampler, rotated, roughness * settings.w).rgb;
        let brdf = textureSampleLevel(specular_brdf, specular_sampler, vec2<f32>(nv, roughness), 0.0).rg;
        result += radiance * settings.z * (f0 * brdf.x + vec3<f32>(brdf.y)) * occlusion(uv);
    }
    for (var i = 0u; i < params.light_count.x; i += 1u) {
        result += pbr_direct(diffuse, f0, roughness, normal, view, sample_light(params.lights[i], world)) * shadow_visibility(i, world, geometric_normal);
    }
    return result;
}

@fragment
fn fragment(input: Output, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let base = base_color(input);
    var illumination = vec3<f32>(1.0);
    if (params.flags.y < 0.5) {
        if (params.pbr.z > 0.5) {
            let normal = surface_normal(input) * select(-1.0, 1.0, front) * input.orientation;
            let geometric_normal = unit_vector(input.normal) * select(-1.0, 1.0, front) * input.orientation;
            return vec4<f32>(clamp(pbr_lighting(base.rgb, normal, geometric_normal, input.world, input.uv), vec3<f32>(0.0), vec3<f32>(65504.0)) * base.a, base.a);
        }
        let normal = input.normal / max(length(input.normal), 0.00001) * select(-1.0, 1.0, front) * input.orientation;
        illumination = (vec3<f32>(params.ambient.x) + diffuse_environment(normal)) * occlusion(input.uv);
        for (var i = 0u; i < params.light_count.x; i += 1u) {
            let light = sample_light(params.lights[i], input.world);
            illumination += light.energy * max(dot(normal, light.direction), 0.0) * shadow_visibility(i, input.world, normal);
        }
    }
    return vec4<f32>(clamp(base.rgb * illumination, vec3<f32>(0.0), vec3<f32>(65504.0)) * base.a, base.a);
}
