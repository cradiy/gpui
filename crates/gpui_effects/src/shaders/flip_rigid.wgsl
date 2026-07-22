fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> {
    let progress = clamp(params.slots[0].x, 0.0, 1.0);
    let pointer_y = clamp(params.slots[0].y, 0.0, 1.0);
    let edge = params.slots[0].z;
    let shadow_strength = params.slots[1].x;
    let highlight_strength = params.slots[1].y;
    let back_brightness = params.slots[1].z;
    let fit = params.slots[2].x;
    let single = params.slots[2].y > 0.5;
    let regions = params.slots[3];
    let background = params.slots[4];
    let page_width = select(0.5, 1.0, single);
    let spine = 1.0 - page_width;
    let page_size = vec2<f32>(input.size.x * page_width, input.size.y);
    let uv = vec2<f32>(
        select(1.0 - input.uv.x, input.uv.x, edge > 0.0),
        input.uv.y,
    );

    let left_uv = vec2<f32>(clamp(uv.x * 2.0, 0.0, 1.0), uv.y);
    let right_uv = vec2<f32>(clamp((uv.x - 0.5) * 2.0, 0.0, 1.0), uv.y);
    var base_left = flip_sample_third(input, left_uv, regions.z, fit, page_size, background);
    var base_right = flip_sample_fourth(input, right_uv, regions.w, fit, page_size, background);
    if (edge < 0.0) {
        base_left = flip_sample_first(input, vec2<f32>(1.0 - left_uv.x, left_uv.y), regions.x, fit, page_size, background);
        base_right = flip_sample_second(input, vec2<f32>(1.0 - right_uv.x, right_uv.y), regions.y, fit, page_size, background);
    }
    var color = select(base_left, base_right, uv.x >= 0.5);
    if (single) {
        color = flip_sample_fourth(input, uv, regions.w, fit, page_size, background);
        if (edge < 0.0) {
            color = flip_sample_second(input, vec2<f32>(1.0 - uv.x, uv.y), regions.y, fit, page_size, background);
        }
    }
    let gutter = exp(-abs(uv.x - 0.5) * 110.0) * select(1.0, 0.0, single);
    color = vec4<f32>(color.rgb * (1.0 - gutter * 0.17), color.a);

    let angle = acos(clamp(1.0 - progress * 2.0, -1.0, 1.0));
    let turn = sin(angle);
    let row_angle = angle + (uv.y - pointer_y) * turn * 0.055;
    let projection = cos(row_angle);
    let width = abs(projection) * page_width;
    let safe_width = max(width, 0.0005);
    let outer_edge = spine + projection * page_width;

    var on_sheet = false;
    var sheet_x = 0.0;
    var front_facing = projection >= 0.0;
    if (front_facing) {
        on_sheet = uv.x >= spine && uv.x <= spine + width;
        sheet_x = (uv.x - spine) / safe_width;
    } else {
        on_sheet = uv.x <= spine && uv.x >= spine - width;
        sheet_x = 1.0 - (spine - uv.x) / safe_width;
    }

    let page_x = clamp(sheet_x, 0.0, 1.0);
    var front = flip_sample_first(input, vec2<f32>(page_x, uv.y), regions.x, fit, page_size, background);
    var back = flip_sample_second(input, vec2<f32>(page_x, uv.y), regions.y, fit, page_size, background);
    if (edge < 0.0) {
        front = flip_sample_third(input, vec2<f32>(1.0 - page_x, uv.y), regions.z, fit, page_size, background);
        back = flip_sample_fourth(input, vec2<f32>(1.0 - page_x, uv.y), regions.w, fit, page_size, background);
    }

    if (on_sheet) {
        let surface = select(back, front, front_facing);
        let normal = normalize(vec3<f32>(-sin(row_angle), 0.0, abs(cos(row_angle))));
        let light = normalize(vec3<f32>(-0.38, -0.2, 0.9));
        let lit_diffuse = 0.72 + 0.32 * max(dot(normal, light), 0.0);
        let diffuse = mix(1.0, lit_diffuse, turn);
        let highlight = pow(max(dot(normal, light), 0.0), 26.0)
            * highlight_strength
            * turn;
        let lit_face_brightness = select(back_brightness, 1.0, front_facing);
        let face_brightness = mix(1.0, lit_face_brightness, turn);
        color = vec4<f32>(
            surface.rgb * (1.0 - gutter * 0.17) * diffuse * face_brightness
                + vec3<f32>(highlight),
            surface.a,
        );
    } else {
        let shadow = exp(-abs(uv.x - outer_edge) / 0.018)
            * turn
            * shadow_strength;
        color = vec4<f32>(color.rgb * (1.0 - shadow * 0.65), color.a);
    }

    let edge_width = 0.002 + turn * 0.0025;
    let paper_edge = exp(-pow((uv.x - outer_edge) / edge_width, 2.0) * 2.0) * turn;
    color = vec4<f32>(
        mix(color.rgb, mix(front.rgb, back.rgb, 0.5), paper_edge * 0.72),
        color.a,
    );
    return color;
}
