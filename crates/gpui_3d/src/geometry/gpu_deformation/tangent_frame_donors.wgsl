struct Frame {
    tangent: vec4<f32>, bitangent: vec4<f32>,
    identity: vec3<u32>, weight: f32, status: vec4<u32>,
}
struct Record { a: vec4<u32>, b: vec4<u32>, c: vec4<u32>, status: vec4<u32> }
struct Params { corners: u32, distance: u32, width: u32, capacity: u32 }
@group(0) @binding(0) var<storage, read> source: array<Frame>;
@group(0) @binding(1) var<storage, read> groups: array<Record>;
@group(0) @binding(2) var<storage, read> weld: array<Record>;
@group(0) @binding(3) var<storage, read_write> donors: array<atomic<u32>>;
@group(0) @binding(4) var<uniform> params: Params;

@compute @workgroup_size(64)
fn clear_donors(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= params.corners { return; }
    atomicStore(&donors[id.x], 0xffffffffu);
}

@compute @workgroup_size(64)
fn select_donors(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= params.corners { return; }
    let frame = source[id.x];
    if groups[id.x].b.z == 0u && frame.identity.y != 0xffffffffu &&
        all(frame.status == vec4(0u)) {
        atomicMin(&donors[weld[id.x].c.z], id.x);
    }
}
