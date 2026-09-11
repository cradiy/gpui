struct Vertex {
    position: vec4<f32>,
    normal: vec4<f32>,
    tangent: vec4<f32>,
    status: vec4<u32>,
}

@group(0) @binding(0) var<storage, read> vertices: array<Vertex>;
@group(0) @binding(1) var<storage, read_write> bounds: array<atomic<u32>, 8>;

var<workgroup> minima: array<vec3<u32>, 64>;
var<workgroup> maxima: array<vec3<u32>, 64>;
var<workgroup> errors: array<u32, 64>;

fn ordered(bits: vec3<u32>) -> vec3<u32> {
    return select(bits ^ vec3(0x80000000u), ~bits, (bits & vec3(0x80000000u)) != vec3(0u));
}

@compute @workgroup_size(64)
fn reduce_bounds(@builtin(global_invocation_id) id: vec3<u32>, @builtin(local_invocation_index) lane: u32) {
    minima[lane] = vec3(0xff800000u);
    maxima[lane] = vec3(0x007fffffu);
    errors[lane] = 0u;
    if id.x < arrayLength(&vertices) {
        let vertex = vertices[id.x];
        let bits = bitcast<vec3<u32>>(vertex.position.xyz);
        if any(vertex.status != vec4(0u)) || any((bits & vec3(0x7f800000u)) == vec3(0x7f800000u)) {
            errors[lane] = 1u;
        } else {
            let position = ordered(bits);
            minima[lane] = position;
            maxima[lane] = position;
        }
    }
    workgroupBarrier();
    for (var stride = 32u; stride > 0u; stride /= 2u) {
        if lane < stride {
            minima[lane] = min(minima[lane], minima[lane + stride]);
            maxima[lane] = max(maxima[lane], maxima[lane + stride]);
            errors[lane] |= errors[lane + stride];
        }
        workgroupBarrier();
    }
    if lane == 0u {
        for (var axis = 0u; axis < 3u; axis++) {
            atomicMin(&bounds[axis], minima[0][axis]);
            atomicMax(&bounds[axis + 4u], maxima[0][axis]);
        }
        atomicOr(&bounds[3], errors[0]);
    }
}
