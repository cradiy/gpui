struct Frame {
    tangent: vec4<f32>, bitangent: vec4<f32>,
    identity: vec3<u32>, weight: f32, status: vec4<u32>,
}
struct Record { a: vec4<u32>, b: vec4<u32>, c: vec4<u32>, status: vec4<u32> }
struct Params { corners: u32, distance: u32, width: u32, capacity: u32 }
// Initialization reads the matching 64-byte face derivative layout from source.
@group(0) @binding(0) var<storage, read> source: array<Frame>;
@group(0) @binding(1) var<storage, read> groups: array<Record>;
@group(0) @binding(2) var<storage, read> weld: array<Record>;
@group(0) @binding(3) var<storage, read_write> output: array<Frame>;
@group(0) @binding(4) var<uniform> params: Params;

fn finite(v: vec4<f32>) -> bool {
    return all((bitcast<vec4<u32>>(v) & vec4(0x7f800000u)) != vec4(0x7f800000u));
}
// W reports whether normalization is defined. Small components retain their
// magnitude; rescaling them would alter subgroup membership and corner weights.
fn unit(v: vec3<f32>) -> vec4<f32> {
    if !any(abs(v) > vec3(0x1p-126)) { return vec4(v, 1.0); }
    let squared = (v.x * v.x + v.y * v.y) + v.z * v.z;
    if squared == 0.0 { return vec4(0.0); }
    if !finite(vec4(squared)) { return vec4(vec3(0.0), 1.0); }
    let result = v * (1.0 / sqrt(squared));
    if !finite(vec4(result, 0.0)) { return vec4(0.0); }
    return vec4(result, 1.0);
}
fn project(v: vec3<f32>, normal: vec3<f32>) -> vec3<f32> {
    return v - dot(normal, v) * normal;
}

@compute @workgroup_size(64)
fn initialize(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= params.capacity { return; }
    var result: Frame;
    result.identity = vec3(0xffffffffu);
    if id.x < params.corners {
        let group = groups[id.x];
        result.identity = vec3(id.x, group.a.zw);
        result.status = group.status;
        if group.a.z != 0xffffffffu && group.b.z == 0u && all(result.status == vec4(0u)) {
            let face = source[id.x / 3u];
            let corner = weld[id.x];
            let normal = bitcast<vec3<f32>>(vec3(corner.a.w, corner.b.xy));
            let position = bitcast<vec3<f32>>(corner.a.xyz);
            let next = id.x / 3u * 3u + (id.x % 3u + 1u) % 3u;
            let previous = id.x / 3u * 3u + (id.x % 3u + 2u) % 3u;
            let e = project(bitcast<vec3<f32>>(weld[next].a.xyz) - position, normal);
            let f = project(bitcast<vec3<f32>>(weld[previous].a.xyz) - position, normal);
            let s = project(face.tangent.xyz, normal);
            let t = project(face.bitangent.xyz, normal);
            if !finite(vec4(e, 0.0)) || !finite(vec4(f, 0.0))
                || !finite(vec4(s, 0.0)) || !finite(vec4(t, 0.0)) {
                result.status.x = 1u;
            } else {
                let edge_e = unit(e);
                let edge_f = unit(f);
                let tangent = unit(s);
                let bitangent = unit(t);
                result.weight = acos(clamp(dot(edge_e.xyz, edge_f.xyz), -1.0, 1.0));
                result.tangent = vec4(tangent.xyz, face.tangent.w);
                result.bitangent = vec4(bitangent.xyz, face.bitangent.w);
                // Scratch-only flags above the orientation bit: undefined S, T,
                // or angle. Direction failures exclude regular subgroup matches;
                // angle failures invalidate only groups accepting this corner.
                if tangent.w == 0.0 { result.identity.z |= 2u; }
                if bitangent.w == 0.0 { result.identity.z |= 4u; }
                if edge_e.w == 0.0 || edge_f.w == 0.0 { result.identity.z |= 8u; }
            }
        }
    }
    output[id.x] = result;
}

fn less(a: Frame, b: Frame) -> bool {
    return a.identity.y < b.identity.y ||
        (a.identity.y == b.identity.y && a.identity.x < b.identity.x);
}
@compute @workgroup_size(64)
fn sort_pairs(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= params.capacity { return; }
    let a = source[id.x];
    let b = source[id.x ^ params.distance];
    let low = ((id.x & params.distance) == 0u) == ((id.x & params.width) == 0u);
    if less(a, b) == low { output[id.x] = a; } else { output[id.x] = b; }
}

@compute @workgroup_size(64)
fn accumulate(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= params.corners { return; }
    let item = source[id.x];
    var result: Frame;
    result.identity = item.identity;
    result.status = item.status;
    if item.identity.y == 0xffffffffu {
        output[item.identity.x] = result;
        return;
    }
    result.identity.z &= 1u;
    var bitangent_defined = true;
    var lo = 0u;
    var hi = id.x;
    while lo < hi {
        let mid = lo + (hi - lo) / 2u;
        if source[mid].identity.y < item.identity.y { lo = mid + 1u; }
        else { hi = mid; }
    }
    for (var index = lo; index < params.corners; index++) {
        let other = source[index];
        if other.identity.y != item.identity.y { break; }
        if any(other.status != vec4(0u)) {
            result.status = other.status;
            output[item.identity.x] = result;
            return;
        }
        if other.identity.x == item.identity.x || groups[item.identity.x].b.z == 1u ||
            groups[other.identity.x].b.z == 1u ||
            (((item.identity.z | other.identity.z) & 6u) == 0u &&
             dot(item.tangent.xyz, other.tangent.xyz) > -1.0 &&
             dot(item.bitangent.xyz, other.bitangent.xyz) > -1.0) {
            if (other.identity.z & 10u) != 0u {
                result.status.x = 2u;
                output[item.identity.x] = result;
                return;
            }
            if (other.identity.z & 4u) != 0u { bitangent_defined = false; }
            result.tangent += other.tangent * other.weight;
            result.bitangent += other.bitangent * other.weight;
            result.weight += other.weight;
        }
    }
    if !finite(result.tangent) || !finite(result.bitangent) || !finite(vec4(result.weight)) {
        result.status.x = 1u;
    } else if result.weight == 0.0 || all(result.tangent.xyz == vec3(0.0)) {
        result.status.x = 2u;
    } else {
        let tangent = unit(result.tangent.xyz);
        let bitangent = unit(result.bitangent.xyz);
        result.tangent = vec4(tangent.xyz, result.tangent.w / result.weight);
        result.bitangent = vec4(bitangent.xyz, result.bitangent.w / result.weight);
        if !bitangent_defined { result.bitangent = vec4(vec3(0.0), result.bitangent.w); }
        if tangent.w == 0.0 || all(tangent.xyz == vec3(0.0)) { result.status.x = 2u; }
    }
    output[item.identity.x] = result;
}
