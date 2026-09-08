fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> {
    let source = sample_effect_image(input, input.uv);
    let field = sample_effect_second_image(input, input.uv);
    let projection = params.slots[1];
    let extent = length(projection.xy);
    if (source.a >= 1.0 || params.slots[0].a <= 0.0 || field.g < 0.5
        || field.r > extent + max(projection.z, projection.w) + 1.0) {
        return source;
    }
    var coverage = 0.0;
    for (var i = 0u; i <= 64u; i += 1u) {
        let t = f32(i) / 64.0;
        let uv = input.uv - projection.xy * t / input.size;
        if (any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0))) { continue; }
        let sample = sample_effect_second_image(input, uv);
        if (sample.g < 0.5) { continue; }
        let softness = max(mix(projection.z, projection.w, t), 0.75);
        let edge = 1.0 - smoothstep(-softness, softness, sample.r);
        let fade = 1.0 - smoothstep(0.65, 1.0, t);
        coverage = max(coverage, edge * fade);
    }
    let shadow_alpha = coverage * params.slots[0].a;
    let alpha = source.a + shadow_alpha * (1.0 - source.a);
    let color = source.rgb * source.a + params.slots[0].rgb * shadow_alpha * (1.0 - source.a);
    return vec4<f32>(color / max(alpha, 0.000001), alpha);
}
