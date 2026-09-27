fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> {
    let region = params.slots[0];
    let source = params.slots[4];
    let distance = glass_contour(input.position - region.xy, region.zw,
        params.slots[1], params.slots[2]);
    let coverage = 1.0 - smoothstep(-0.5, 0.5, distance);
    let covered_color = params.slots[3];
    let original_weight = 1.0 - coverage;
    let target_weight = coverage * clamp(covered_color.a, 0.0, 1.0);
    let alpha = original_weight + target_weight;
    let color = (source.rgb * original_weight + covered_color.rgb * target_weight) / max(alpha, 0.0001);
    return vec4<f32>(color, source.a * alpha);
}
