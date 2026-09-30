fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> {
    let position = vec3<f32>(input.uv * input.size, 1.0);
    let source = vec2<f32>(dot(params.slots[0].xyz, position), dot(params.slots[1].xyz, position));
    if (any(source < vec2<f32>(0.0)) || any(source >= input.size)) {
        return vec4<f32>(0.0);
    }
    return sample_effect_image(input, source / max(input.size, vec2<f32>(0.0001)));
}
