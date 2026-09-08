fn deformation_weight(position: vec2<f32>, center: vec2<f32>, radius: f32) -> f32 {
    let delta = (position - center) / radius;
    let falloff = max(1.0 - dot(delta, delta), 0.0);
    return falloff * falloff * falloff;
}

fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> {
    let radius = params.slots[1].z;
    var offset = params.slots[1].xy;
    let distance = length(offset);
    if (radius <= 0.0 || distance == 0.0) {
        return sample_effect_image(input, input.uv);
    }
    offset *= min(1.0, 0.35 * radius / distance);
    let center = params.slots[0].xy * input.size;
    let destination = input.uv * input.size;
    if (length(destination - center) >= radius) {
        return sample_effect_image(input, input.uv);
    }
    var source = destination;
    for (var iteration = 0u; iteration < 24u; iteration += 1u) {
        source = destination - offset * deformation_weight(source, center, radius);
    }
    let uv = source / input.size;
    if (any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0))) {
        return vec4<f32>(0.0);
    }
    return sample_effect_image(input, uv);
}
