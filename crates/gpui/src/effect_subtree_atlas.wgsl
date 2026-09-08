@group(1) @binding(BINDING) var t_effect_INDEX_image: texture_2d<f32>;

fn sample_effect_INDEX_image(input: EffectInput, uv: vec2<f32>) -> vec4<f32> {
    let origin = input.INDEX_image_origin;
    let end = origin + max(input.INDEX_image_size - vec2<f32>(1.0), vec2<f32>(0.0));
    let pixel = clamp(origin + uv * input.INDEX_image_size - vec2<f32>(0.5), origin, end);
    let low = floor(pixel);
    let high = min(low + vec2<f32>(1.0), end);
    let factor = fract(pixel);
    return mix(
        mix(textureLoad(t_effect_INDEX_image, vec2<i32>(low), 0),
            textureLoad(t_effect_INDEX_image, vec2<i32>(vec2<f32>(high.x, low.y)), 0), factor.x),
        mix(textureLoad(t_effect_INDEX_image, vec2<i32>(vec2<f32>(low.x, high.y)), 0),
            textureLoad(t_effect_INDEX_image, vec2<i32>(high), 0), factor.x), factor.y);
}

fn effect_INDEX_repeat_texel(input: EffectInput, texel: vec2<i32>) -> vec4<f32> {
    let size = max(vec2<i32>(input.INDEX_image_size), vec2<i32>(1));
    let wrapped = ((texel % size) + size) % size;
    return textureLoad(t_effect_INDEX_image, vec2<i32>(input.INDEX_image_origin) + wrapped, 0);
}

fn sample_effect_INDEX_image_repeat(input: EffectInput, uv: vec2<f32>) -> vec4<f32> {
    let pixel = fract(uv) * input.INDEX_image_size - vec2<f32>(0.5);
    let low = vec2<i32>(floor(pixel));
    let factor = fract(pixel);
    return mix(
        mix(effect_INDEX_repeat_texel(input, low), effect_INDEX_repeat_texel(input, low + vec2<i32>(1, 0)), factor.x),
        mix(effect_INDEX_repeat_texel(input, low + vec2<i32>(0, 1)), effect_INDEX_repeat_texel(input, low + vec2<i32>(1, 1)), factor.x), factor.y);
}

fn sample_effect_INDEX_image_cover(input: EffectInput, uv: vec2<f32>) -> vec4<f32> {
    let source_aspect = input.INDEX_image_size.x / max(input.INDEX_image_size.y, 1.0);
    let target_aspect = input.size.x / max(input.size.y, 1.0);
    var covered = uv;
    if (source_aspect > target_aspect) {
        covered.x = (uv.x - 0.5) * target_aspect / source_aspect + 0.5;
    } else {
        covered.y = (uv.y - 0.5) * source_aspect / target_aspect + 0.5;
    }
    return sample_effect_INDEX_image(input, covered);
}
