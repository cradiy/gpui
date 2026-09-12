struct Vertex {
    position: vec4<f32>,
    normal: vec4<f32>,
    tangent: vec4<f32>,
    status: vec4<u32>,
}
struct Params {
    vertices: u32,
    tangents: u32,
    offset_words: u32,
    padding: u32,
}
@group(0) @binding(0) var<storage, read> source: array<Vertex>;
@group(0) @binding(1) var<storage, read> influences: array<u32>;
@group(0) @binding(2) var<storage, read> palette: array<mat4x4<f32>>;
@group(0) @binding(3) var<storage, read_write> output: array<Vertex>;
@group(0) @binding(4) var<uniform> params: Params;

fn finite(v: vec3<f32>) -> bool {
    return all(abs(v) <= vec3<f32>(3.402823466e+38));
}
fn magnitude(v: vec3<f32>) -> f32 {
    return max(max(abs(v.x), abs(v.y)), abs(v.z));
}
fn unit(v: vec3<f32>) -> vec3<f32> {
    let scale = magnitude(v);
    if scale == 0.0 { return vec3<f32>(0.0); }
    let scaled = v / scale;
    return scaled / length(scaled);
}

@compute @workgroup_size(64)
fn skin(@builtin(global_invocation_id) id: vec3<u32>) {
    let vertex = id.x;
    if vertex >= params.vertices { return; }
    var result = source[vertex];
    if any(result.status != vec4<u32>(0u)) {
        output[vertex] = result;
        return;
    }
    var matrix = mat4x4<f32>();
    for (var influence = influences[vertex]; influence < influences[vertex + 1u]; influence += 1u) {
        let word = params.offset_words + influence * 2u;
        let joint = influences[word];
        let weight = bitcast<f32>(influences[word + 1u]);
        matrix += palette[joint] * weight;
    }
    let a = matrix[0].xyz;
    let b = matrix[1].xyz;
    let c = matrix[2].xyz;
    let translation = matrix[3].xyz;
    if !finite(a) || !finite(b) || !finite(c) || !finite(translation) {
        result.status.x = 1u;
        output[vertex] = result;
        return;
    }
    let scales = vec3<f32>(magnitude(a), magnitude(b), magnitude(c));
    if any(scales == vec3<f32>(0.0)) {
        result.status.x = 3u;
        output[vertex] = result;
        return;
    }
    let u = a / scales.x;
    let v = b / scales.y;
    let w = c / scales.z;
    let determinant = dot(u, cross(v, w));
    let volume = length(u) * length(v) * length(w);
    if abs(determinant) <= volume * 0.00000001 {
        result.status.x = 3u;
        output[vertex] = result;
        return;
    }
    let inverse_transpose = mat3x3<f32>(
        (cross(v, w) / determinant) / scales.x,
        (cross(w, u) / determinant) / scales.y,
        (cross(u, v) / determinant) / scales.z,
    );
    if !finite(inverse_transpose[0]) || !finite(inverse_transpose[1]) || !finite(inverse_transpose[2])
        || !finite(transpose(inverse_transpose) * translation) {
        result.status.x = 3u;
        output[vertex] = result;
        return;
    }
    let linear = mat3x3<f32>(a, b, c);
    let position = linear * result.position.xyz + translation;
    let normal = inverse_transpose * result.normal.xyz;
    let tangent = linear * result.tangent.xyz;
    if !finite(position) || !finite(normal) || !finite(tangent) {
        result.status.x = 1u;
        output[vertex] = result;
        return;
    }
    result.position = vec4<f32>(position, 0.0);
    let n = unit(normal);
    result.normal = vec4<f32>(n, 0.0);
    if params.tangents != 0u {
        let t = unit(tangent);
        let orthogonal = t - n * dot(n, t);
        if length(n) == 0.0 || length(orthogonal) <= 0.000001 {
            result.status.x = 2u;
        } else {
            result.tangent = vec4<f32>(unit(orthogonal), result.tangent.w * sign(determinant));
        }
    }
    if !finite(result.normal.xyz) || !finite(result.tangent.xyz) { result.status.x = 1u; }
    output[vertex] = result;
}
