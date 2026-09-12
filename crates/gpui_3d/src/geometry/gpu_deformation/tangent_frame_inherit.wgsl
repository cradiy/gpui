struct Frame {
    tangent: vec4<f32>, bitangent: vec4<f32>,
    identity: vec3<u32>, weight: f32, status: vec4<u32>,
}
struct Record { a: vec4<u32>, b: vec4<u32>, c: vec4<u32>, status: vec4<u32> }
struct Params { corners: u32, distance: u32, width: u32, capacity: u32 }
@group(0) @binding(0) var<storage, read> source: array<Frame>;
@group(0) @binding(1) var<storage, read> donors: array<u32>;
@group(0) @binding(2) var<storage, read> weld: array<Record>;
@group(0) @binding(3) var<storage, read_write> output: array<Frame>;
@group(0) @binding(4) var<uniform> params: Params;

@compute @workgroup_size(64)
fn inherit(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= params.corners { return; }
    var frame = source[id.x];
    if frame.identity.y == 0xffffffffu && all(frame.status == vec4(0u)) {
        let first = id.x / 3u * 3u;
        let p0 = bitcast<vec3<f32>>(weld[first].a.xyz);
        let p1 = bitcast<vec3<f32>>(weld[first + 1u].a.xyz);
        let p2 = bitcast<vec3<f32>>(weld[first + 2u].a.xyz);
        if all(p0 == p1) || all(p1 == p2) || all(p2 == p0) {
            let donor = donors[weld[id.x].c.z];
            if donor != 0xffffffffu { frame = source[donor]; }
        }
    }
    output[id.x] = frame;
}
