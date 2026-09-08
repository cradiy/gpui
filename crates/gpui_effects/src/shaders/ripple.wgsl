fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> {
    let amplitude = max(params.slots[0].x, 0.0);
    let wavelength = max(params.slots[0].y, 1.0);
    let width = max(params.slots[0].z, 1.0);
    let speed = max(params.slots[0].w, 0.0);
    let duration = max(params.slots[1].x, 0.001);
    var offset = vec2<f32>(0.0);
    for (var i = 2u; i < 6u; i += 1u) {
        let wave = params.slots[i];
        if (wave.w < 0.5 || wave.z <= 0.0 || wave.z >= duration) { continue; }
        let delta = (input.uv - wave.xy) * input.size;
        let distance = length(delta);
        let radius = speed * wave.z;
        let travel = distance - radius;
        let packet = 1.0 - smoothstep(0.0, 1.0, abs(travel) / width);
        let attack = smoothstep(0.0, min(0.1, duration * 0.15), wave.z);
        let release = 1.0 - smoothstep(0.35, 1.0, wave.z / duration);
        let spread = inverseSqrt(1.0 + radius / (wavelength * 2.0));
        let center_fade = smoothstep(0.0, wavelength * 0.15, distance);
        let phase = travel * 6.2831853 / wavelength;
        offset += delta / max(distance, 0.001) * sin(phase)
            * amplitude * packet * attack * release * spread * center_fade;
    }
    offset *= min(1.0, amplitude / max(length(offset), 0.001));
    let to_edge = min(input.uv, vec2<f32>(1.0) - input.uv) * input.size;
    let edge = smoothstep(0.0, max(params.slots[6].x, 1.0), min(to_edge.x, to_edge.y));
    let uv = input.uv + offset * edge / max(input.size, vec2<f32>(1.0));
    let inset = vec2<f32>(0.5) / max(input.size, vec2<f32>(1.0));
    return sample_effect_image(input, clamp(uv, inset, vec2<f32>(1.0) - inset));
}
