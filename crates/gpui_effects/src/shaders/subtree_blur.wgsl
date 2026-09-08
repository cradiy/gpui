fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> {
    let radius = max(params.slots[0].x, 0.0);
    if (radius <= 0.0) {
        return sample_effect_image(input, input.uv);
    }
    var accumulated = vec4<f32>(0.0);
    var weight_sum = 0.0;
    for (var y = -3; y <= 3; y += 1) {
        for (var x = -3; x <= 3; x += 1) {
            let offset = vec2<f32>(f32(x), f32(y));
            let weight = exp(-dot(offset, offset) / 4.0);
            let color = sample_effect_image(input, input.uv + offset * (radius / 3.0) / input.size);
            accumulated += vec4<f32>(color.rgb * color.a, color.a) * weight;
            weight_sum += weight;
        }
    }
    return vec4<f32>(accumulated.rgb / max(accumulated.a, 0.000001), accumulated.a / weight_sum);
}
