struct Vertex {
    position: vec4<f32>,
    normal: vec4<f32>,
    tangent: vec4<f32>,
    status: vec4<u32>,
}
struct Params {
    vertices: u32,
    padding_0: u32,
    padding_1: u32,
    padding_2: u32,
}
@group(0) @binding(0) var<storage, read> source: array<Vertex>;
@group(0) @binding(1) var<storage, read> faces: array<u32>;
@group(0) @binding(2) var<storage, read> indices: array<u32>;
@group(0) @binding(3) var<storage, read_write> output: array<Vertex>;
@group(0) @binding(4) var<uniform> params: Params;

fn finite(v: vec3<f32>) -> bool {
    let bits = bitcast<vec3<u32>>(v);
    return all((bits & vec3(0x7f800000u)) != vec3(0x7f800000u));
}
fn magnitude(v: vec3<f32>) -> f32 {
    return max(max(abs(v.x), abs(v.y)), abs(v.z));
}

@compute @workgroup_size(64)
fn flat_normals(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= params.vertices { return; }
    var result = source[id.x];
    let face = faces[id.x];
    let a = source[indices[face]];
    let b = source[indices[face + 1u]];
    let c = source[indices[face + 2u]];
    for (var corner = 0u; corner < 3u; corner++) {
        let status = source[indices[face + corner]].status;
        if any(status != vec4(0u)) {
            result.status = status;
            output[id.x] = result;
            return;
        }
    }
    let u = b.position.xyz - a.position.xyz;
    let v = c.position.xyz - a.position.xyz;
    if !finite(a.position.xyz) || !finite(b.position.xyz) || !finite(c.position.xyz)
        || !finite(u) || !finite(v) {
        result.status.x = 1u;
    } else {
        let su = magnitude(u);
        let sv = magnitude(v);
        if su == 0.0 || sv == 0.0 {
            result.status.x = 4u;
        } else {
            let n = cross(u / su, v / sv);
            let scale = magnitude(n);
            if scale == 0.0 {
                result.status.x = 4u;
            } else {
                let scaled = n / scale;
                result.normal = vec4(scaled / length(scaled), 0.0);
                if !finite(result.normal.xyz) { result.status.x = 1u; }
            }
        }
    }
    output[id.x] = result;
}
