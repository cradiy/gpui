fn foil_shade(input: EffectInput, params: EffectParams, base: vec4<f32>) -> vec4<f32> {
    let surface = params.slots[0];
    let light = params.slots[1];
    let illumination = params.slots[2];
    let optics = params.slots[3];
    let axis = params.slots[5].xy;
    let p = (input.uv - vec2<f32>(0.5)) * input.size / max(min(input.size.x, input.size.y), 1.0);
    let grain_phase = dot(p, axis) * 720.0;
    let grain_filter = 1.0 - smoothstep(0.6, 3.0, fwidth(grain_phase));
    let grain = sin(grain_phase) * grain_filter * optics.z;
    let normal = normalize(vec3<f32>(surface.xy + p * surface.z + axis * grain * 0.025, 1.0));
    let toward_light = normalize(light.xyz);
    let toward_eye = normalize(vec3<f32>(-p * 0.25, 2.5));
    let half_vector = toward_light + toward_eye;
    let half_direction = half_vector / max(length(half_vector), 0.0001);
    let facing = max(dot(normal, toward_light), 0.0);
    let gloss = max(dot(normal, half_direction), 0.0);
    let exponent = mix(160.0, 10.0, surface.w * surface.w);
    let highlight = pow(gloss, exponent);
    let sheen = pow(gloss, 8.0);
    let phase = dot(p, axis) * optics.y * 1.4
        + dot(normal, toward_light) * 2.6 + dot(surface.xy, vec2<f32>(2.3, 1.7));
    let spectral_filter = 1.0 - smoothstep(0.15, 0.6, fwidth(phase));
    let spectrum = vec3<f32>(0.55) + 0.45 * cos(6.2831853 * (vec3<f32>(phase) + vec3<f32>(0.0, 0.33, 0.67)));
    let reflection = mix(vec3<f32>(0.92, 0.96, 1.0), spectrum, optics.x * spectral_filter);
    let diffuse = base.rgb * (illumination.w + facing * light.w * 0.3);
    let reflected = reflection * illumination.rgb * light.w * (sheen * 0.28 + highlight * 0.8) * facing;
    let shaded = clamp(diffuse + reflected + grain * 0.012, vec3<f32>(0.0), vec3<f32>(1.0));
    return vec4<f32>(mix(base.rgb, shaded, optics.w), base.a);
}
