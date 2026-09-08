fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> {
    let source = sample_effect_image(input, input.uv);
    let glow = sample_effect_second_image(input, input.uv);
    let intensity = max(params.slots[0].z, 0.0);
    let base = clamp(source.rgb, vec3<f32>(0.0), vec3<f32>(1.0)) * source.a;
    let headroom = 1.0 - 0.75 * max(max(base.r, base.g), base.b);
    let halo_alpha = 1.0 - exp(-max(glow.a, 0.0) * intensity * headroom);
    let alpha = source.a + halo_alpha * (1.0 - source.a);
    let light = clamp(glow.rgb, vec3<f32>(0.0), vec3<f32>(1.0)) * halo_alpha;
    let rgb = base + light * (vec3<f32>(1.0) - base);
    return vec4<f32>(rgb / max(alpha, 0.000001), alpha);
}
