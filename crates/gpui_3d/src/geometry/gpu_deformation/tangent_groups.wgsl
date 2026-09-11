struct Record { a: vec4<u32>, b: vec4<u32>, c: vec4<u32>, status: vec4<u32> }
struct Params { corners: u32, padding_0: u32, padding_1: u32, padding_2: u32 }
@group(0) @binding(0) var<storage, read> source: array<Record>;
@group(0) @binding(1) var<storage, read> edges: array<Record>;
@group(0) @binding(2) var<storage, read> weld: array<Record>;
@group(0) @binding(3) var<storage, read_write> output: array<Record>;
@group(0) @binding(4) var<uniform> params: Params;

@compute @workgroup_size(64)
fn initialize(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= params.corners { return; }
    let edge = edges[id.x];
    var result: Record;
    result.a = vec4(id.x, weld[id.x].c.z, 0xffffffffu, 0xffffffffu);
    result.b = vec4(0xffffffffu, 0xffffffffu, edge.b.w, 0u);
    result.status = edge.status;
    if edge.b.w == 0u && all(edge.status == vec4(0u)) {
        result.a.z = id.x;
        result.a.w = edge.c.z;
        if edge.b.z != 0u { result.b.x = edge.b.x; }
        let previous = id.x / 3u * 3u + (id.x % 3u + 2u) % 3u;
        if edges[previous].b.z != 0u { result.b.y = edges[previous].b.y; }
    }
    result.c = vec4(result.b.xy, 0u, 0u);
    output[id.x] = result;
}

@compute @workgroup_size(64)
fn propagate(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= params.corners { return; }
    var result = source[id.x];
    if result.a.z != 0xffffffffu {
        let forward = result.c.x;
        let backward = result.c.y;
        if forward != 0xffffffffu {
            let other = source[forward];
            result.a.z = min(result.a.z, other.a.z);
            result.c.x = other.c.x;
        }
        if backward != 0xffffffffu {
            let other = source[backward];
            result.a.z = min(result.a.z, other.a.z);
            result.c.y = other.c.y;
        }
    }
    output[id.x] = result;
}

@compute @workgroup_size(64)
fn finalize(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= params.corners { return; }
    var result = source[id.x];
    if result.a.z != 0xffffffffu {
        result.b.x = 0xffffffffu;
        result.b.y = 0xffffffffu;
        let outgoing = edges[id.x].b.x;
        let previous = id.x / 3u * 3u + (id.x % 3u + 2u) % 3u;
        let incoming = edges[previous].b.y;
        if outgoing != 0xffffffffu && source[outgoing].a.z == result.a.z { result.b.x = outgoing; }
        if incoming != 0xffffffffu && source[incoming].a.z == result.a.z { result.b.y = incoming; }
    }
    result.c = vec4(0u);
    output[id.x] = result;
}
