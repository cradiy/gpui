fn flow_warp(p: vec2<f32>, time: f32, seed: f32) -> vec2<f32> {
    let phase = seed * 6.283185;
    let broad = vec2<f32>(
        sin(dot(p, vec2<f32>(1.7, 2.3)) + time * 0.17 + phase),
        cos(dot(p, vec2<f32>(-2.1, 1.3)) - time * 0.13 + phase * 0.73),
    );
    let bend = vec2<f32>(
        sin(dot(p, vec2<f32>(-2.6, 1.8)) - time * 0.23 + broad.y * 0.65),
        cos(dot(p, vec2<f32>(1.9, 2.7)) + time * 0.19 + broad.x * 0.65),
    );
    return broad * 0.32 + bend * 0.18;
}

// Sixteen stratified samples are grouped by tone before averaging, so bright
// paper, colored midtones, and shadows are not merged just by sharing a quadrant.
fn flow_image_palette(input: EffectInput) -> array<vec4<f32>, 4> {
    var samples: array<vec4<f32>, 16>;
    var low = 1.0;
    var high = 0.0;
    for (var i = 0u; i < 16u; i += 1u) {
        let column = (f32(i % 4u) + 0.5) / 4.0;
        let uv = vec2<f32>(column, (f32(i / 4u) + column) / 4.0);
        let sample = sample_effect_image(input, uv);
        samples[i] = sample;
        if (sample.a > 0.0) {
            let luminance = dot(sample.rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
            low = min(low, luminance);
            high = max(high, luminance);
        }
    }
    var palette: array<vec4<f32>, 4>;
    for (var i = 0u; i < 16u; i += 1u) {
        let sample = samples[i];
        let luminance = dot(sample.rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
        let band = u32(clamp((luminance - low) / max(high - low, 0.0001) * 4.0, 0.0, 3.0));
        palette[band] += vec4<f32>(sample.rgb * sample.a, sample.a);
    }
    for (var i = 0u; i < 4u; i += 1u) {
        let entry = palette[i];
        palette[i] = vec4<f32>(entry.rgb / max(entry.a, 0.0001), entry.a / 16.0);
    }
    return palette;
}

// A shared RGB scale gives highlights a soft shoulder without shifting hue
// or clipping individual channels into colored bands.
fn flow_tone(color: vec3<f32>, ceiling: f32) -> vec3<f32> {
    let peak = max(max(color.r, color.g), color.b);
    let knee = ceiling * 0.75;
    if (peak <= knee) {
        return color;
    }
    let shoulder = max(ceiling - knee, 0.0001);
    let compressed = knee + shoulder * (1.0 - exp(-(peak - knee) / shoulder));
    return color * compressed / max(peak, 0.0001);
}

fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> {
    let diffusion = clamp(params.slots[0].x / 0.3, 0.0, 1.0);
    let saturation = max(params.slots[0].y, 0.0);
    let brightness = max(params.slots[0].z, 0.0);
    if (brightness == 0.0) {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    let motion = clamp(params.slots[0].w, 0.0, 2.0);
    let flow_scale = clamp(params.slots[1].x, 0.0, 3.0);
    let drift = clamp(params.slots[1].y, 0.0, 2.0);
    let vignette = clamp(params.slots[1].z, 0.0, 1.0);
    let seed = params.slots[1].w;
    let glow = max(params.slots[2].x, 0.0);
    let grain = clamp(params.slots[2].z, 0.0, 2.0);
    let cohesion = clamp(params.slots[2].w, 0.0, 1.0);
    let shadow_level = clamp(params.slots[7].x, 0.0, 1.0);
    let highlight_level = clamp(params.slots[7].y, 0.0, 1.0);
    let neutral_weight = clamp(params.slots[7].z, 0.0, 1.0);
    var palette: array<vec4<f32>, 4>;
    if (params.slots[2].y > 0.5) {
        palette = array<vec4<f32>, 4>(params.slots[3], params.slots[4], params.slots[5], params.slots[6]);
    } else {
        palette = flow_image_palette(input);
    }

    var base = vec3<f32>(0.0);
    var base_weight = 0.0;
    var population = 0.0;
    for (var i = 0u; i < 4u; i += 1u) {
        let source = palette[i].rgb;
        let luminance = dot(source, vec3<f32>(0.2126, 0.7152, 0.0722));
        let peak = max(max(source.r, source.g), source.b);
        let low = min(min(source.r, source.g), source.b);
        let chroma = (peak - low) / max(peak, 0.0001);
        let bright_neutral = smoothstep(0.35, 0.8, luminance)
            * (1.0 - smoothstep(0.06, 0.35, chroma));
        let weight = max(palette[i].a, 0.0) * mix(1.0, neutral_weight, bright_neutral);
        let toned = flow_tone(source, highlight_level);
        // A smooth dark-color preference retains the image's shadow hue.
        // Palette population also contributes to the choice of the base.
        let dark_weight = weight * exp(-4.0 * luminance);
        base += toned * dark_weight;
        base_weight += dark_weight;
        population += weight;
        palette[i] = vec4<f32>(toned, weight);
    }
    if (population <= 0.0 || highlight_level == 0.0) {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    base = base / max(base_weight, 0.0000001) * shadow_level;

    let aspect = input.size.x / max(input.size.y, 1.0);
    let metric = vec2<f32>(max(aspect, 1.0), max(1.0 / max(aspect, 0.001), 1.0));
    let p = (input.uv - vec2<f32>(0.5)) * metric;
    let time = input.time;
    let warp = flow_warp(p, time, seed);
    let point = p + warp * 0.42 * flow_scale * motion;
    // The group drifts gently as one body. Local stirring is independent of
    // drift, so reducing travel does not stop colors from mixing internally.
    let group_center = vec2<f32>(
        sin(time * 0.17 + seed * 2.1),
        cos(time * 0.13 - seed * 1.7),
    ) * 0.08 * motion * drift;
    var color = vec3<f32>(0.0);
    var total = 0.0;
    for (var i = 0u; i < 4u; i += 1u) {
        let phase = f32(i) * 2.399963 + seed * 6.283185;
        let home = vec2<f32>(cos(phase), sin(phase)) * 0.30;
        let orbit = vec2<f32>(
            sin(time * 0.43 + phase) * 0.34 + sin(time * 0.23 - phase * 1.7) * 0.16,
            cos(time * 0.37 + phase * 1.3) * 0.32 + sin(time * 0.29 + phase) * 0.14,
        );
        let free_center = home + orbit * motion * drift;
        let stirring = home + orbit * 0.22 * motion;
        // Smooth radial confinement has no hard boundary where motion can stop
        // or bounce. Adjacent volumes remain close enough to overlap.
        let confined = stirring / sqrt(1.0 + dot(stirring, stirring) / (0.32 * 0.32));
        let center = mix(free_center, group_center + confined, cohesion) * metric;
        let angle = phase + time * 0.21 * motion;
        let delta = point - center;
        let rotated = vec2<f32>(
            delta.x * cos(angle) - delta.y * sin(angle),
            delta.x * sin(angle) + delta.y * cos(angle),
        ) * vec2<f32>(0.82, 1.12);
        // Scale the volumes with their shared footprint to retain color detail
        // rather than averaging the concentrated field into a single flat tint.
        let radius = mix(0.30, 0.72, diffusion) * mix(1.0, 0.85, cohesion)
            * (1.0 + sin(time * 0.31 + phase) * 0.12 * motion);
        let weight = exp(-dot(rotated, rotated) / (radius * radius)) * palette[i].a / population;
        color += palette[i].rgb * weight;
        total += weight;
    }
    // Finite light density exposes the dark base outside the shared color body.
    // Normalization chooses only the local hue, never the brightness of empty space.
    let coverage = 1.0 - exp(-total * (2.0 + glow));
    var rgb = mix(base, color / max(total, 0.0000001), coverage);
    let luminance = dot(rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
    rgb = mix(vec3<f32>(luminance), rgb, saturation) * brightness;
    let edge = input.uv - vec2<f32>(0.5);
    rgb *= 1.0 - vignette * smoothstep(0.08, 0.50, dot(edge, edge));

    // Stationary, monochrome sub-LSB dither breaks quantized gradient bands
    // without introducing colored noise or temporal shimmer.
    let pixel = floor(input.uv * input.size);
    let dither = fract(52.9829189 * fract(dot(pixel, vec2<f32>(0.06711056, 0.00583715)))) - 0.5;
    return vec4<f32>(clamp(rgb + vec3<f32>(dither * grain / 255.0), vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}
