struct Params { rect: vec4<u32>, output: vec4<u32> };
@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var<uniform> params: Params;

@vertex
fn vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let positions = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    return vec4<f32>(positions[index], 0.0, 1.0);
}

fn decode(value: vec4<f32>) -> vec4<f32> {
    if (params.output.z == 0u) { return value; }
    let c = value.rgb;
    return vec4<f32>(select(pow((c + 0.055) / 1.055, vec3<f32>(2.4)), c / 12.92, c <= vec3<f32>(0.04045)), value.a);
}

@fragment
fn fragment(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let ratio = vec2<f32>(params.rect.zw) / vec2<f32>(params.output.xy);
    let low = floor(position.xy) * ratio;
    let high = (floor(position.xy) + vec2<f32>(1.0)) * ratio;
    var result = vec4<f32>(0.0);
    for (var y = u32(floor(low.y)); y < u32(ceil(high.y)); y += 1u) {
        for (var x = u32(floor(low.x)); x < u32(ceil(high.x)); x += 1u) {
            let overlap = max(vec2<f32>(0.0), min(high, vec2<f32>(f32(x + 1u), f32(y + 1u))) - max(low, vec2<f32>(f32(x), f32(y))));
            let pixel = params.rect.xy + min(vec2<u32>(x, y), params.rect.zw - vec2<u32>(1u));
            result += decode(textureLoad(source, vec2<i32>(pixel), 0)) * overlap.x * overlap.y;
        }
    }
    return result / (ratio.x * ratio.y);
}
