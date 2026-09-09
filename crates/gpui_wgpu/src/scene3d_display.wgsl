struct Params { settings: vec4<f32>, origin: vec4<f32> };
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
    return store_color(display_sample(textureLoad(hdr, vec2<i32>(position.xy - params.origin.xy), 0)));
}

@fragment
fn fragment_msaa(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    var color = vec4<f32>(0.0);
    let count = textureNumSamples(hdr_msaa);
    for (var i = 0u; i < count; i += 1u) {
        color += display_sample(textureLoad(hdr_msaa, vec2<i32>(position.xy - params.origin.xy), i32(i)));
    }
    return store_color(color / f32(count));
}
