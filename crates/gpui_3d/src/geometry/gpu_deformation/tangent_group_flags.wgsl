struct Record { a: vec4<u32>, b: vec4<u32>, c: vec4<u32>, status: vec4<u32> }
struct Params { corners: u32, padding_0: u32, padding_1: u32, padding_2: u32 }
@group(0) @binding(0) var<storage, read> edges: array<Record>;
@group(0) @binding(3) var<storage, read_write> flags: array<u32>;
@group(0) @binding(4) var<uniform> params: Params;
var<workgroup> needed: atomic<u32>;

@compute @workgroup_size(64)
fn detect_inheritance(@builtin(global_invocation_id) id: vec3<u32>,
    @builtin(local_invocation_index) local: u32, @builtin(workgroup_id) group: vec3<u32>) {
    if local == 0u { atomicStore(&needed, 0u); }
    workgroupBarrier();
    if id.x < params.corners && edges[id.x].b.w == 1u {
        atomicOr(&needed, 1u);
    }
    workgroupBarrier();
    if local == 0u { flags[group.x] = atomicLoad(&needed); }
}
