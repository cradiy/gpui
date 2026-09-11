struct Vertex {
    position: vec4<f32>, normal: vec4<f32>, tangent: vec4<f32>, status: vec4<u32>,
}
struct Frame {
    tangent: vec4<f32>, bitangent: vec4<f32>,
    identity: vec3<u32>, weight: f32, status: vec4<u32>,
}
struct Topology { vertex: u32, reserved: u32, uv: vec2<f32> }
struct Params { vertices: u32, mode: u32, reserved: vec2<u32> }
@group(0) @binding(0) var<storage, read> source: array<Vertex>;
@group(0) @binding(1) var<storage, read> frames: array<Frame>;
@group(0) @binding(2) var<storage, read> topology: array<Topology>;
@group(0) @binding(3) var<storage, read_write> output: array<Vertex>;
@group(0) @binding(4) var<storage, read_write> repairs: array<u32>;
@group(0) @binding(5) var<uniform> params: Params;

fn finite(v: vec4<f32>) -> bool {
    return all((bitcast<vec4<u32>>(v) & vec4(0x7f800000u)) != vec4(0x7f800000u));
}
fn magnitude(v: vec3<f32>) -> f32 { return max(max(abs(v.x), abs(v.y)), abs(v.z)); }
fn unit(v: vec3<f32>) -> vec3<f32> {
    let scale = magnitude(v);
    if scale == 0.0 { return vec3(0.0); }
    let scaled = v / scale;
    return scaled / length(scaled);
}
fn orthogonal(v: vec3<f32>, normal: vec3<f32>) -> vec3<f32> {
    let direction = unit(v);
    let projected = direction - dot(normal, direction) * normal;
    if length(projected) <= 0.000001 { return vec3(0.0); }
    return unit(projected);
}
fn normal_basis(normal: vec3<f32>) -> vec3<f32> {
    var axis = vec3(1.0, 0.0, 0.0);
    if abs(normal.y) < abs(normal.x) && abs(normal.y) <= abs(normal.z) {
        axis = vec3(0.0, 1.0, 0.0);
    } else if abs(normal.z) < abs(normal.x) && abs(normal.z) < abs(normal.y) {
        axis = vec3(0.0, 0.0, 1.0);
    }
    return orthogonal(axis, normal);
}
fn reject(first: u32, status: vec4<u32>) {
    for (var corner = 0u; corner < 3u; corner++) {
        let vertex = topology[first + corner].vertex;
        var result = source[vertex];
        result.tangent = vec4(0.0);
        result.status = status;
        output[vertex] = result;
        repairs[vertex] = 0u;
    }
}

@compute @workgroup_size(64)
fn publish(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= params.vertices / 3u { return; }
    let first = id.x * 3u;
    var normals: array<vec3<f32>, 3>;
    var tangents: array<vec4<f32>, 3>;
    var tags: array<u32, 3>;
    for (var corner = 0u; corner < 3u; corner++) {
        let input = source[topology[first + corner].vertex];
        if any(input.status != vec4(0u)) { reject(first, input.status); return; }
        if !finite(input.position) || !finite(input.normal) {
            reject(first, vec4(1u, 0u, 0u, 0u)); return;
        }
        normals[corner] = unit(input.normal.xyz);
        if all(normals[corner] == vec3(0.0)) {
            reject(first, vec4(2u, 0u, 0u, 0u)); return;
        }
        let frame = frames[first + corner];
        if any(frame.status != vec4(0u)) && any(frame.status != vec4(2u, 0u, 0u, 0u)) {
            reject(first, frame.status); return;
        }
        if !finite(frame.tangent) || !finite(frame.bitangent) || !finite(vec4(frame.weight)) {
            reject(first, vec4(1u, 0u, 0u, 0u)); return;
        }
        if all(frame.status == vec4(0u)) && frame.identity.y != 0xffffffffu && frame.identity.z <= 1u {
            let tangent = orthogonal(frame.tangent.xyz, normals[corner]);
            if any(tangent != vec3(0.0)) {
                tangents[corner] = vec4(tangent, select(-1.0, 1.0, frame.identity.z == 1u));
            }
        }
    }
    let a = topology[first];
    let b = topology[first + 1u];
    let c = topology[first + 2u];
    let e = source[b.vertex].position.xyz - source[a.vertex].position.xyz;
    let f = source[c.vertex].position.xyz - source[a.vertex].position.xyz;
    let duv = b.uv - a.uv;
    let euv = c.uv - a.uv;
    let determinant = duv.x * euv.y - duv.y * euv.x;
    let derivative = (e * euv.y - f * duv.y) * select(-1.0, 1.0, determinant >= 0.0);
    if !finite(vec4(e, 0.0)) || !finite(vec4(f, 0.0)) || !finite(vec4(duv, euv)) ||
        !finite(vec4(determinant)) || !finite(vec4(derivative, 0.0)) {
        reject(first, vec4(1u, 0u, 0u, 0u)); return;
    }
    if params.mode == 0u {
        let es = magnitude(e);
        let fs = magnitude(f);
        var zero_area = es == 0.0 || fs == 0.0;
        if !zero_area { zero_area = all(cross(e / es, f / fs) == vec3(0.0)); }
        if zero_area {
            reject(first, vec4(4u, 0u, 0u, 0u)); return;
        }
        if determinant == 0.0 { reject(first, vec4(2u, 0u, 0u, 0u)); return; }
    }
    var repair_sign = select(-1.0, 1.0, determinant >= 0.0);
    for (var corner = 0u; corner < 3u; corner++) {
        if tangents[corner].w != 0.0 { repair_sign = tangents[corner].w; break; }
    }
    for (var corner = 0u; corner < 3u; corner++) {
        if tangents[corner].w == 0.0 {
            if params.mode != 2u { reject(first, vec4(2u, 0u, 0u, 0u)); return; }
            var tangent = vec3(0.0);
            if determinant != 0.0 { tangent = orthogonal(derivative, normals[corner]); }
            if any(tangent != vec3(0.0)) { tags[corner] = 1u; }
            else { tangent = normal_basis(normals[corner]); tags[corner] = 2u; }
            tangents[corner] = vec4(tangent, repair_sign);
        }
    }
    if tangents[0].w != tangents[1].w || tangents[0].w != tangents[2].w {
        reject(first, vec4(2u, 0u, 0u, 0u)); return;
    }
    for (var corner = 0u; corner < 3u; corner++) {
        let vertex = topology[first + corner].vertex;
        var result = source[vertex];
        result.tangent = tangents[corner];
        result.status = vec4(0u);
        output[vertex] = result;
        repairs[vertex] = tags[corner];
    }
}
