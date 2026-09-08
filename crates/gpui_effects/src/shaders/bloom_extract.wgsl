fn bloom_highlight(color: vec4<f32>, threshold: f32, knee: f32) -> vec4<f32> {
    let brightness = clamp(max(max(color.r, color.g), color.b), 0.0, 1.0);
    var contribution = select(0.0, 1.0, brightness > threshold);
    if (knee > 0.0) {
        contribution = smoothstep(threshold - knee, threshold + knee, brightness);
    }
    let alpha = color.a * contribution;
    return vec4<f32>(color.rgb * alpha, alpha);
}

fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> {
    let step = params.slots[2].zw / max(input.size, vec2<f32>(1.0));
    var total = vec4<f32>(0.0);
    for (var y = 0; y < 4; y += 1) {
        for (var x = 0; x < 4; x += 1) {
            let offset = (vec2<f32>(f32(x), f32(y)) / 4.0 - vec2<f32>(0.375)) * step;
            total += bloom_highlight(sample_effect_image(input, input.uv + offset), params.slots[0].x, params.slots[0].y);
        }
    }
    total /= 16.0;
    return vec4<f32>(total.rgb / max(total.a, 0.000001), total.a);
}
