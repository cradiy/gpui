@group(1) @binding(3) var t_effect_second_image: texture_2d<f32>;

fn effect_second_image_texel(texel: vec2<i32>) -> vec4<f32> {
    let dimensions = vec2<i32>(textureDimensions(t_effect_second_image));
    return textureLoad(t_effect_second_image, clamp(texel, vec2<i32>(0), dimensions - 1), 0);
}

fn sample_effect_second_image(input: EffectInput, uv: vec2<f32>) -> vec4<f32> {
    let pixel = input.second_image_origin + uv * input.second_image_size - vec2<f32>(0.5);
    let base = vec2<i32>(floor(pixel));
    let fraction = fract(pixel);
    let top = mix(
        effect_second_image_texel(base),
        effect_second_image_texel(base + vec2<i32>(1, 0)),
        fraction.x,
    );
    let bottom = mix(
        effect_second_image_texel(base + vec2<i32>(0, 1)),
        effect_second_image_texel(base + vec2<i32>(1, 1)),
        fraction.x,
    );
    return mix(top, bottom, fraction.y);
}

fn sample_effect_second_image_cover(input: EffectInput, uv: vec2<f32>) -> vec4<f32> {
    let source_aspect = input.second_image_size.x / max(input.second_image_size.y, 1.0);
    let target_aspect = input.size.x / max(input.size.y, 1.0);
    var covered_uv = uv;
    if (source_aspect > target_aspect) {
        let visible = target_aspect / source_aspect;
        covered_uv.x = (uv.x - 0.5) * visible + 0.5;
    } else {
        let visible = source_aspect / target_aspect;
        covered_uv.y = (uv.y - 0.5) * visible + 0.5;
    }
    return sample_effect_second_image(input, covered_uv);
}
