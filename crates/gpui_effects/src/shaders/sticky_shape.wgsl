fn sticky_rounded_rect(position: vec2<f32>, half_size: vec2<f32>, radius: f32) -> f32 {
    let safe_half = max(half_size, vec2<f32>(0.5));
    let safe_radius = clamp(radius, 0.0, min(safe_half.x, safe_half.y));
    let corner = abs(position) - safe_half + vec2<f32>(safe_radius);
    return length(max(corner, vec2<f32>(0.0)))
        + min(max(corner.x, corner.y), 0.0)
        - safe_radius;
}

fn sticky_smooth_union(first: f32, second: f32, radius: f32) -> f32 {
    let safe_radius = max(radius, 0.001);
    let h = max(safe_radius - abs(first - second), 0.0) / safe_radius;
    return min(first, second) - h * h * safe_radius * 0.25;
}

fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> {
    let anchor = params.slots[0];
    let target_shape = params.slots[1];
    let shape = params.slots[2];
    let fill = params.slots[3];
    let position = input.uv * input.size;

    let anchor_sdf = sticky_rounded_rect(
        position - anchor.xy,
        anchor.zw,
        shape.x,
    );
    let target_sdf = sticky_rounded_rect(
        position - target_shape.xy,
        target_shape.zw,
        shape.y,
    );

    let axis = target_shape.xy - anchor.xy;
    let distance = max(length(axis), 0.001);
    let direction = axis / distance;
    let relative = position - anchor.xy;
    let along = dot(relative, direction);
    let progress = clamp(along / distance, 0.0, 1.0);
    let perpendicular = abs(relative.x * direction.y - relative.y * direction.x);

    let anchor_radius = min(anchor.z, anchor.w);
    let target_radius = min(target_shape.z, target_shape.w);
    let start_radius = anchor_radius * mix(0.46, 0.30, shape.z);
    let end_radius = target_radius * mix(0.72, 0.58, shape.z);
    let bridge_radius = max(
        mix(start_radius, end_radius, progress),
        0.72 * shape.w,
    );
    let bridge_sdf = max(
        perpendicular - bridge_radius,
        max(-along, along - distance),
    );

    let joined = sticky_smooth_union(anchor_sdf, bridge_sdf, 1.4 * shape.w);
    let sdf = sticky_smooth_union(joined, target_sdf, 1.4 * shape.w);
    let coverage = 1.0 - smoothstep(-0.72, 0.72, sdf);
    return vec4<f32>(fill.rgb, fill.a * coverage);
}
