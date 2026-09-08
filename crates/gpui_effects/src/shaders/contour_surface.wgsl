fn contour_height(distance: f32, width: f32, depth: f32) -> f32 {
    let inward = clamp(-distance / max(width, 0.001), 0.0, 1.0);
    return depth * inward * (2.0 - inward);
}

fn contour_sample_height(input: EffectInput, uv: vec2<f32>, width: f32, depth: f32) -> f32 {
    let field = sample_effect_second_image(input, uv);
    return select(0.0, contour_height(field.r, width, depth), field.g > 0.5);
}

fn contour_normal(input: EffectInput, width: f32, depth: f32, sample_step: f32) -> vec3<f32> {
    let step = max(sample_step, 0.5);
    let dx = vec2<f32>(step / input.size.x, 0.0);
    let dy = vec2<f32>(0.0, step / input.size.y);
    let left = contour_sample_height(input, input.uv - dx, width, depth);
    let right = contour_sample_height(input, input.uv + dx, width, depth);
    let top = contour_sample_height(input, input.uv - dy, width, depth);
    let bottom = contour_sample_height(input, input.uv + dy, width, depth);
    return normalize(vec3<f32>((left - right) / (2.0 * step), (top - bottom) / (2.0 * step), 1.0));
}
