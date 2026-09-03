fn smootherstep01(value: f32) -> f32 {
    let t = clamp(value, 0.0, 1.0);
    return t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
}

fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> {
    let color = params.slots[0];
    let reveal = params.slots[1].x;
    let completed = params.slots[1].y;
    let softness = max(params.slots[1].z, 0.0001);
    let leading_opacity = params.slots[1].w;

    let span = max(reveal - completed, 0.0001);
    let progress = smootherstep01((input.uv.x - completed) / span);
    let trail_opacity = mix(1.0, leading_opacity, progress);
    let front_fade = 1.0 - smoothstep(reveal - softness, reveal + softness, input.uv.x);
    return vec4<f32>(color.rgb, color.a * trail_opacity * front_fade);
}
