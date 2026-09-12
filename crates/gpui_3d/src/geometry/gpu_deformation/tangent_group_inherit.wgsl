struct Record { a: vec4<u32>, b: vec4<u32>, c: vec4<u32>, status: vec4<u32> }
struct Params { corners: u32, padding_0: u32, padding_1: u32, padding_2: u32 }
@group(0) @binding(0) var<storage, read> flags: array<u32>;
@group(0) @binding(1) var<storage, read> edges: array<Record>;
@group(0) @binding(3) var<storage, read_write> state: array<Record>;
@group(0) @binding(4) var<uniform> params: Params;

fn push(corner: u32, seed: u32, orientation: u32, head: u32) -> u32 {
    if corner == 0xffffffffu { return head; }
    let edge = edges[corner];
    if edge.b.w > 1u || any(edge.status != vec4(0u)) || state[corner].a.z != 0xffffffffu {
        return head;
    }
    let first = corner / 3u * 3u;
    if state[first].c.z == 0xffffffffu { state[first].c.z = orientation; }
    if state[first].c.z != orientation { return head; }
    state[corner].a.z = seed;
    state[corner].a.w = orientation;
    state[corner].c.x = head;
    return corner;
}

@compute @workgroup_size(64)
fn inherit(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x != 0u { return; }
    var needed = false;
    for (var index = 0u; index < params.corners / 64u + u32(params.corners % 64u != 0u); index++) {
        needed = needed || flags[index] != 0u;
    }
    if !needed { return; }
    for (var corner = 0u; corner < params.corners; corner++) {
        state[corner].a.z = 0xffffffffu;
        state[corner].a.w = 0xffffffffu;
        state[corner].c = vec4(0u, 0u, 0xffffffffu, 0u);
        if edges[corner].b.w == 0u { state[corner].c.z = edges[corner].c.z; }
    }
    for (var seed = 0u; seed < params.corners; seed++) {
        if edges[seed].b.w != 0u || state[seed].a.z != 0xffffffffu { continue; }
        let orientation = edges[seed].c.z;
        var head = push(seed, seed, orientation, 0xffffffffu);
        while head != 0xffffffffu {
            let corner = head;
            head = state[corner].c.x;
            let previous = corner / 3u * 3u + (corner % 3u + 2u) % 3u;
            head = push(edges[corner].b.x, seed, orientation, head);
            head = push(edges[previous].b.y, seed, orientation, head);
        }
    }
}
