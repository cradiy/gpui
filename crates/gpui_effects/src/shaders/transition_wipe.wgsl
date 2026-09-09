fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> {
    let progress = clamp(params.slots[0].x, 0.0, 1.0);
    if (progress == 0.0) { return sample_effect_image(input, input.uv); }
    if (progress == 1.0) { return sample_effect_second_image(input, input.uv); }
    let axis = params.slots[1].xy;
    let coordinate = dot(input.uv, max(axis, vec2<f32>(0.0)))
        + dot(vec2<f32>(1.0) - input.uv, max(-axis, vec2<f32>(0.0)));
    let width = max(params.slots[0].z, 0.001);
    let edge = mix(-width, 1.0 + width, progress);
    let coverage = 1.0 - smoothstep(edge - width, edge + width, coordinate);
    return transition_mix(sample_effect_image(input, input.uv),
        sample_effect_second_image(input, input.uv), coverage);
}
