struct VertexOutput { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> }
@vertex
fn vertex(@builtin(vertex_index) index: u32) -> VertexOutput {
    let uv = vec2<f32>(f32(index & 1u), f32(index >> 1u));
    let p = params.bounds.xy + uv * params.bounds.zw;
    var result: VertexOutput;
    result.position = vec4<f32>(p / params.viewport.xy * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
    result.uv = uv;
    return result;
}
@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    if any(in.position.xy < params.clip.xy) || any(in.position.xy >= params.clip.xy + params.clip.zw) { discard; }
    let dye = max(sample_a(in.uv * vec2<f32>(params.grid.xy) - 0.5), vec4<f32>(0.0));
    let alpha = (1.0 - exp(-dye.a)) * params.viewport.w;
    return vec4<f32>(dye.rgb / max(dye.a, 0.0001) * alpha, alpha);
}
