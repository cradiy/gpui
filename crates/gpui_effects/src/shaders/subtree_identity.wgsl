fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> {
    return sample_effect_image(input, input.uv);
}
