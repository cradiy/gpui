struct Params {
    region: vec4<u32>,
    jump: u32,
    threshold: f32,
    pad: vec2<u32>,
}

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var seeds: texture_2d<f32>;
@group(0) @binding(2) var output: texture_storage_2d<OUTPUT_FORMAT, write>;
@group(0) @binding(3) var<uniform> params: Params;

fn valid(pixel: vec2<i32>) -> bool {
    return all(pixel >= vec2<i32>(0)) && all(pixel < vec2<i32>(params.region.zw));
}

fn alpha(pixel: vec2<i32>) -> f32 {
    if (!valid(pixel)) { return 0.0; }
    return textureLoad(source, pixel + vec2<i32>(params.region.xy), 0).a;
}

@compute @workgroup_size(8, 8)
fn seed(@builtin(global_invocation_id) id: vec3<u32>) {
    let pixel = vec2<i32>(id.xy);
    if (!valid(pixel)) { return; }
    let position = vec2<f32>(pixel) + 0.5;
    let coverage = alpha(pixel);
    var best = vec4<f32>(0.0);
    var best_distance = 1e20;
    let directions = array<vec2<i32>, 4>(vec2<i32>(1, 0), vec2<i32>(-1, 0), vec2<i32>(0, 1), vec2<i32>(0, -1));
    for (var i = 0u; i < 4u; i += 1u) {
        let neighbor = alpha(pixel + directions[i]);
        if ((coverage >= params.threshold) != (neighbor >= params.threshold)) {
            let t = clamp((params.threshold - coverage) / (neighbor - coverage), 0.0, 1.0);
            if (t * t < best_distance) {
                best = vec4<f32>(position + vec2<f32>(directions[i]) * t, 1.0, 0.0);
                best_distance = t * t;
            }
        }
    }
    textureStore(output, pixel, best);
}

@compute @workgroup_size(8, 8)
fn jump(@builtin(global_invocation_id) id: vec3<u32>) {
    let pixel = vec2<i32>(id.xy);
    if (!valid(pixel)) { return; }
    let position = vec2<f32>(pixel) + 0.5;
    var best = vec4<f32>(0.0);
    var best_distance = 1e20;
    for (var y = -1; y <= 1; y += 1) {
        for (var x = -1; x <= 1; x += 1) {
            let neighbor = pixel + vec2<i32>(x, y) * i32(params.jump);
            if (!valid(neighbor)) { continue; }
            let candidate = textureLoad(seeds, neighbor, 0);
            let delta = candidate.xy - position;
            let distance = dot(delta, delta);
            if (candidate.z > 0.0 && distance < best_distance) {
                best = candidate;
                best_distance = distance;
            }
        }
    }
    textureStore(output, pixel, best);
}

@compute @workgroup_size(8, 8)
fn resolve(@builtin(global_invocation_id) id: vec3<u32>) {
    let pixel = vec2<i32>(id.xy);
    if (!valid(pixel)) { return; }
    let nearest = textureLoad(seeds, pixel, 0);
    let distance = min(length(nearest.xy - (vec2<f32>(pixel) + 0.5)), 65504.0);
    let signed_distance = select(distance, -distance, alpha(pixel) >= params.threshold);
    textureStore(output, pixel, vec4<f32>(select(65504.0, signed_distance, nearest.z > 0.0), nearest.z, 0.0, 1.0));
}
