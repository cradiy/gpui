@group(1) @binding(BINDING) var t_effect_NAMEimage: texture_2d<f32>;

fn load_effect_NAMEimage(pixel: vec2<i32>) -> vec4<f32> {
    let size = vec2<i32>(textureDimensions(t_effect_NAMEimage));
    return textureLoad(t_effect_NAMEimage, clamp(pixel, vec2<i32>(0), size - 1), 0);
}

fn sample_effect_NAMEimage(input: EffectInput, uv: vec2<f32>) -> vec4<f32> {
    let pixel = clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0)) * input.NAMEimage_size - 0.5;
    var color: vec4<f32>;
    if (NEAREST) {
        color = load_effect_NAMEimage(vec2<i32>(floor(pixel + 0.5)));
    } else {
        let low = vec2<i32>(floor(pixel));
        let factor = fract(pixel);
        color = mix(
            mix(load_effect_NAMEimage(low), load_effect_NAMEimage(low + vec2<i32>(1, 0)), factor.x),
            mix(load_effect_NAMEimage(low + vec2<i32>(0, 1)), load_effect_NAMEimage(low + vec2<i32>(1, 1)), factor.x),
            factor.y,
        );
    }
    if (PREMULTIPLIED) {
        if (color.a > 0.0) {
            return vec4<f32>(color.rgb / color.a, color.a);
        }
        return vec4<f32>(0.0);
    }
    return color;
}

fn sample_effect_NAMEimage_cover(input: EffectInput, uv: vec2<f32>) -> vec4<f32> {
    let source_aspect = input.NAMEimage_size.x / max(input.NAMEimage_size.y, 1.0);
    let target_aspect = input.size.x / max(input.size.y, 1.0);
    var covered = uv;
    if (source_aspect > target_aspect) {
        covered.x = (uv.x - 0.5) * target_aspect / source_aspect + 0.5;
    } else {
        covered.y = (uv.y - 0.5) * source_aspect / target_aspect + 0.5;
    }
    return sample_effect_NAMEimage(input, covered);
}
