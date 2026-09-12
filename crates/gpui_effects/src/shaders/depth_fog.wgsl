fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> {
    let source = sample_effect_image(input, input.uv);
    let depth = sample_effect_second_image(input, input.uv).r;
    if (depth <= 0.0) {
        return source;
    }
    let start = max(params.slots[0].x, 0.0);
    let end = max(params.slots[0].y, start);
    var amount = select(0.0, 1.0, depth >= start);
    if (end > start) {
        amount = smoothstep(start, end, depth);
    }
    amount *= clamp(params.slots[1].a, 0.0, 1.0);
    return vec4<f32>(mix(source.rgb, max(params.slots[1].rgb, vec3<f32>(0.0)), amount), source.a);
}
