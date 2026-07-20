@group(1) @binding(4) var t_effect_third_image: texture_2d<f32>;
@group(1) @binding(5) var t_effect_fourth_image: texture_2d<f32>;

fn effect_third_image_texel(texel: vec2<i32>) -> vec4<f32> {
    let dimensions = vec2<i32>(textureDimensions(t_effect_third_image));
    return textureLoad(t_effect_third_image, clamp(texel, vec2<i32>(0), dimensions - 1), 0);
}

fn effect_fourth_image_texel(texel: vec2<i32>) -> vec4<f32> {
    let dimensions = vec2<i32>(textureDimensions(t_effect_fourth_image));
    return textureLoad(t_effect_fourth_image, clamp(texel, vec2<i32>(0), dimensions - 1), 0);
}

fn sample_effect_third_image(input: EffectInput, uv: vec2<f32>) -> vec4<f32> {
    let pixel = input.third_image_origin + uv * input.third_image_size - vec2<f32>(0.5);
    let base = vec2<i32>(floor(pixel));
    let fraction = fract(pixel);
    let top = mix(effect_third_image_texel(base), effect_third_image_texel(base + vec2<i32>(1, 0)), fraction.x);
    let bottom = mix(effect_third_image_texel(base + vec2<i32>(0, 1)), effect_third_image_texel(base + vec2<i32>(1, 1)), fraction.x);
    return mix(top, bottom, fraction.y);
}

fn sample_effect_fourth_image(input: EffectInput, uv: vec2<f32>) -> vec4<f32> {
    let pixel = input.fourth_image_origin + uv * input.fourth_image_size - vec2<f32>(0.5);
    let base = vec2<i32>(floor(pixel));
    let fraction = fract(pixel);
    let top = mix(effect_fourth_image_texel(base), effect_fourth_image_texel(base + vec2<i32>(1, 0)), fraction.x);
    let bottom = mix(effect_fourth_image_texel(base + vec2<i32>(0, 1)), effect_fourth_image_texel(base + vec2<i32>(1, 1)), fraction.x);
    return mix(top, bottom, fraction.y);
}
