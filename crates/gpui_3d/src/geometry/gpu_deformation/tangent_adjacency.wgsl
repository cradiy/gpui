struct Record { a: vec4<u32>, b: vec4<u32>, c: vec4<u32>, status: vec4<u32> }
struct Params { corners: u32, distance: u32, width: u32, capacity: u32 }
@group(0) @binding(0) var<storage, read> source: array<Record>;
@group(0) @binding(1) var<storage, read> faces: array<Record>;
@group(0) @binding(2) var<storage, read> weld: array<Record>;
@group(0) @binding(3) var<storage, read_write> output: array<Record>;
@group(0) @binding(4) var<uniform> params: Params;

fn next_corner(corner: u32) -> u32 { return corner / 3u * 3u + (corner % 3u + 1u) % 3u; }
fn bucket(value: Record) -> u32 {
    if value.b.x == 4u { return 2u; }
    if value.b.x >= 2u { return 1u; }
    return 0u;
}

@compute @workgroup_size(64)
fn initialize(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= params.capacity { return; }
    var result: Record;
    if id.x >= params.corners {
        result.a = vec4(0xffffffffu);
        result.b.x = 4u;
    } else {
        let first = id.x / 3u * 3u;
        result.c = faces[id.x / 3u].c;
        for (var corner = 0u; corner < 3u; corner++) {
            let status = weld[first + corner].status;
            if any(status != vec4(0u)) { result.status = status; break; }
        }
        if all(result.status == vec4(0u)) { result.status = faces[id.x / 3u].status; }
        let p0 = bitcast<vec3<f32>>(weld[first].a.xyz);
        let p1 = bitcast<vec3<f32>>(weld[first + 1u].a.xyz);
        let p2 = bitcast<vec3<f32>>(weld[first + 2u].a.xyz);
        var kind = 0u;
        if any(result.status != vec4(0u)) { kind = 3u; }
        else if all(p0 == p1) || all(p1 == p2) || all(p2 == p0) { kind = 2u; }
        else if result.c.y != 0u || result.c.w != 0u { kind = 1u; }
        let start = weld[id.x].c.z;
        let end = weld[next_corner(id.x)].c.z;
        result.a = vec4(min(start, end), max(start, end), u32(start > end), id.x);
        result.b = vec4(kind, id.x, start, end);
    }
    output[id.x] = result;
}

fn less(a: Record, b: Record) -> bool {
    if bucket(a) != bucket(b) { return bucket(a) < bucket(b); }
    for (var lane = 0u; lane < 4u; lane++) {
        if a.a[lane] != b.a[lane] { return a.a[lane] < b.a[lane]; }
    }
    return false;
}
@compute @workgroup_size(64)
fn sort_pairs(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= params.capacity { return; }
    let a = source[id.x];
    let b = source[id.x ^ params.distance];
    let low = ((id.x & params.distance) == 0u) == ((id.x & params.width) == 0u);
    if less(a, b) == low { output[id.x] = a; } else { output[id.x] = b; }
}

fn lower_bound(key: vec3<u32>) -> u32 {
    var lo = 0u;
    var hi = params.corners;
    while lo < hi {
        let mid = lo + (hi - lo) / 2u;
        let item = source[mid];
        let before = bucket(item) == 0u && (item.a.x < key.x || (item.a.x == key.x &&
            (item.a.y < key.y || (item.a.y == key.y && item.a.z < key.z))));
        if before { lo = mid + 1u; } else { hi = mid; }
    }
    return lo;
}

@compute @workgroup_size(64)
fn resolve(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= params.corners { return; }
    let item = source[id.x];
    var result: Record;
    result.a = vec4(item.b.yzw, 0xffffffffu);
    result.b = vec4(0xffffffffu, 0xffffffffu, 0u, item.b.x);
    result.c = item.c;
    result.status = item.status;
    if bucket(item) == 0u {
        let own_first = lower_bound(item.a.xyz);
        let opposite_direction = 1u - item.a.z;
        let opposite_first = lower_bound(vec3(item.a.xy, opposite_direction));
        let opposite_end = lower_bound(vec3(item.a.xy, opposite_direction + 1u));
        let partner = opposite_first + (id.x - own_first);
        if partner < opposite_end {
            let other = source[partner];
            result.a.w = other.b.y;
            result.b.x = next_corner(other.b.y);
            result.b.y = other.b.y;
            result.b.z = u32(item.b.x == 0u && other.b.x == 0u && item.c.z == other.c.z);
        }
    }
    output[item.b.y] = result;
}
