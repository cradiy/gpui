fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> {
    let source = sample_effect_image(input, input.uv);
    var radiance = max(source.rgb, vec3<f32>(0.0)) * exp2(clamp(params.slots[0].x, -32.0, 32.0));
    if (params.slots[0].y > 0.5) {
        radiance = vec3<f32>(1.0) - vec3<f32>(1.0) / (vec3<f32>(1.0) + radiance);
    }
    let linear = clamp(radiance, vec3<f32>(0.0), vec3<f32>(1.0));
    let encoded = select(12.92 * linear, 1.055 * pow(linear, vec3<f32>(1.0 / 2.4)) - 0.055,
        linear > vec3<f32>(0.0031308));
    return vec4<f32>(encoded, source.a);
}
