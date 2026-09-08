fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> {
    let step = params.slots[2].xy * params.slots[1].x / max(input.size, vec2<f32>(1.0));
    let taps = i32(clamp(ceil(length(step * input.image_size)), 1.0, 64.0));
    var total = vec4<f32>(0.0);
    var weights = 0.0;
    for (var i = -taps; i <= taps; i += 1) {
        let distance = f32(i) / f32(taps);
        let squared_distance = distance * distance;
        let core = 6.5 * exp(-50.0 * squared_distance);
        let halo = 1.05 * exp(-4.5 * squared_distance);
        let weight = core + halo;
        let color = sample_effect_image(input, input.uv + step * distance);
        total += vec4<f32>(color.rgb * color.a, color.a) * weight;
        weights += weight;
    }
    total /= weights;
    return vec4<f32>(total.rgb / max(total.a, 0.000001), total.a);
}
