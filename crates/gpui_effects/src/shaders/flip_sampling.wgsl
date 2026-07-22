struct FlipSampleUv {
    uv: vec2<f32>,
    inside: bool,
}

fn flip_region_size(image_size: vec2<f32>, region: f32) -> vec2<f32> {
    return vec2<f32>(
        select(image_size.x, image_size.x * 0.5, region > 0.5),
        image_size.y,
    );
}

fn flip_region_uv(uv: vec2<f32>, region: f32) -> vec2<f32> {
    if (region < 0.5) {
        return uv;
    }
    let offset = select(0.0, 0.5, region > 1.5);
    return vec2<f32>(uv.x * 0.5 + offset, uv.y);
}

fn flip_fit_uv(
    uv: vec2<f32>,
    source_size: vec2<f32>,
    target_size: vec2<f32>,
    fit: f32,
) -> FlipSampleUv {
    if (fit < 0.5) {
        return FlipSampleUv(uv, true);
    }

    let source_aspect = source_size.x / max(source_size.y, 1.0);
    let target_aspect = target_size.x / max(target_size.y, 1.0);

    if (fit < 1.5 || (fit > 2.5 && fit < 3.5 && any(source_size > target_size))) {
        var fitted = uv;
        var inside = true;
        if (source_aspect > target_aspect) {
            let height = target_aspect / source_aspect;
            let top = (1.0 - height) * 0.5;
            inside = uv.y >= top && uv.y <= top + height;
            fitted.y = (uv.y - top) / max(height, 0.0001);
        } else {
            let width = source_aspect / target_aspect;
            let left = (1.0 - width) * 0.5;
            inside = uv.x >= left && uv.x <= left + width;
            fitted.x = (uv.x - left) / max(width, 0.0001);
        }
        return FlipSampleUv(fitted, inside);
    }

    if (fit < 2.5) {
        var covered = uv;
        if (source_aspect > target_aspect) {
            let visible_width = target_aspect / source_aspect;
            covered.x = (uv.x - 0.5) * visible_width + 0.5;
        } else {
            let visible_height = source_aspect / target_aspect;
            covered.y = (uv.y - 0.5) * visible_height + 0.5;
        }
        return FlipSampleUv(covered, true);
    }

    // ObjectFit::None keeps the source at its device-pixel size and centers it.
    let rendered = source_size / max(target_size, vec2<f32>(1.0));
    let top_left = (vec2<f32>(1.0) - rendered) * 0.5;
    let inside = all(uv >= top_left) && all(uv <= top_left + rendered);
    return FlipSampleUv((uv - top_left) / max(rendered, vec2<f32>(0.0001)), inside);
}

fn flip_sample_first(
    input: EffectInput,
    uv: vec2<f32>,
    region: f32,
    fit: f32,
    target_size: vec2<f32>,
    background: vec4<f32>,
) -> vec4<f32> {
    let mapped = flip_fit_uv(uv, flip_region_size(input.image_size, region), target_size, fit);
    if (!mapped.inside) {
        return background;
    }
    return sample_effect_image(input, flip_region_uv(mapped.uv, region));
}

fn flip_sample_second(
    input: EffectInput,
    uv: vec2<f32>,
    region: f32,
    fit: f32,
    target_size: vec2<f32>,
    background: vec4<f32>,
) -> vec4<f32> {
    let mapped = flip_fit_uv(uv, flip_region_size(input.second_image_size, region), target_size, fit);
    if (!mapped.inside) {
        return background;
    }
    return sample_effect_second_image(input, flip_region_uv(mapped.uv, region));
}

fn flip_sample_third(
    input: EffectInput,
    uv: vec2<f32>,
    region: f32,
    fit: f32,
    target_size: vec2<f32>,
    background: vec4<f32>,
) -> vec4<f32> {
    let mapped = flip_fit_uv(uv, flip_region_size(input.third_image_size, region), target_size, fit);
    if (!mapped.inside) {
        return background;
    }
    return sample_effect_third_image(input, flip_region_uv(mapped.uv, region));
}

fn flip_sample_fourth(
    input: EffectInput,
    uv: vec2<f32>,
    region: f32,
    fit: f32,
    target_size: vec2<f32>,
    background: vec4<f32>,
) -> vec4<f32> {
    let mapped = flip_fit_uv(uv, flip_region_size(input.fourth_image_size, region), target_size, fit);
    if (!mapped.inside) {
        return background;
    }
    return sample_effect_fourth_image(input, flip_region_uv(mapped.uv, region));
}
