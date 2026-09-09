struct Params { settings: vec4<f32>, output_rect: vec4<f32> };
@group(0) @binding(0) var hdr: texture_2d<f32>;
@group(0) @binding(1) var<uniform> params: Params;
@group(0) @binding(2) var hdr_msaa: texture_multisampled_2d<f32>;

@vertex
fn vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let positions = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    return vec4<f32>(positions[index], 0.0, 1.0);
}

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    return select(1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055, 12.92 * c, c <= vec3<f32>(0.0031308));
}
fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    return select(pow((c + 0.055) / 1.055, vec3<f32>(2.4)), c / 12.92, c <= vec3<f32>(0.04045));
}

fn display_sample(sample: vec4<f32>) -> vec4<f32> {
    if (sample.a <= 0.0) { return vec4<f32>(0.0); }
    var color = max(sample.rgb / sample.a, vec3<f32>(0.0)) * params.settings.x;
    if (params.settings.y > 0.5) { color = color / (vec3<f32>(1.0) + color); }
    let encoded = linear_to_srgb(clamp(color, vec3<f32>(0.0), vec3<f32>(1.0))) * sample.a;
    return vec4<f32>(encoded, sample.a);
}

fn store_color(color: vec4<f32>) -> vec4<f32> {
    // sRGB attachments encode on store; preserve display-encoded premultiplication.
    if (params.settings.z > 0.5) { return vec4<f32>(srgb_to_linear(color.rgb), color.a); }
    return color;
}

@fragment
fn fragment(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let extent = vec2<f32>(textureDimensions(hdr));
    let local = position.xy - params.output_rect.xy;
    if (all(extent == params.output_rect.zw)) {
        return store_color(display_sample(textureLoad(hdr, vec2<i32>(local), 0)));
    }
    let pixel = clamp(local * extent / params.output_rect.zw - 0.5, vec2<f32>(0.0), extent - 1.0);
    let low = vec2<i32>(floor(pixel));
    let high = min(low + 1, vec2<i32>(extent) - 1);
    let weight = fract(pixel);
    let top = mix(display_sample(textureLoad(hdr, low, 0)), display_sample(textureLoad(hdr, vec2<i32>(high.x, low.y), 0)), weight.x);
    let bottom = mix(display_sample(textureLoad(hdr, vec2<i32>(low.x, high.y), 0)), display_sample(textureLoad(hdr, high, 0)), weight.x);
    return store_color(mix(top, bottom, weight.y));
}

fn resolve_pixel(pixel: vec2<i32>) -> vec4<f32> {
    var color = vec4<f32>(0.0);
    let count = textureNumSamples(hdr_msaa);
    for (var i = 0u; i < count; i += 1u) {
        color += display_sample(textureLoad(hdr_msaa, pixel, i32(i)));
    }
    return color / f32(count);
}

@fragment
fn fragment_msaa(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let extent = vec2<f32>(textureDimensions(hdr_msaa));
    let local = position.xy - params.output_rect.xy;
    if (all(extent == params.output_rect.zw)) {
        return store_color(resolve_pixel(vec2<i32>(local)));
    }
    let pixel = clamp(local * extent / params.output_rect.zw - 0.5, vec2<f32>(0.0), extent - 1.0);
    let low = vec2<i32>(floor(pixel));
    let high = min(low + 1, vec2<i32>(extent) - 1);
    let weight = fract(pixel);
    let top = mix(resolve_pixel(low), resolve_pixel(vec2<i32>(high.x, low.y)), weight.x);
    let bottom = mix(resolve_pixel(vec2<i32>(low.x, high.y)), resolve_pixel(high), weight.x);
    return store_color(mix(top, bottom, weight.y));
}
