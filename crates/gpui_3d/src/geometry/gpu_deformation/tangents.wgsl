struct Vertex {
    position: vec4<f32>, normal: vec4<f32>, tangent: vec4<f32>, status: vec4<u32>,
}
struct Frame {
    tangent: vec4<f32>, bitangent: vec4<f32>,
    identity: vec3<u32>, weight: f32, status: vec4<u32>,
}
struct Topology { vertex: u32, zero_uv: u32, uv: vec2<f32> }
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
fn normal_float(v: f32) -> bool {
    let exponent = bitcast<u32>(v) & 0x7f800000u;
    return exponent != 0u && exponent != 0x7f800000u;
}
fn squared_length(v: vec3<f32>) -> f32 { return v.x * v.x + v.y * v.y + v.z * v.z; }
fn supported_edge(v: vec3<f32>, allow_zero: bool) -> bool {
    return normal_float(squared_length(v)) || (allow_zero && all(v == vec3(0.0)));
}
fn supported_derivative(v: vec3<f32>, inherited: bool, determinant: f32) -> bool {
    let squared = squared_length(v);
    if !normal_float(squared) { return inherited && all(v == vec3(0.0)); }
    return inherited || finite(vec4(sqrt(squared) / abs(determinant)));
}
fn orthogonal(v: vec3<f64>, normal: vec3<f64>) -> vec3<f32> {
    let projected = v - wide_dot(v, normal) * normal;
    let length = sqrt(wide_dot(projected, projected));
    if length <= sqrt(wide_dot(v, v)) * f64(0.000001) { return vec3(0.0); }
    return narrow_vector(projected / length);
}
fn normal_basis(normal: vec3<f64>) -> vec3<f32> {
    var axis = vec3<f64>(1.0, 0.0, 0.0);
    if abs(normal.y) < abs(normal.x) && abs(normal.y) <= abs(normal.z) {
        axis = vec3<f64>(0.0, 1.0, 0.0);
    } else if abs(normal.z) < abs(normal.x) && abs(normal.z) < abs(normal.y) {
        axis = vec3<f64>(0.0, 0.0, 1.0);
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
    var normals: array<vec3<f64>, 3>;
    var tangents: array<vec4<f32>, 3>;
    var tags: array<u32, 3>;
    for (var corner = 0u; corner < 3u; corner++) {
        let input = source[topology[first + corner].vertex];
        if any(input.status != vec4(0u)) { reject(first, input.status); return; }
        if !finite(input.position) || !finite(input.normal) {
            reject(first, vec4(1u, 0u, 0u, 0u)); return;
        }
        normals[corner] = wide_unit(wide_vector(input.normal.xyz));
        if all(normals[corner] == vec3(f64(0.0))) {
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
            let tangent = orthogonal(wide_vector(frame.tangent.xyz), normals[corner]);
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
    let opposite = source[c.vertex].position.xyz - source[b.vertex].position.xyz;
    let duv = b.uv - a.uv;
    let euv = c.uv - a.uv;
    let determinant = duv.x * euv.y - duv.y * euv.x;
    let derivative = (e * euv.y - f * duv.y) * select(-1.0, 1.0, determinant >= 0.0);
    let bitangent = f * duv.x - e * euv.x;
    if !finite(vec4(e, 0.0)) || !finite(vec4(f, 0.0)) || !finite(vec4(duv, euv)) ||
        !finite(vec4(determinant)) || !finite(vec4(derivative, 0.0)) ||
        !finite(vec4(opposite, 0.0)) || !finite(vec4(bitangent, 0.0)) {
        reject(first, vec4(1u, 0u, 0u, 0u)); return;
    }
    let wide_e = wide_vector(source[b.vertex].position.xyz) - wide_vector(source[a.vertex].position.xyz);
    let wide_f = wide_vector(source[c.vertex].position.xyz) - wide_vector(source[a.vertex].position.xyz);
    let zero_area = all(cross(wide_e, wide_f) == vec3(f64(0.0)));
    let zero_uv = a.zero_uv != 0u;
    if params.mode == 0u {
        if zero_area {
            reject(first, vec4(4u, 0u, 0u, 0u)); return;
        }
        if zero_uv { reject(first, vec4(2u, 0u, 0u, 0u)); return; }
    }
    let inherited = zero_area || zero_uv;
    if !(normal_float(determinant) || (zero_uv && determinant == 0.0)) ||
        !supported_edge(e, inherited) || !supported_edge(f, inherited) || !supported_edge(opposite, inherited) ||
        !supported_derivative(derivative, inherited, determinant) || !supported_derivative(bitangent, inherited, determinant) {
        reject(first, vec4(5u, 0u, 0u, 0u)); return;
    }
    let wide_duv = wide_vector(vec3(b.uv, 0.0)).xy - wide_vector(vec3(a.uv, 0.0)).xy;
    let wide_euv = wide_vector(vec3(c.uv, 0.0)).xy - wide_vector(vec3(a.uv, 0.0)).xy;
    let wide_determinant = wide_duv.x * wide_euv.y - wide_duv.y * wide_euv.x;
    var repair_sign = select(-1.0, 1.0, wide_determinant >= f64(0.0));
    let repair_derivative = narrow_vector(wide_unit(
        (wide_e * wide_euv.y - wide_f * wide_duv.y) * f64(repair_sign)
    ));
    for (var corner = 0u; corner < 3u; corner++) {
        if tangents[corner].w != 0.0 { repair_sign = tangents[corner].w; break; }
    }
    for (var corner = 0u; corner < 3u; corner++) {
        if tangents[corner].w == 0.0 {
            if params.mode != 2u { reject(first, vec4(2u, 0u, 0u, 0u)); return; }
            var tangent = vec3(0.0);
            if wide_determinant != f64(0.0) {
                tangent = orthogonal(wide_vector(repair_derivative), normals[corner]);
            }
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
