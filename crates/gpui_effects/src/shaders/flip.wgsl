fn page_curve_position(
    sheet_x: f32,
    angle: f32,
    curvature: f32,
    spine: f32,
    page_width: f32,
) -> vec2<f32> {
    let wave = sin(sheet_x * 3.14159265359);
    let direction = vec2<f32>(cos(angle), sin(angle));
    let normal = vec2<f32>(-direction.y, direction.x);
    return vec2<f32>(spine, 0.0)
        + direction * (sheet_x * page_width)
        + normal * (wave * curvature);
}

fn page_curve_tangent(
    sheet_x: f32,
    angle: f32,
    curvature: f32,
    page_width: f32,
) -> vec2<f32> {
    let direction = vec2<f32>(cos(angle), sin(angle));
    let normal = vec2<f32>(-direction.y, direction.x);
    return direction * page_width
        + normal * (cos(sheet_x * 3.14159265359) * 3.14159265359 * curvature);
}

fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> {
    let pi = 3.14159265359;
    let progress = clamp(params.slots[0].x, 0.0, 1.0);
    let pointer_y = clamp(params.slots[0].y, 0.0, 1.0);
    let edge = params.slots[0].z;
    let radius = clamp(params.slots[0].w, 0.04, 0.28);
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

    // Both directions share one canonical geometry. Artwork is remapped below,
    // so a left-edge turn never mirrors text or swaps the resting spread.
    let uv = vec2<f32>(
        select(1.0 - input.uv.x, input.uv.x, edge > 0.0),
        input.uv.y,
    );
    let left_uv = vec2<f32>(clamp(uv.x * 2.0, 0.0, 1.0), uv.y);
    let right_uv = vec2<f32>(clamp((uv.x - 0.5) * 2.0, 0.0, 1.0), uv.y);

    var base_left = flip_sample_third(input, left_uv, regions.z, fit, page_size, background);
    var base_right = flip_sample_fourth(input, right_uv, regions.w, fit, page_size, background);
    if (edge < 0.0) {
        base_left = flip_sample_first(
            input,
            vec2<f32>(1.0 - left_uv.x, left_uv.y),
            regions.x,
            fit,
            page_size,
            background,
        );
        base_right = flip_sample_second(
            input,
            vec2<f32>(1.0 - right_uv.x, right_uv.y),
            regions.y,
            fit,
            page_size,
            background,
        );
    }
    var color = select(base_left, base_right, uv.x >= 0.5);

    if (single) {
        color = flip_sample_fourth(input, uv, regions.w, fit, page_size, background);
        if (edge < 0.0) {
            color = flip_sample_second(
                input,
                vec2<f32>(1.0 - uv.x, uv.y),
                regions.y,
                fit,
                page_size,
                background,
            );
        }
    }

    let gutter = exp(-abs(uv.x - 0.5) * 105.0) * select(1.0, 0.0, single);
    color = vec4<f32>(color.rgb * (1.0 - gutter * 0.17), color.a);

    // acos makes the grabbed outer point follow x = 1 - progress exactly.
    // Rows farther from the pointer lag slightly, producing a soft diagonal
    // fold without turning the page into a rigid trapezoid.
    let base_angle = acos(clamp(1.0 - progress * 2.0, -1.0, 1.0));
    let turn = sin(base_angle);
    let grab_offset = uv.y - pointer_y;
    let row_angle = base_angle + grab_offset * turn * (0.11 + radius * 0.32);
    let curvature = (0.025 + radius * 0.72) * turn * page_width * 2.0
        * (1.0 - abs(grab_offset) * 0.16);

    // Invert the curved sheet numerically. A curl can overlap itself in screen
    // space, so retain the intersection closest to the viewer (largest depth).
    var best_sheet_x = -1.0;
    var best_depth = -1000.0;
    var best_tangent = vec2<f32>(1.0, 0.0);
    var min_curve_distance = 1000.0;
    var previous_sheet_x = 0.0;
    var previous_position = page_curve_position(0.0, row_angle, curvature, spine, page_width);

    for (var index: u32 = 1u; index <= 32u; index = index + 1u) {
        let sheet_x = f32(index) / 32.0;
        let position = page_curve_position(sheet_x, row_angle, curvature, spine, page_width);
        min_curve_distance = min(
            min_curve_distance,
            min(abs(uv.x - previous_position.x), abs(uv.x - position.x)),
        );

        let segment_min = min(previous_position.x, position.x);
        let segment_max = max(previous_position.x, position.x);
        let segment_width = position.x - previous_position.x;
        if (
            uv.x >= segment_min
            && uv.x <= segment_max
            && abs(segment_width) > 0.00001
        ) {
            let amount = clamp(
                (uv.x - previous_position.x) / segment_width,
                0.0,
                1.0,
            );
            let candidate_sheet_x = mix(previous_sheet_x, sheet_x, amount);
            let candidate_position = page_curve_position(
                candidate_sheet_x,
                row_angle,
                curvature,
                spine,
                page_width,
            );
            if (candidate_position.y > best_depth) {
                best_depth = candidate_position.y;
                best_sheet_x = candidate_sheet_x;
                best_tangent = page_curve_tangent(
                    candidate_sheet_x,
                    row_angle,
                    curvature,
                    page_width,
                );
            }
        }

        previous_sheet_x = sheet_x;
        previous_position = position;
    }

    let on_sheet = best_sheet_x >= 0.0;
    let sheet_x = clamp(best_sheet_x, 0.0, 1.0);
    let bend = sin(sheet_x * pi) * curvature;
    let sheet_y = clamp(
        uv.y
            + grab_offset * bend * 0.34
            - sign(grab_offset) * bend * bend * 0.18,
        0.0,
        1.0,
    );

    var front = flip_sample_first(input, vec2<f32>(sheet_x, sheet_y), regions.x, fit, page_size, background);
    var back = flip_sample_second(
        input,
        vec2<f32>(1.0 - sheet_x, sheet_y),
        regions.y,
        fit,
        page_size,
        background,
    );
    if (edge < 0.0) {
        front = flip_sample_third(
            input,
            vec2<f32>(1.0 - sheet_x, sheet_y),
            regions.z,
            fit,
            page_size,
            background,
        );
        back = flip_sample_fourth(
            input,
            vec2<f32>(sheet_x, sheet_y),
            regions.w,
            fit,
            page_size,
            background,
        );
    }

    if (on_sheet) {
        let front_facing = best_tangent.x >= 0.0;
        let surface = select(back, front, front_facing);
        let tangent = normalize(best_tangent);
        let normal = normalize(vec3<f32>(
            -tangent.y,
            -grab_offset * curvature * 0.85,
            abs(tangent.x),
        ));
        let light = normalize(vec3<f32>(-0.34, -0.28, 0.9));
        let lit_diffuse = 0.7 + 0.34 * max(dot(normal, light), 0.0);
        let diffuse = mix(1.0, lit_diffuse, turn);
        let specular = pow(
            max(dot(normal, normalize(vec3<f32>(-0.55, -0.18, 0.82))), 0.0),
            22.0,
        ) * highlight_strength * turn;
        let local_fold = abs(curvature * pi * pi * sin(sheet_x * pi));
        let self_shadow = clamp(local_fold * 0.42, 0.0, 0.24);
        let lit_face_brightness = select(back_brightness, 1.0, front_facing);
        let face_brightness = mix(1.0, lit_face_brightness, turn);
        let transmission = bend * 0.16;
        let paper_rgb = mix(
            surface.rgb,
            mix(front.rgb, back.rgb, 0.5),
            transmission,
        ) * (1.0 - gutter * 0.17);
        color = vec4<f32>(
            paper_rgb * diffuse * face_brightness * (1.0 - self_shadow)
                + vec3<f32>(specular),
            surface.a,
        );
    } else {
        // The elevated curve casts a broad, depth-dependent shadow on the two
        // stationary pages. It stays soft near the outer edge and tightens at
        // the gutter.
        let shadow_width = 0.009 + radius * 0.055 + turn * 0.012;
        let cast_shadow = exp(-min_curve_distance / shadow_width)
            * turn
            * shadow_strength;
        color = vec4<f32>(
            color.rgb * (1.0 - cast_shadow * 0.72),
            color.a,
        );
    }

    // Preserve a thin illuminated paper edge when the surface is nearly
    // perpendicular to the viewer.
    let outer = page_curve_position(1.0, row_angle, curvature, spine, page_width);
    let edge_width = 0.0015 + radius * turn * 0.012;
    let edge_distance = abs(uv.x - outer.x) / edge_width;
    let paper_edge = exp(-edge_distance * edge_distance * 2.2) * turn;
    let edge_color = mix(front.rgb, back.rgb, 0.5)
        * (0.74 + highlight_strength * 0.65);
    color = vec4<f32>(
        mix(color.rgb, edge_color, clamp(paper_edge, 0.0, 0.82)),
        color.a,
    );

    return color;
}
