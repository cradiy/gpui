struct Record { words: array<vec4<u32>, 4> }
struct Params { vertices: u32, padding_0: u32, padding_1: u32, padding_2: u32 }
@group(0) @binding(0) var<storage, read> source: array<Record>;
@group(0) @binding(1) var<storage, read> mapping: array<u32>;
@group(0) @binding(2) var<storage, read_write> output: array<Record>;
@group(0) @binding(3) var<uniform> params: Params;

@compute @workgroup_size(64)
fn remap(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= params.vertices { return; }
    output[id.x] = source[mapping[id.x]];
}
