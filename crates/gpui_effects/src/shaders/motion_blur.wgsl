fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> {
    let displacement = params.slots[0].xy;
    let distance = length(displacement);
    if (distance < 0.001) {
        return sample_effect_image(input, input.uv);
    }
    let budget = clamp(u32(params.slots[1].x), 3u, 129u);
    let count = min(budget, max(3u, u32(ceil(distance)) | 1u));
    var accumulated = vec4<f32>(0.0);
    var total_weight = 0.0;
    for (var i = 0u; i < count; i += 1u) {
        let t = f32(i) / f32(count - 1u) - 0.5;
        let weight = exp(-18.0 * t * t);
        let color = sample_effect_image(input, input.uv + displacement * t / input.size);
        accumulated += vec4<f32>(color.rgb * color.a, color.a) * weight;
        total_weight += weight;
    }
    return vec4<f32>(accumulated.rgb / max(accumulated.a, 0.000001), accumulated.a / total_weight);
}
