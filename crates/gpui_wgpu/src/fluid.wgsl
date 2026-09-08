struct Params {
    grid: vec4<u32>,
    step: vec4<f32>,
    viewport: vec4<f32>,
    bounds: vec4<f32>,
    clip: vec4<f32>,
    logical: vec4<f32>,
}
@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> field_a: array<vec4<f32>>;

fn index_at(p: vec2<i32>) -> u32 {
    let q = clamp(p, vec2<i32>(0), vec2<i32>(params.grid.xy) - 1);
    return u32(q.y) * params.grid.x + u32(q.x);
}
fn a(p: vec2<i32>) -> vec4<f32> { return field_a[index_at(p)]; }
fn sample_a(p: vec2<f32>) -> vec4<f32> {
    let i = vec2<i32>(floor(p));
    let f = fract(p);
    return mix(mix(a(i), a(i + vec2<i32>(1, 0)), f.x),
               mix(a(i + vec2<i32>(0, 1)), a(i + vec2<i32>(1, 1)), f.x), f.y);
}
