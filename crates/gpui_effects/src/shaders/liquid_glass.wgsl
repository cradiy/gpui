fn glass_distance(p: vec2<f32>, size: vec2<f32>, radii: vec4<f32>) -> f32 {
    var radius = select(radii.w, radii.x, p.y < 0.0);
    if (p.x >= 0.0) {
        radius = select(radii.z, radii.y, p.y < 0.0);
    }
    let half_size = size * 0.5;
    radius = clamp(radius, 0.0, min(half_size.x, half_size.y));
    let corner = abs(p) - half_size + radius;
    return length(max(corner, vec2<f32>(0.0)))
        + min(max(corner.x, corner.y), 0.0) - radius;
}

fn glass_sample(input: BackdropInput, offset: vec2<f32>, clarity: f32) -> vec3<f32> {
    return mix(
        sample_blurred_backdrop(input, offset).rgb,
        sample_raw_backdrop(input, offset).rgb,
        clarity,
    );
}

fn glass_border_reflection(sampled: vec3<f32>, lift: f32) -> vec3<f32> {
    let rgb = clamp(sampled, vec3<f32>(0.0), vec3<f32>(1.0));
    let peak = max(max(rgb.r, rgb.g), rgb.b);
    if (peak <= 0.0) {
        return rgb;
    }
    // Scale all channels together. Even maximum lift retains the sampled RGB
    // ratios instead of replacing a colored sample with white light.
    return rgb * (mix(peak, 1.0, lift) / peak);
}

fn backdrop_effect(input: BackdropInput, params: BackdropParams) -> vec4<f32> {
    let optics = params.slots[0];
    let tint = params.slots[1];
    let surface = params.slots[2];
    let light = params.slots[3];
    let radii = params.slots[4];
    let edge_tint = params.slots[5];
    let p = (input.uv - vec2<f32>(0.5)) * input.size;
    let distance = glass_distance(p, input.size, radii);
    let inside = max(-distance, 0.0);

    // The distance gradient follows straight edges and each individual corner.
    let gradient = vec2<f32>(
        glass_distance(p + vec2<f32>(0.5, 0.0), input.size, radii)
            - glass_distance(p - vec2<f32>(0.5, 0.0), input.size, radii),
        glass_distance(p + vec2<f32>(0.0, 0.5), input.size, radii)
            - glass_distance(p - vec2<f32>(0.0, 0.5), input.size, radii),
    );
    let normal = gradient / max(length(gradient), 0.0001);
    let thickness = min(optics.w, min(input.size.x, input.size.y) * 0.45);
    var curvature = 0.0;
    if (thickness > 0.0) {
        let edge = 1.0 - clamp(inside / thickness, 0.0, 1.0);
        curvature = edge * edge;
    }
    // The quadratic profile has slope 2 / thickness at the rim. Keeping
    // displacement below half the thickness preserves ordering on straight edges,
    // including the extra red-channel displacement from dispersion.
    let displacement = -normal * min(optics.z, thickness * 0.45) * curvature;
    var color = glass_sample(input, displacement, surface.w);
    if (surface.z > 0.0) {
        color.r = glass_sample(input, displacement * (1.0 + surface.z), surface.w).r;
        color.b = glass_sample(input, displacement * (1.0 - surface.z), surface.w).b;
    }

    let luminance = dot(color, vec3<f32>(0.2126, 0.7152, 0.0722));
    color = mix(vec3<f32>(luminance), color, optics.x) * optics.y;
    color = mix(color, tint.rgb, clamp(tint.a, 0.0, 1.0));

    var direction = vec2<f32>(-0.6, -0.8);
    if (length(light.xy) > 0.0001) {
        direction = normalize(light.xy);
    }
    let facing = dot(normal, direction);
    let reflection = pow(max(facing, 0.0), 3.0);
    let opposite = pow(max(-facing, 0.0), 2.0);
    var tinted_stroke = 0.0;
    if (edge_tint.x > 0.0 && edge_tint.y > 0.0) {
        let width = min(edge_tint.y, min(input.size.x, input.size.y) * 0.5);
        tinted_stroke = 1.0 - smoothstep(width - 0.5, width + 0.5, inside);
    }
    var rim = 0.0;
    if (light.z > 0.0) {
        rim = 1.0 - smoothstep(light.z, light.z + 1.0, inside);
    }
    // Reflection follows the fine rim, not a broad raised bevel.
    let highlight = surface.x * rim * (0.12 + reflection * 0.88)
        * (1.0 - edge_tint.x * tinted_stroke);
    color *= 1.0 - surface.y * opposite * curvature;
    color = mix(color, vec3<f32>(1.0), highlight);

    if (edge_tint.x > 0.0 && edge_tint.y > 0.0) {
        // Gather RGB together from the surrounding scene, not from separated
        // color channels. Sample relative to the silhouette, independently of blur.
        let outward = normal * (inside + edge_tint.z);
        let tangent = vec2<f32>(-normal.y, normal.x) * edge_tint.z * 0.5;
        var gathered = (
            sample_raw_backdrop(input, outward).rgb * 2.0
            + sample_raw_backdrop(input, outward + tangent).rgb
            + sample_raw_backdrop(input, outward - tangent).rgb
        ) * 0.25;
        let gathered_luminance = dot(gathered, vec3<f32>(0.2126, 0.7152, 0.0722));
        gathered = mix(vec3<f32>(gathered_luminance), gathered, optics.x) * optics.y;
        // Direction affects only brightness. Border opacity does not fade back
        // toward the white rim on the side opposite the light.
        let illumination = 1.0 - surface.x * (1.0 - reflection);
        let reflection_color = glass_border_reflection(gathered, edge_tint.w * illumination);
        color = mix(color, reflection_color, edge_tint.x * tinted_stroke);
    }
    return vec4<f32>(clamp(color, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}
