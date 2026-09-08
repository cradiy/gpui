fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> {
    let amplitude = max(params.slots[0].x, 0.0);
    let wavelength = max(params.slots[0].y, 1.0);
    let position = (input.uv - vec2<f32>(0.5)) * input.size;
    let phase = input.time * params.slots[1].x;
    let displacement = vec2<f32>(
        sin(position.y * 6.2831853 / wavelength - phase),
        sin(position.x * 6.2831853 / wavelength + phase * 0.83),
    ) * amplitude / input.size;
    return sample_effect_image(input, input.uv + displacement);
}
