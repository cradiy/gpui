fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> {
    let source = sample_effect_image(input, input.uv);
    let field = sample_effect_second_image(input, input.uv);
    let radius = params.slots[1].x;
    let distance = abs(field.r);
    if (field.g < 0.5 || radius <= 0.0 || distance >= radius) {
        return source;
    }
    let edge = max(params.slots[1].y, 0.0);
    let core = 1.0 - smoothstep(0.0, max(edge, 0.5), distance);
    let spread = distance / radius;
    let halo = exp(-4.0 * spread * spread) * (1.0 - smoothstep(0.7, 1.0, spread));
    let glow_alpha = (1.0 - exp(-params.slots[2].x * (core * 1.8 + halo * 0.45))) * params.slots[0].a;
    let alpha = source.a + glow_alpha * (1.0 - source.a);
    let premultiplied = source.rgb * source.a + params.slots[0].rgb * glow_alpha * (1.0 - source.a);
    return vec4<f32>(premultiplied / max(alpha, 0.000001), alpha);
}
