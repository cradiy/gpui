fn sample_effect_mask(input: EffectInput, uv: vec2<f32>) -> f32 {
    return sample_effect_image(input, uv).r;
}

fn effect_mask_coverage(input: EffectInput) -> f32 {
    return sample_effect_mask(input, input.mask_uv);
}
