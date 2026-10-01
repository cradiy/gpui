@group(1) @binding(2) var s_backdrop: sampler;

fn sample_backdrop(uv: vec2<f32>) -> vec4<f32> {
    return textureSample(t_backdrop, s_backdrop, uv);
}
