struct Splat { line: vec4<f32>, velocity: vec4<f32>, color: vec4<f32> }
@group(0) @binding(2) var<storage, read> field_b: array<vec4<f32>>;
@group(0) @binding(3) var<storage, read_write> output: array<vec4<f32>>;
@group(0) @binding(4) var<storage, read> splats: array<Splat>;

fn b(p: vec2<i32>) -> vec4<f32> { return field_b[index_at(p)]; }
fn sample_b(p: vec2<f32>) -> vec4<f32> {
    let i = vec2<i32>(floor(p)); let f = fract(p);
    return mix(mix(b(i), b(i + vec2<i32>(1, 0)), f.x),
               mix(b(i + vec2<i32>(0, 1)), b(i + vec2<i32>(1, 1)), f.x), f.y);
}
fn cell_size() -> vec2<f32> { return params.logical.xy / vec2<f32>(params.grid.xy); }
fn weight(p: vec2<f32>, s: Splat) -> f32 {
    let d = s.line.zw - s.line.xy;
    let t = clamp(dot(p - s.line.xy, d) / max(dot(d, d), 0.0001), 0.0, 1.0);
    let distance = p - s.line.xy - t * d;
    return exp(-2.0 * dot(distance, distance) / max(s.velocity.z * s.velocity.z, 0.25));
}
fn boundary(p: vec2<i32>, velocity: vec2<f32>) -> vec2<f32> {
    var v = velocity;
    if p.x == 0 || p.x == i32(params.grid.x) - 1 { v.x = 0.0; }
    if p.y == 0 || p.y == i32(params.grid.y) - 1 { v.y = 0.0; }
    return v * min(1.0, 2000.0 / max(length(v), 0.001));
}

@compute @workgroup_size(8, 8)
fn advect_velocity(@builtin(global_invocation_id) id: vec3<u32>) {
    if any(id.xy >= params.grid.xy) { return; }
    let p = vec2<i32>(id.xy); let h = cell_size();
    let position = (vec2<f32>(id.xy) + 0.5) * h;
    var v = vec2<f32>(0.0);
    if params.grid.w == 0u {
        v = sample_a(vec2<f32>(id.xy) - min(params.step.x, 0.066667) * a(p).xy / h).xy;
        v *= exp(-params.step.y * params.step.x);
    }
    for (var i = 0u; i < params.grid.z; i++) { v += splats[i].velocity.xy * weight(position, splats[i]); }
    output[index_at(p)] = vec4<f32>(boundary(p, v), 0.0, 0.0);
}
@compute @workgroup_size(8, 8)
fn curl(@builtin(global_invocation_id) id: vec3<u32>) {
    if any(id.xy >= params.grid.xy) { return; }
    let p = vec2<i32>(id.xy); let h = cell_size();
    let c = (a(p + vec2<i32>(1, 0)).y - a(p - vec2<i32>(1, 0)).y) / (2.0 * h.x)
          - (a(p + vec2<i32>(0, 1)).x - a(p - vec2<i32>(0, 1)).x) / (2.0 * h.y);
    output[index_at(p)] = vec4<f32>(c, 0.0, 0.0, 0.0);
}
@compute @workgroup_size(8, 8)
fn confine(@builtin(global_invocation_id) id: vec3<u32>) {
    if any(id.xy >= params.grid.xy) { return; }
    let p = vec2<i32>(id.xy); let h = cell_size();
    let g = vec2<f32>(abs(b(p + vec2<i32>(1, 0)).x) - abs(b(p - vec2<i32>(1, 0)).x),
                       abs(b(p + vec2<i32>(0, 1)).x) - abs(b(p - vec2<i32>(0, 1)).x)) / (2.0 * h);
    let n = g / max(length(g), 0.0001);
    let force = vec2<f32>(n.y, -n.x) * b(p).x * params.step.w * min(h.x, h.y);
    output[index_at(p)] = vec4<f32>(boundary(p, a(p).xy + force * min(params.step.x, 0.066667)), 0.0, 0.0);
}
@compute @workgroup_size(8, 8)
fn divergence(@builtin(global_invocation_id) id: vec3<u32>) {
    if any(id.xy >= params.grid.xy) { return; }
    let p = vec2<i32>(id.xy); let h = cell_size();
    let d = (a(p + vec2<i32>(1, 0)).x - a(p - vec2<i32>(1, 0)).x) / (2.0 * h.x)
          + (a(p + vec2<i32>(0, 1)).y - a(p - vec2<i32>(0, 1)).y) / (2.0 * h.y);
    output[index_at(p)] = vec4<f32>(d, 0.0, 0.0, 0.0);
}
@compute @workgroup_size(8, 8)
fn pressure(@builtin(global_invocation_id) id: vec3<u32>) {
    if any(id.xy >= params.grid.xy) { return; }
    let p = vec2<i32>(id.xy); let w = 1.0 / (cell_size() * cell_size());
    let sum = (a(p - vec2<i32>(1, 0)).x + a(p + vec2<i32>(1, 0)).x) * w.x
            + (a(p - vec2<i32>(0, 1)).x + a(p + vec2<i32>(0, 1)).x) * w.y;
    output[index_at(p)] = vec4<f32>((sum - b(p).x) / (2.0 * (w.x + w.y)), 0.0, 0.0, 0.0);
}
@compute @workgroup_size(8, 8)
fn project(@builtin(global_invocation_id) id: vec3<u32>) {
    if any(id.xy >= params.grid.xy) { return; }
    let p = vec2<i32>(id.xy);
    let g = vec2<f32>(b(p + vec2<i32>(1, 0)).x - b(p - vec2<i32>(1, 0)).x,
                       b(p + vec2<i32>(0, 1)).x - b(p - vec2<i32>(0, 1)).x) / (2.0 * cell_size());
    output[index_at(p)] = vec4<f32>(boundary(p, a(p).xy - g), 0.0, 0.0);
}
@compute @workgroup_size(8, 8)
fn advect_dye(@builtin(global_invocation_id) id: vec3<u32>) {
    if any(id.xy >= params.grid.xy) { return; }
    let p = vec2<i32>(id.xy); let h = cell_size();
    var dye = vec4<f32>(0.0);
    if params.grid.w == 0u {
        dye = sample_b(vec2<f32>(id.xy) - min(params.step.x, 0.066667) * a(p).xy / h)
            * exp(-params.step.z * params.step.x);
    }
    let position = (vec2<f32>(id.xy) + 0.5) * h;
    for (var i = 0u; i < params.grid.z; i++) {
        let s = splats[i];
        dye += vec4<f32>(s.color.rgb, 1.0) * s.color.a * s.velocity.w * weight(position, s);
    }
    dye *= min(1.0, 4.0 / max(dye.a, 0.0001));
    if dye.a < 0.000244 { dye = vec4<f32>(0.0); }
    output[index_at(p)] = dye;
}
