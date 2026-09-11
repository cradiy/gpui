struct Record {
    key_0: vec4<u32>,
    key_1: vec4<u32>,
    identity: vec4<u32>,
    status: vec4<u32>,
}
struct Params { corners: u32, distance: u32, width: u32, capacity: u32 }
// During initialization the source has the 64-byte GpuDeformationVertex layout.
// Position and normal bits occupy key_0 and key_1; the tangent lanes are unused.
@group(0) @binding(0) var<storage, read> source: array<Record>;
@group(0) @binding(1) var<storage, read> uv: array<vec2<f32>>;
@group(0) @binding(2) var<storage, read> indices: array<u32>;
@group(0) @binding(3) var<storage, read_write> output: array<Record>;
@group(0) @binding(4) var<uniform> params: Params;

fn finite(v: vec3<f32>) -> bool {
    return all((bitcast<vec3<u32>>(v) & vec3(0x7f800000u)) != vec3(0x7f800000u));
}

fn normal_key(bits: vec3<u32>) -> vec3<u32> {
    let x = wide_component(bits.x);
    let y = wide_component(bits.y);
    let z = wide_component(bits.z);
    let squared_xy = x * x + y * y;
    let magnitude = sqrt(squared_xy + z * z);
    let signs = bits & vec3(0x80000000u);
    return vec3(
        component_key(x / magnitude, signs.x),
        component_key(y / magnitude, signs.y),
        component_key(z / magnitude, signs.z),
    );
}

@compute @workgroup_size(64)
fn initialize(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= params.capacity { return; }
    var result: Record;
    if id.x >= params.corners {
        result.identity = vec4(0xffffffffu, 0xffffffffu, 0xffffffffu, 2u);
    } else {
        let vertex = indices[id.x];
        let input = source[vertex];
        let position = bitcast<vec3<f32>>(input.key_0.xyz);
        let normal = bitcast<vec3<f32>>(input.key_1.xyz);
        let coords = uv[vertex];
        result.identity = vec4(id.x, vertex, id.x, 0u);
        result.status = input.status;
        if all(result.status == vec4(0u)) {
            if !finite(position) || !finite(normal) || !finite(vec3(coords, 0.0)) {
                result.status.x = 1u;
            } else {
                if all((input.key_1.xyz & vec3(0x7fffffffu)) == vec3(0u)) {
                    result.status.x = 2u;
                } else {
                    let n = normal_key(input.key_1.xyz);
                    result.key_0 = vec4(input.key_0.xyz, n.x);
                    result.key_1 = vec4(n.yz, bitcast<vec2<u32>>(coords));
                }
            }
        }
        if any(result.status != vec4(0u)) { result.identity.w = 1u; }
    }
    output[id.x] = result;
}

fn key_less(a: Record, b: Record) -> bool {
    if a.identity.w != b.identity.w { return a.identity.w < b.identity.w; }
    for (var lane = 0u; lane < 4u; lane++) {
        if a.key_0[lane] != b.key_0[lane] { return a.key_0[lane] < b.key_0[lane]; }
    }
    for (var lane = 0u; lane < 4u; lane++) {
        if a.key_1[lane] != b.key_1[lane] { return a.key_1[lane] < b.key_1[lane]; }
    }
    return false;
}
fn less(a: Record, b: Record) -> bool {
    if key_less(a, b) { return true; }
    if key_less(b, a) { return false; }
    return a.identity.x < b.identity.x;
}

@compute @workgroup_size(64)
fn sort_pairs(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= params.capacity { return; }
    let a = source[id.x];
    let b = source[id.x ^ params.distance];
    let ascending = (id.x & params.width) == 0u;
    let want_low = ((id.x & params.distance) == 0u) == ascending;
    if less(a, b) == want_low { output[id.x] = a; }
    else { output[id.x] = b; }
}

@compute @workgroup_size(64)
fn resolve(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= params.corners { return; }
    var result = source[id.x];
    if result.identity.w == 0u {
        var lo = 0u;
        var hi = id.x;
        while lo < hi {
            let mid = lo + (hi - lo) / 2u;
            if key_less(source[mid], result) { lo = mid + 1u; }
            else { hi = mid; }
        }
        result.identity.z = source[lo].identity.x;
    }
    output[result.identity.x] = result;
}
