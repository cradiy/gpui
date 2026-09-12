fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> {
    let extent = params.slots[0].x;
    let edge = u32(params.slots[1].x);
    let position = input.uv * input.size;
    let distances = vec4<f32>(position.y, input.size.y - position.y,
        position.x, input.size.x - position.x);
    let strength = 1.0 - smoothstep(0.0, max(extent, 0.0001), distances[edge]);
    let radius = params.slots[0].y * strength;
    if (extent <= 0.0 || radius < 0.01) {
        return sample_effect_image(input, input.uv);
    }
    let axis = params.slots[1].yz;
    let half_pixel = vec2<f32>(0.5) / input.image_size;
    var accumulated = vec4<f32>(0.0);
    var weight_sum = 0.0;
    for (var tap = -24; tap <= 24; tap += 1) {
        let position_in_kernel = f32(tap) / 24.0;
        let offset = position_in_kernel * radius;
        let weight = exp(-4.5 * position_in_kernel * position_in_kernel);
        let uv = clamp(input.uv + axis * offset / input.size,
            half_pixel, vec2<f32>(1.0) - half_pixel);
        let color = sample_effect_image(input, uv);
        accumulated += vec4<f32>(color.rgb * color.a, color.a) * weight;
        weight_sum += weight;
    }
    return vec4<f32>(accumulated.rgb / max(accumulated.a, 0.000001),
        accumulated.a / weight_sum);
}
