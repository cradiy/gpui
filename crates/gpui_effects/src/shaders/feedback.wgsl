fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> {
    let source = sample_effect_image(input, input.uv);
    let previous = sample_effect_second_image(input, input.uv);
    let source_alpha = source.a * clamp(params.slots[7].y, 0.0, 1.0);
    var history_alpha = previous.a * clamp(params.slots[7].x, 0.0, 1.0);
    if (history_alpha < params.slots[7].z) { history_alpha = 0.0; }
    let alpha = source_alpha + history_alpha * (1.0 - source_alpha);
    let color = source.rgb * source_alpha + previous.rgb * history_alpha * (1.0 - source_alpha);
    return vec4<f32>(color / max(alpha, 0.000001), alpha);
}
