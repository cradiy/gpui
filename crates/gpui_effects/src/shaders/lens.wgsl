fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> {
    let radius = params.slots[1].x;
    let zoom = clamp(params.slots[0].z, 0.5, 3.0);
    let delta = (input.uv - params.slots[0].xy) * input.size;
    let distance = length(delta);
    if (radius <= 0.0 || distance >= radius || zoom == 1.0) {
        return sample_effect_image(input, input.uv);
    }
    let normalized = distance / radius;
    let power = 3.0 + clamp(params.slots[0].w, 0.0, 1.0) * 3.0;
    let weight = pow(max(1.0 - normalized * normalized, 0.0), power);
    let to_edge = min(input.uv, vec2<f32>(1.0) - input.uv) * input.size;
    let edge = smoothstep(0.0, max(params.slots[1].y, 1.0), min(to_edge.x, to_edge.y));
    let scale = exp(-log(zoom) * weight * edge);
    let uv = input.uv + delta * (scale - 1.0) / max(input.size, vec2<f32>(1.0));
    let inset = vec2<f32>(0.5) / max(input.size, vec2<f32>(1.0));
    return sample_effect_image(input, clamp(uv, inset, vec2<f32>(1.0) - inset));
}
