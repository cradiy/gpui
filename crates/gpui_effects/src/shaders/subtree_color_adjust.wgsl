fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> {
    let color = sample_effect_image(input, input.uv);
    let controls = max(params.slots[0].xyz, vec3<f32>(0.0));
    let luminance = dot(color.rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
    let saturated = mix(vec3<f32>(luminance), color.rgb, controls.x);
    let adjusted = ((saturated - vec3<f32>(0.5)) * controls.y + vec3<f32>(0.5)) * controls.z;
    return vec4<f32>(clamp(adjusted, vec3<f32>(0.0), vec3<f32>(1.0)), color.a);
}
