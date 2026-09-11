struct Attributes {
    position: vec4<f32>,
    normal: vec4<f32>,
    tangent: vec4<f32>,
    status: vec4<u32>,
}
const VERTEX_WORDS: u32 = 24u;
const NORMAL_WORD: u32 = 3u;
const TANGENT_WORD: u32 = 10u;
const UV_WORDS = array<u32, 5>(6u, 8u, 14u, 16u, 18u);
const COLOR_WORD = 20u;
const INVALID_UV = 1u;
const INVALID_COLOR = 2u;
const DEFORMATION_STATUS = 4u;
const NONFINITE_DEFORMATION = 8u;
const INVALID_TANGENT = 16u;
const TRIANGLE_TANGENT_SIGN = 32u;
@group(0) @binding(0) var<storage, read> base: array<u32>;
@group(0) @binding(1) var<storage, read> attributes: array<Attributes>;
@group(0) @binding(2) var<storage, read> indices: array<u32>;
@group(0) @binding(3) var<storage, read_write> vertices: array<u32>;
@group(0) @binding(4) var<storage, read_write> draw: array<atomic<u32>>;

fn finite(v: vec4<f32>) -> bool {
    return all(abs(v) <= vec4<f32>(3.402823466e+38));
}

fn unit(v: vec3<f32>) -> vec3<f32> {
    let scale = max(max(abs(v.x), abs(v.y)), abs(v.z));
    if scale == 0.0 { return vec3<f32>(0.0); }
    let scaled = v / scale;
    return scaled / length(scaled);
}

fn reject_vertex(vertex: u32, issue: u32) {
    atomicStore(&draw[1], 0u);
    atomicOr(&draw[5], issue);
    atomicMin(&draw[6], vertex);
}

@compute @workgroup_size(64)
fn pack(@builtin(global_invocation_id) invocation: vec3<u32>) {
    let index = invocation.x;
    if index < arrayLength(&attributes) {
        let value = attributes[index];
        let word = index * VERTEX_WORDS;
        for (var lane = 0u; lane < VERTEX_WORDS; lane += 1u) {
            vertices[word + lane] = base[word + lane];
        }
        for (var slot = 0u; slot < 5u; slot += 1u) {
            let uv = vec2<f32>(bitcast<f32>(base[word + UV_WORDS[slot]]),
                bitcast<f32>(base[word + UV_WORDS[slot] + 1u]));
            if !finite(vec4<f32>(uv, 0.0, 0.0)) { reject_vertex(index, INVALID_UV); }
        }
        for (var lane = 0u; lane < 4u; lane += 1u) {
            let color = bitcast<f32>(base[word + COLOR_WORD + lane]);
            if !(color >= 0.0 && color <= 1.0) { reject_vertex(index, INVALID_COLOR); }
        }
        for (var lane = 0u; lane < 3u; lane += 1u) {
            vertices[word + lane] = bitcast<u32>(value.position[lane]);
            vertices[word + NORMAL_WORD + lane] = bitcast<u32>(value.normal[lane]);
        }
        for (var lane = 0u; lane < 4u; lane += 1u) {
            vertices[word + TANGENT_WORD + lane] = bitcast<u32>(value.tangent[lane]);
        }
        if any(value.status != vec4<u32>(0u)) { reject_vertex(index, DEFORMATION_STATUS); }
        if !finite(value.position) || !finite(value.normal) || !finite(value.tangent) {
            reject_vertex(index, NONFINITE_DEFORMATION);
        }
        if value.tangent.w != 0.0 && abs(value.tangent.w) != 1.0 {
            reject_vertex(index, INVALID_TANGENT);
        }
        let has_tangents = bitcast<f32>(base[word + TANGENT_WORD + 3u]) != 0.0;
        if has_tangents {
            let n = unit(value.normal.xyz);
            let t = unit(value.tangent.xyz);
            if abs(value.tangent.w) != 1.0 || length(n) == 0.0 || length(t - n * dot(n, t)) <= 0.000001 {
                reject_vertex(index, INVALID_TANGENT);
            }
        } else if value.tangent.w != 0.0 { reject_vertex(index, INVALID_TANGENT); }
    }
    if index < arrayLength(&indices) / 3u {
        let a = attributes[indices[index * 3u]].tangent.w;
        let b = attributes[indices[index * 3u + 1u]].tangent.w;
        let c = attributes[indices[index * 3u + 2u]].tangent.w;
        if a != b || b != c {
            atomicStore(&draw[1], 0u);
            atomicOr(&draw[5], TRIANGLE_TANGENT_SIGN);
            atomicMin(&draw[7], index);
        }
    }
}
