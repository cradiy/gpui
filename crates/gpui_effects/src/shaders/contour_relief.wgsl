fn contour_shade(base: vec3<f32>, normal: vec3<f32>, params: EffectParams) -> vec3<f32> {
    let light = params.slots[1];
    let material = params.slots[2];
    let color = params.slots[3].rgb;
    let facing = max(dot(normal, light.xyz), 0.0);
    let half_vector = light.xyz + vec3<f32>(0.0, 0.0, 1.0);
    let half_direction = half_vector / max(length(half_vector), 0.0001);
    let exponent = mix(160.0, 8.0, material.x * material.x);
    let highlight = pow(max(dot(normal, half_direction), 0.0), exponent) * material.y * facing;
    return clamp(base * (vec3<f32>(material.w) + color * facing * light.w * 0.7) + color * highlight * light.w, vec3<f32>(0.0), vec3<f32>(1.0));
}

fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> {
    let source = sample_effect_image(input, input.uv);
    let field = sample_effect_second_image(input, input.uv);
    let surface = params.slots[0];
    if (source.a == 0.0 || field.g < 0.5 || surface.x <= 0.0 || surface.y == 0.0 || params.slots[2].z == 0.0) {
        return source;
    }
    let normal = contour_normal(input, surface.x, surface.y, surface.z);
    let shaded = contour_shade(source.rgb, normal, params);
    return vec4<f32>(mix(source.rgb, shaded, params.slots[2].z), source.a);
}
