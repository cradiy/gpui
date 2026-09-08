fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> {
    let amplitude = params.slots[0].xy;
    if (all(amplitude == vec2<f32>(0.0))) { return sample_effect_image(input, input.uv); }
    let uv = input.uv * params.slots[1].xy + params.slots[1].zw + input.time * params.slots[2].xy;
    var map: vec4<f32>;
    if (params.slots[2].z > 0.5) {
        map = sample_effect_second_image_repeat(input, uv);
    } else {
        map = sample_effect_second_image(input, uv);
    }
    let direction = clamp((map.rg * 255.0 - vec2<f32>(128.0)) / 127.0, vec2<f32>(-1.0), vec2<f32>(1.0));
    let strength = map.a * displacement_mask(input);
    var source_uv = input.uv + direction * amplitude * strength / max(input.size, vec2<f32>(1.0));
    if (params.slots[2].w > 0.5) {
        let half_texel = vec2<f32>(0.5) / max(input.image_size, vec2<f32>(1.0));
        source_uv = clamp(source_uv, half_texel, vec2<f32>(1.0) - half_texel);
    }
    return sample_effect_image(input, source_uv);
}
