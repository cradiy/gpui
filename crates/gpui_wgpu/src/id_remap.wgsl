@group(0) @binding(0) var source: texture_2d<u32>;
@group(0) @binding(1) var<storage, read> labels: array<u32>;

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let positions = array<vec2<f32>, 3>(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    return vec4(positions[index], 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) u32 {
    let id = textureLoad(source, vec2<i32>(position.xy), 0).x;
    if id == 0u || id > arrayLength(&labels) {
        return 0u;
    }
    return labels[id - 1u];
}
