struct Vertex {
    position: vec4<f32>,
    normal: vec4<f32>,
    tangent: vec4<f32>,
    status: vec4<u32>,
}
struct Derivative {
    tangent: vec4<f32>,
    bitangent: vec4<f32>,
    classification: vec4<u32>,
    status: vec4<u32>,
}
struct Params { triangles: u32, padding_0: u32, padding_1: u32, padding_2: u32 }
const MIN_NORMAL: f32 = 0x1p-126;
@group(0) @binding(0) var<storage, read> source: array<Vertex>;
@group(0) @binding(1) var<storage, read> uv: array<vec2<f32>>;
@group(0) @binding(2) var<storage, read> indices: array<u32>;
@group(0) @binding(3) var<storage, read_write> output: array<Derivative>;
@group(0) @binding(4) var<uniform> params: Params;

fn finite(v: vec4<f32>) -> bool {
    return all((bitcast<vec4<u32>>(v) & vec4(0x7f800000u)) != vec4(0x7f800000u));
}
fn magnitude(v: vec3<f32>) -> f32 { return max(max(abs(v.x), abs(v.y)), abs(v.z)); }
fn squared_length(v: vec3<f32>) -> f32 { return (v.x * v.x + v.y * v.y) + v.z * v.z; }
fn normal_float(v: f32) -> bool {
    let exponent = bitcast<u32>(v) & 0x7f800000u;
    return exponent != 0u && exponent != 0x7f800000u;
}

@compute @workgroup_size(64)
fn tangent_derivatives(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= params.triangles { return; }
    var result: Derivative;
    let face = id.x * 3u;
    let ia = indices[face];
    let ib = indices[face + 1u];
    let ic = indices[face + 2u];
    for (var corner = 0u; corner < 3u; corner++) {
        let vertex = source[indices[face + corner]];
        if any(vertex.status != vec4(0u)) {
            result.status = vertex.status;
            output[id.x] = result;
            return;
        }
    }
    let a = source[ia].position;
    let b = source[ib].position;
    let c = source[ic].position;
    let e = b.xyz - a.xyz;
    let f = c.xyz - a.xyz;
    let u = uv[ib] - uv[ia];
    let v = uv[ic] - uv[ia];
    let determinant = u.x * v.y - u.y * v.x;
    let s = e * v.y - f * u.y;
    let t = f * u.x - e * v.x;
    if !finite(a) || !finite(b) || !finite(c) || !finite(vec4(e, determinant))
        || !finite(vec4(f, 0.0)) || !finite(vec4(u, v))
        || !finite(vec4(s, 0.0)) || !finite(vec4(t, 0.0)) {
        result.status.x = 1u;
        output[id.x] = result;
        return;
    }
    let es = magnitude(e);
    let fs = magnitude(f);
    var zero_area = es == 0.0 || fs == 0.0;
    if !zero_area { zero_area = all(cross(e / es, f / fs) == vec3(0.0)); }
    result.classification = vec4(u32(zero_area), u32(determinant == 0.0), u32(determinant > 0.0), 0u);
    let ss = magnitude(s);
    let ts = magnitude(t);
    if abs(determinant) <= MIN_NORMAL || ss == 0.0 || ts == 0.0 {
        result.classification.w = 1u;
    } else {
        let squared_s = squared_length(s);
        let squared_t = squared_length(t);
        if !normal_float(squared_s) || !normal_float(squared_t) {
            result.status.x = 5u;
            output[id.x] = result;
            return;
        }
        let sl = sqrt(squared_s);
        let tl = sqrt(squared_t);
        result.tangent = vec4((s * (1.0 / sl)) * sign(determinant), sl / abs(determinant));
        result.bitangent = vec4((t * (1.0 / tl)) * sign(determinant), tl / abs(determinant));
        if !finite(result.tangent) || !finite(result.bitangent) {
            result.status.x = 5u;
        } else if result.tangent.w <= MIN_NORMAL || result.bitangent.w <= MIN_NORMAL {
            result.classification.w = 1u;
            result.tangent = vec4(0.0);
            result.bitangent = vec4(0.0);
        }
    }
    output[id.x] = result;
}
