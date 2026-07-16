@group(1) @binding(1) var t_effect_image: texture_2d<f32>;

fn effect_image_pixel(input: EffectInput, uv: vec2<f32>) -> vec2<f32> {
    let clamped = clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0));
    let image_span = max(input.image_size - vec2<f32>(1.0), vec2<f32>(0.0));
    return input.image_origin + clamped * image_span;
}

fn effect_image_cover_uv(input: EffectInput, uv: vec2<f32>) -> vec2<f32> {
    let output_aspect = input.size.x / max(input.size.y, 1.0);
    let image_aspect = input.image_size.x / max(input.image_size.y, 1.0);
    var covered = uv;
    if (image_aspect > output_aspect) {
        let visible_width = output_aspect / image_aspect;
        covered.x = (uv.x - 0.5) * visible_width + 0.5;
    } else {
        let visible_height = image_aspect / output_aspect;
        covered.y = (uv.y - 0.5) * visible_height + 0.5;
    }
    return covered;
}

fn sample_effect_image(input: EffectInput, uv: vec2<f32>) -> vec4<f32> {
    let pixel = effect_image_pixel(input, uv);
    let low = floor(pixel);
    let high = min(low + vec2<f32>(1.0), input.image_origin + input.image_size - vec2<f32>(1.0));
    let factor = fract(pixel);
    let top_left = textureLoad(
        t_effect_image,
        vec2<i32>(i32(low.x), i32(low.y)),
        0,
    );
    let top_right = textureLoad(
        t_effect_image,
        vec2<i32>(i32(high.x), i32(low.y)),
        0,
    );
    let bottom_left = textureLoad(
        t_effect_image,
        vec2<i32>(i32(low.x), i32(high.y)),
        0,
    );
    let bottom_right = textureLoad(
        t_effect_image,
        vec2<i32>(i32(high.x), i32(high.y)),
        0,
    );
    return mix(
        mix(top_left, top_right, factor.x),
        mix(bottom_left, bottom_right, factor.x),
        factor.y,
    );
}

fn sample_effect_image_cover(input: EffectInput, uv: vec2<f32>) -> vec4<f32> {
    return sample_effect_image(input, effect_image_cover_uv(input, uv));
}
