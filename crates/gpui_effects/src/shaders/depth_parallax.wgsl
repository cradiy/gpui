fn parallax_depth(input: EffectInput, uv: vec2<f32>, focus: f32, inverted: bool) -> f32 {
    let dimensions = max(input.second_image_size, vec2<f32>(1.0));
    let map_uv = (vec2<f32>(0.5) + clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0)) * (dimensions - 1.0)) / dimensions;
    let sample = sample_effect_second_image(input, map_uv);
    let depth = select(sample.r, 1.0 - sample.r, inverted);
    return mix(focus, depth, sample.a);
}

fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> {
    let strength = params.slots[0].z;
    let focus = params.slots[0].w;
    let inverted = params.slots[1].x > 0.5;
    let image_aspect = input.image_size.x / max(input.image_size.y, 1.0);
    let view_aspect = input.size.x / max(input.size.y, 1.0);
    let cover = vec2<f32>(min(view_aspect / image_aspect, 1.0), min(image_aspect / view_aspect, 1.0));
    let zoom = 1.0 + 2.0 * strength * max(focus, 1.0 - focus);
    let base = vec2<f32>(0.5) + (input.uv - vec2<f32>(0.5)) * cover / zoom;
    let shift = params.slots[0].xy * strength * cover / zoom;
    if (all(shift == vec2<f32>(0.0))) { return sample_effect_image(input, base); }

    let steps = clamp(u32(params.slots[1].y), 8u, 64u);
    var front = 1.0;
    var back = 0.0;
    for (var i = 0u; i <= steps; i += 1u) {
        let z = 1.0 - f32(i) / f32(steps);
        let uv = base - shift * (z - focus);
        if (parallax_depth(input, uv, focus, inverted) >= z) {
            back = z;
            break;
        }
        front = z;
    }
    for (var i = 0u; i < 5u; i += 1u) {
        let z = (front + back) * 0.5;
        if (parallax_depth(input, base - shift * (z - focus), focus, inverted) >= z) {
            back = z;
        } else {
            front = z;
        }
    }
    return sample_effect_image(input, base - shift * ((front + back) * 0.5 - focus));
}
