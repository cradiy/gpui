struct MaskParams {
    bounds: vec4<f32>,
    region: vec4<u32>,
    settings: vec4<f32>,
}
struct MaskSample {
    position: vec4<f32>,
    color: vec4<f32>,
}
struct MaskSamples {
    count: atomic<u32>,
    pad0: u32,
    pad1: u32,
    pad2: u32,
    samples: array<MaskSample>,
}
@group(0) @binding(0) var<uniform> params: MaskParams;
@group(0) @binding(1) var source: texture_2d<f32>;
@group(0) @binding(2) var<storage, read_write> candidates: MaskSamples;

fn source_alpha(position: vec2<i32>) -> f32 {
    let low = vec2<i32>(params.region.xy);
    let high = low + vec2<i32>(params.region.zw);
    if (any(position < low) || any(position >= high)) { return 0.0; }
    return textureLoad(source, position, 0).a;
}

@compute @workgroup_size(8, 8)
fn sample_mask(@builtin(global_invocation_id) id: vec3<u32>) {
    let local = id.xy * u32(params.bounds.w);
    if (any(local >= params.region.zw)) { return; }
    let position = vec2<i32>(params.region.xy + local);
    let color = textureLoad(source, position, 0);
    if (color.a < params.settings.x) { return; }
    if (params.settings.y > 0.0) {
        let offsets = array<vec2<f32>, 8>(
            vec2<f32>(1.0, 0.0), vec2<f32>(-1.0, 0.0),
            vec2<f32>(0.0, 1.0), vec2<f32>(0.0, -1.0),
            vec2<f32>(0.707, 0.707), vec2<f32>(-0.707, 0.707),
            vec2<f32>(0.707, -0.707), vec2<f32>(-0.707, -0.707));
        var edge = false;
        for (var i = 0u; i < 8u; i += 1u) {
            let offset = vec2<i32>(round(offsets[i] * max(params.settings.y, 1.0)));
            edge = edge || source_alpha(position + offset) < params.settings.x;
        }
        if (!edge) { return; }
    }
    let index = atomicAdd(&candidates.count, 1u);
    if (index < arrayLength(&candidates.samples)) {
        candidates.samples[index] = MaskSample(
            vec4<f32>((vec2<f32>(position) + 0.5 - params.bounds.xy) / params.bounds.z, 0.0, 0.0),
            vec4<f32>(color.rgb / max(color.a, 0.001), color.a));
    }
}
