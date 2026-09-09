fn transition_blur(input: EffectInput, radius: f32, second: bool) -> vec4<f32> {
    if (radius <= 0.0) {
        if (second) { return sample_effect_second_image(input, input.uv); }
        return sample_effect_image(input, input.uv);
    }
    var sum = vec4<f32>(0.0);
    var total = 0.0;
    for (var y = -3; y <= 3; y += 1) {
        for (var x = -3; x <= 3; x += 1) {
            let offset = vec2<f32>(f32(x), f32(y));
            let weight = exp(-dot(offset, offset) / 4.5);
            let uv = input.uv + offset * radius / (3.0 * max(input.size, vec2<f32>(1.0)));
            var color: vec4<f32>;
            if (second) { color = sample_effect_second_image(input, uv); }
            else { color = sample_effect_image(input, uv); }
            sum += vec4<f32>(color.rgb * color.a, color.a) * weight;
            total += weight;
        }
    }
    sum /= total;
    return vec4<f32>(sum.rgb / max(sum.a, 0.000001), sum.a);
}

fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> {
    let progress = clamp(params.slots[0].x, 0.0, 1.0);
    if (progress == 0.0) { return sample_effect_image(input, input.uv); }
    if (progress == 1.0) { return sample_effect_second_image(input, input.uv); }
    let radius = max(params.slots[0].y, 0.0);
    return transition_mix(transition_blur(input, radius * progress, false),
        transition_blur(input, radius * (1.0 - progress), true), progress);
}
