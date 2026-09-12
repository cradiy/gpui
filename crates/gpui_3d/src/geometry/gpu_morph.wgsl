struct Vertex {
    position: vec4<f32>,
    normal: vec4<f32>,
    tangent: vec4<f32>,
    status: vec4<u32>,
}
struct Params {
    vertices: u32,
    targets: u32,
    tangents: u32,
    padding: u32,
}
@group(0) @binding(0) var<storage, read> source: array<Vertex>;
@group(0) @binding(1) var<storage, read> deltas: array<Vertex>;
@group(0) @binding(2) var<storage, read> weights: array<f32>;
@group(0) @binding(3) var<storage, read_write> output: array<Vertex>;
@group(0) @binding(4) var<uniform> params: Params;

fn finite(v: vec3<f32>) -> bool {
    return all(abs(v) <= vec3<f32>(3.402823466e+38));
}
fn unit(v: vec3<f32>) -> vec3<f32> {
    let scale = max(max(abs(v.x), abs(v.y)), abs(v.z));
    if scale == 0.0 { return vec3<f32>(0.0); }
    let scaled = v / scale;
    return scaled / length(scaled);
}

@compute @workgroup_size(64)
fn morph(@builtin(global_invocation_id) id: vec3<u32>) {
    let vertex = id.x;
    if vertex >= params.vertices { return; }
    var result = source[vertex];
    var has_active = false;
    var normals = false;
    for (var target_index = 0u; target_index < params.targets; target_index += 1u) {
        let weight = weights[target_index];
        if weight == 0.0 { continue; }
        has_active = true;
        let delta = deltas[target_index * params.vertices + vertex];
        result.position = vec4<f32>(result.position.xyz + weight * delta.position.xyz, 0.0);
        result.normal = vec4<f32>(result.normal.xyz + weight * delta.normal.xyz, 0.0);
        result.tangent = vec4<f32>(result.tangent.xyz + weight * delta.tangent.xyz, result.tangent.w);
        normals = normals || delta.status.x != 0u;
    }
    if has_active {
        if !finite(result.position.xyz) || !finite(result.normal.xyz) || !finite(result.tangent.xyz) {
            result.status.x = 1u;
        } else {
            if normals { result.normal = vec4<f32>(unit(result.normal.xyz), 0.0); }
            if params.tangents != 0u {
                let n = unit(result.normal.xyz);
                let t = unit(result.tangent.xyz);
                let orthogonal = t - n * dot(n, t);
                if length(n) == 0.0 || length(orthogonal) <= 0.000001 {
                    result.status.x = 2u;
                } else {
                    result.tangent = vec4<f32>(unit(orthogonal), result.tangent.w);
                }
            }
        }
    }
    if !finite(result.normal.xyz) || !finite(result.tangent.xyz) {
        result.status.x = 1u;
    }
    output[vertex] = result;
}
