struct Particle {
    motion: vec4<f32>,
    life: vec4<f32>,
    color: vec4<f32>,
}
struct Params {
    counts: vec4<u32>,
    timing: vec4<f32>,
    viewport: vec4<f32>,
    bounds: vec4<f32>,
    clip: vec4<f32>,
    acceleration: vec4<f32>,
    field: vec4<f32>,
    mask: vec4<u32>,
}
@group(0) @binding(0) var<uniform> params: Params;
