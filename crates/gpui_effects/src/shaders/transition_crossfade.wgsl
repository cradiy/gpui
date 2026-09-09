fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> {
    return transition_mix(sample_effect_image(input, input.uv),
        sample_effect_second_image(input, input.uv), clamp(params.slots[0].x, 0.0, 1.0));
}
