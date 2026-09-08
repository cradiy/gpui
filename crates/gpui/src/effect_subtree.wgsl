@group(1) @binding(1) var t_effect_image: texture_2d<f32>;

fn load_subtree_pixel(input: EffectInput, pixel: vec2<i32>) -> vec4<f32> {
    let position = vec2<f32>(pixel) + vec2<f32>(0.5);
    if (any(position < input.image_origin)
        || any(position >= input.image_origin + input.image_size)
        || any(pixel < vec2<i32>(0))
        || any(pixel >= vec2<i32>(textureDimensions(t_effect_image)))) {
        return vec4<f32>(0.0);
    }
    return textureLoad(t_effect_image, pixel, 0);
}

fn sample_effect_image(input: EffectInput, uv: vec2<f32>) -> vec4<f32> {
    let pixel = input.image_origin + uv * input.image_size - vec2<f32>(0.5);
    let low = vec2<i32>(floor(pixel));
    let factor = fract(pixel);
    let color = mix(
        mix(load_subtree_pixel(input, low), load_subtree_pixel(input, low + vec2<i32>(1, 0)), factor.x),
        mix(load_subtree_pixel(input, low + vec2<i32>(0, 1)), load_subtree_pixel(input, low + vec2<i32>(1, 1)), factor.x),
        factor.y,
    );
    return vec4<f32>(color.rgb / max(color.a, 0.000001), color.a);
}

fn sample_effect_image_cover(input: EffectInput, uv: vec2<f32>) -> vec4<f32> {
    return sample_effect_image(input, uv);
}
