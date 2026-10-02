// --- quads --- //

struct Quad {
    order: u32,
    border_style: u32,
    bounds: Bounds,
    content_mask: Bounds,
    background: Background,
    border_colors: BorderColors,
    border_gradient: BorderGradient,
    corner_radii: Corners,
    border_widths: Edges,
}
@group(1) @binding(0) var<storage, read> b_quads: array<Quad>;

struct QuadVarying {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) border_color_top: vec4<f32>,
    @location(1) @interpolate(flat) border_color_right: vec4<f32>,
    @location(2) @interpolate(flat) border_color_bottom: vec4<f32>,
    @location(3) @interpolate(flat) border_color_left: vec4<f32>,
    @location(4) @interpolate(flat) border_gradient_color0: vec4<f32>,
    @location(5) @interpolate(flat) border_gradient_color1: vec4<f32>,
    @location(8) @interpolate(flat) quad_id: u32,
    // TODO: use `clip_distance` once Naga supports it
    @location(9) clip_distances: vec4<f32>,
    @location(10) @interpolate(flat) background_solid: vec4<f32>,
    @location(11) @interpolate(flat) background_color0: vec4<f32>,
    @location(12) @interpolate(flat) background_color1: vec4<f32>,
}

@vertex
fn vs_quad(@builtin(vertex_index) vertex_id: u32, @builtin(instance_index) instance_id: u32) -> QuadVarying {
    let unit_vertex = vec2<f32>(f32(vertex_id & 1u), 0.5 * f32(vertex_id & 2u));
    let quad = b_quads[instance_id];

    var out = QuadVarying();
    out.position = to_device_position(unit_vertex, quad.bounds);

    let gradient = prepare_gradient_color(quad.background);
    out.background_solid = gradient.solid;
    out.background_color0 = gradient.colors[0];
    out.background_color1 = gradient.colors[1];
    out.border_color_top = linear_srgb_to_oklab(hsla_to_rgba(quad.border_colors.top));
    out.border_color_right = linear_srgb_to_oklab(hsla_to_rgba(quad.border_colors.right));
    out.border_color_bottom = linear_srgb_to_oklab(hsla_to_rgba(quad.border_colors.bottom));
    out.border_color_left = linear_srgb_to_oklab(hsla_to_rgba(quad.border_colors.left));
    var gradient_colors = array<vec4<f32>, 2>();
    if (quad.border_gradient.stop_offset == 0u && quad.border_gradient.stop_count >= 2u) {
        for (var ix = 0u; ix < 2u; ix += 1u) {
            let color = hsla_to_rgba(quad.border_gradient.stops[ix].color);
            gradient_colors[ix] = select(
                linear_to_srgba(color),
                linear_srgb_to_oklab(color),
                quad.border_gradient.color_space == 1u,
            );
        }
    }
    out.border_gradient_color0 = gradient_colors[0];
    out.border_gradient_color1 = gradient_colors[1];
    out.quad_id = instance_id;
    out.clip_distances = distance_from_clip_rect(unit_vertex, quad.bounds, quad.content_mask);
    return out;
}

fn border_perimeter_position(
    point: vec2<f32>,
    size: vec2<f32>,
    center_to_point: vec2<f32>,
    corner_center_to_point: vec2<f32>,
    is_near_rounded_corner: bool,
    radii: Corners,
) -> f32 {
    let top = max(0.0, size.x - radii.top_left - radii.top_right);
    let right = max(0.0, size.y - radii.top_right - radii.bottom_right);
    let bottom = max(0.0, size.x - radii.bottom_right - radii.bottom_left);
    let left = max(0.0, size.y - radii.bottom_left - radii.top_left);
    let arc_tr = radii.top_right * M_PI_F / 2.0;
    let arc_br = radii.bottom_right * M_PI_F / 2.0;
    let arc_bl = radii.bottom_left * M_PI_F / 2.0;
    let arc_tl = radii.top_left * M_PI_F / 2.0;
    let upto_right = top + arc_tr;
    let upto_bottom = upto_right + right + arc_br;
    let upto_left = upto_bottom + bottom + arc_bl;
    let perimeter = max(0.001, upto_left + left + arc_tl);

    var position = 0.0;
    if (is_near_rounded_corner) {
        let angle = atan2(corner_center_to_point.y, corner_center_to_point.x);
        if (center_to_point.x >= 0.0) {
            if (center_to_point.y < 0.0) {
                position = top + radii.top_right * (M_PI_F / 2.0 - angle);
            } else {
                position = upto_right + right + radii.bottom_right * angle;
            }
        } else if (center_to_point.y >= 0.0) {
            position = upto_bottom + bottom + radii.bottom_left * (M_PI_F / 2.0 - angle);
        } else {
            position = upto_left + left + radii.top_left * angle;
        }
    } else if (corner_center_to_point.x < corner_center_to_point.y) {
        if (center_to_point.y < 0.0) {
            position = clamp(point.x - radii.top_left, 0.0, top);
        } else {
            position = upto_bottom + clamp(size.x - radii.bottom_right - point.x, 0.0, bottom);
        }
    } else if (center_to_point.x >= 0.0) {
        position = upto_right + clamp(point.y - radii.top_right, 0.0, right);
    } else {
        position = upto_left + clamp(size.y - radii.bottom_left - point.y, 0.0, left);
    }

    return position / perimeter;
}

fn border_stop_position(gradient: BorderGradient, index: u32) -> f32 {
    if (gradient.stop_offset == 0u) { return gradient.stops[index].position; }
    return gradient_stops[gradient.stop_offset - 1u + index].position;
}

fn border_stop_color(gradient: BorderGradient, colors: array<vec4<f32>, 2>, index: u32) -> vec4<f32> {
    if (gradient.stop_offset == 0u) { return colors[index]; }
    let color = hsla_to_rgba(gradient_stops[gradient.stop_offset - 1u + index].color);
    return select(linear_to_srgba(color), linear_srgb_to_oklab(color), gradient.color_space == 1u);
}

fn sample_border_gradient(
    gradient: BorderGradient,
    position: f32,
    colors: array<vec4<f32>, 2>,
) -> vec4<f32> {
    let count = gradient.stop_count;
    var left_ix = count - 1u;
    var right_ix = 0u;
    var sample_position = position;
    var left_position = border_stop_position(gradient, left_ix);
    var right_position = border_stop_position(gradient, 0u) + 1.0;

    if (position >= border_stop_position(gradient, 0u) && position < left_position) {
        var low = 0u;
        var high = count - 1u;
        loop {
            if (high - low <= 1u) { break; }
            let mid = low + (high - low) / 2u;
            if (position < border_stop_position(gradient, mid)) { high = mid; }
            else { low = mid; }
        }
        left_ix = low;
        right_ix = high;
        left_position = border_stop_position(gradient, left_ix);
        right_position = border_stop_position(gradient, right_ix);
    }
    if (right_ix == 0u && sample_position < border_stop_position(gradient, 0u)) {
        sample_position += 1.0;
    }

    let delta = right_position - left_position;
    var t = 1.0;
    if (delta > 0.0) {
        t = clamp((sample_position - left_position) / delta, 0.0, 1.0);
    }
    var midpoint = 0.5;
    if (gradient.stop_offset != 0u) {
        midpoint = gradient_stops[gradient.stop_offset - 1u + left_ix].midpoint;
    }
    var weight = t;
    if (midpoint != 0.5 && t > 0.0 && t < 1.0) {
        weight = pow(t, log(0.5) / log(midpoint));
    }
    var color = mix(border_stop_color(gradient, colors, left_ix), border_stop_color(gradient, colors, right_ix), weight);
    color.a *= gradient.opacity;
    return color;
}

@fragment
fn fs_quad(input: QuadVarying) -> @location(0) vec4<f32> {
    let quad = b_quads[input.quad_id];
    // Fragment positions preserve pixel centers at shared edges. Interpolating
    // clip distances can turn an exact zero negative on Metal.
    if (any((input.position.xy + globals.viewport_origin) < quad.content_mask.origin)
        || any((input.position.xy + globals.viewport_origin) > quad.content_mask.origin + quad.content_mask.size)) {
        return vec4<f32>(0.0);
    }

    let background_color = gradient_color(
        quad.background,
        (input.position.xy + globals.viewport_origin),
        quad.bounds,
        input.background_solid,
        array(
            input.background_color0,
            input.background_color1,
        ),
    );

    let unrounded = quad.corner_radii.top_left == 0.0 &&
        quad.corner_radii.bottom_left == 0.0 &&
        quad.corner_radii.top_right == 0.0 &&
        quad.corner_radii.bottom_right == 0.0;

    // Fast path when the quad is not rounded and doesn't have any border
    if (quad.border_widths.top == 0.0 &&
            quad.border_widths.left == 0.0 &&
            quad.border_widths.right == 0.0 &&
            quad.border_widths.bottom == 0.0 &&
            unrounded) {
        return blend_color(background_color, 1.0);
    }

    let size = quad.bounds.size;
    let half_size = size / 2.0;
    let point = (input.position.xy + globals.viewport_origin) - quad.bounds.origin;
    let center_to_point = point - half_size;

    // Signed distance field threshold for inclusion of pixels. 0.5 is the
    // minimum distance between the center of the pixel and the edge.
    let antialias_threshold = 0.5;

    // Radius of the nearest corner
    let corner_radius = pick_corner_radius(center_to_point, quad.corner_radii);

    // Width of the nearest borders
    let border = vec2<f32>(
        select(
            quad.border_widths.right,
            quad.border_widths.left,
            center_to_point.x < 0.0),
        select(
            quad.border_widths.bottom,
            quad.border_widths.top,
            center_to_point.y < 0.0));

    // 0-width borders are reduced so that `inner_sdf >= antialias_threshold`.
    // The purpose of this is to not draw antialiasing pixels in this case.
    let reduced_border =
        vec2<f32>(select(border.x, -antialias_threshold, border.x == 0.0),
                  select(border.y, -antialias_threshold, border.y == 0.0));

    // Vector from the corner of the quad bounds to the point, after mirroring
    // the point into the bottom right quadrant. Both components are <= 0.
    let corner_to_point = abs(center_to_point) - half_size;

    // Vector from the point to the center of the rounded corner's circle, also
    // mirrored into bottom right quadrant.
    let corner_center_to_point = corner_to_point + corner_radius;

    // Whether the nearest point on the border is rounded
    let is_near_rounded_corner =
            corner_center_to_point.x >= 0 &&
            corner_center_to_point.y >= 0;

    // Vector from straight border inner corner to point.
    let straight_border_inner_corner_to_point = corner_to_point + reduced_border;

    // Whether the point is beyond the inner edge of the straight border.
    let is_beyond_inner_straight_border =
            straight_border_inner_corner_to_point.x > 0 ||
            straight_border_inner_corner_to_point.y > 0;

    // Whether the point is far enough inside the quad, such that the pixels are
    // not affected by the straight border.
    let is_within_inner_straight_border =
        straight_border_inner_corner_to_point.x < -antialias_threshold &&
        straight_border_inner_corner_to_point.y < -antialias_threshold;

    // Fast path for points that must be part of the background.
    //
    // This could be optimized further for large rounded corners by including
    // points in an inscribed rectangle, or some other quick linear check.
    // However, that might negatively impact performance in the case of
    // reasonable sizes for rounded corners.
    if (is_within_inner_straight_border && !is_near_rounded_corner) {
        return blend_color(background_color, 1.0);
    }

    // Signed distance of the point to the outside edge of the quad's border. It
    // is positive outside this edge, and negative inside.
    let outer_sdf = quad_sdf_impl(corner_center_to_point, corner_radius);

    // Approximate signed distance of the point to the inside edge of the quad's
    // border. It is negative outside this edge (within the border), and
    // positive inside.
    //
    // This is not always an accurate signed distance:
    // * The rounded portions with varying border width use an approximation of
    //   nearest-point-on-ellipse.
    // * When it is quickly known to be outside the edge, -1.0 is used.
    var inner_sdf = 0.0;
    if (corner_center_to_point.x <= 0 || corner_center_to_point.y <= 0) {
        // Fast paths for straight borders.
        inner_sdf = -max(straight_border_inner_corner_to_point.x,
                         straight_border_inner_corner_to_point.y);
    } else if (is_beyond_inner_straight_border) {
        // Fast path for points that must be outside the inner edge.
        inner_sdf = -1.0;
    } else if (reduced_border.x == reduced_border.y) {
        // Fast path for circular inner edge.
        inner_sdf = -(outer_sdf + reduced_border.x);
    } else {
        let ellipse_radii = max(vec2<f32>(0.0), corner_radius - reduced_border);
        inner_sdf = quarter_ellipse_sdf(corner_center_to_point, ellipse_radii);
    }

    // Negative when inside the border
    let border_sdf = max(inner_sdf, outer_sdf);

    var color = background_color;
    if (border_sdf < antialias_threshold) {
        var border_color: vec4<f32>;
        let transition_extent = max(
            0.001,
            min(
                min(half_size.x, half_size.y),
                max(corner_radius, 6.0 * max(border.x, border.y)),
            ),
        );
        if (center_to_point.x < 0.0) {
            if (center_to_point.y < 0.0) {
                let corner_t = smoothstep(
                    0.0,
                    1.0,
                    0.5 + (point.x - point.y) / (2.0 * transition_extent),
                );
                border_color = mix(input.border_color_left, input.border_color_top, corner_t);
            } else {
                let bottom_distance = size.y - point.y;
                let corner_t = smoothstep(
                    0.0,
                    1.0,
                    0.5 + (point.x - bottom_distance) / (2.0 * transition_extent),
                );
                border_color = mix(input.border_color_left, input.border_color_bottom, corner_t);
            }
        } else {
            let right_distance = size.x - point.x;
            if (center_to_point.y < 0.0) {
                let corner_t = smoothstep(
                    0.0,
                    1.0,
                    0.5 + (right_distance - point.y) / (2.0 * transition_extent),
                );
                border_color = mix(input.border_color_right, input.border_color_top, corner_t);
            } else {
                let bottom_distance = size.y - point.y;
                let corner_t = smoothstep(
                    0.0,
                    1.0,
                    0.5 + (right_distance - bottom_distance) / (2.0 * transition_extent),
                );
                border_color = mix(input.border_color_right, input.border_color_bottom, corner_t);
            }
        }
        border_color = oklab_to_linear_srgb(border_color);

        if (quad.border_gradient.stop_count >= 2u) {
            let perimeter_position = border_perimeter_position(
                point,
                size,
                center_to_point,
                corner_center_to_point,
                is_near_rounded_corner,
                quad.corner_radii,
            );
            let gradient_position = fract(perimeter_position + quad.border_gradient.phase);
            let gradient_color = sample_border_gradient(
                quad.border_gradient,
                gradient_position,
                array<vec4<f32>, 2>(
                    input.border_gradient_color0,
                    input.border_gradient_color1,
                ),
            );
            border_color = select(
                srgba_to_linear(gradient_color),
                oklab_to_linear_srgb(gradient_color),
                quad.border_gradient.color_space == 1u,
            );
        }

        // Dashed border logic when border_style == 1
        if (quad.border_style == 1) {
            // Position along the perimeter in "dash space", where each dash
            // period has length 1
            var t = 0.0;

            // Total number of dash periods, so that the dash spacing can be
            // adjusted to evenly divide it
            var max_t = 0.0;

            // Border width is proportional to dash size. This is the behavior
            // used by browsers, but also avoids dashes from different segments
            // overlapping when dash size is smaller than the border width.
            //
            // Dash pattern: (2 * border width) dash, (1 * border width) gap
            let dash_length_per_width = 2.0;
            let dash_gap_per_width = 1.0;
            let dash_period_per_width = dash_length_per_width + dash_gap_per_width;

            // Since the dash size is determined by border width, the density of
            // dashes varies. Multiplying a pixel distance by this returns a
            // position in dash space - it has units (dash period / pixels). So
            // a dash velocity of (1 / 10) is 1 dash every 10 pixels.
            var dash_velocity = 0.0;

            // Dividing this by the border width gives the dash velocity
            let dv_numerator = 1.0 / dash_period_per_width;

            if (unrounded) {
                // When corners aren't rounded, the dashes are separately laid
                // out on each straight line, rather than around the whole
                // perimeter. This way each line starts and ends with a dash.
                let is_horizontal =
                        corner_center_to_point.x <
                        corner_center_to_point.y;

                // When applying dashed borders to just some, not all, the sides.
                // The way we chose border widths above sometimes comes with a 0 width value.
                // So we choose again to avoid division by zero.
                // TODO: A better solution exists taking a look at the whole file.
                // this does not fix single dashed borders at the corners
                let dashed_border = vec2<f32>(
                        max(
                            quad.border_widths.bottom,
                            quad.border_widths.top,
                        ),
                        max(
                            quad.border_widths.right,
                            quad.border_widths.left,
                        )
                   );

                let border_width = select(dashed_border.y, dashed_border.x, is_horizontal);
                dash_velocity = dv_numerator / border_width;
                t = select(point.y, point.x, is_horizontal) * dash_velocity;
                max_t = select(size.y, size.x, is_horizontal) * dash_velocity;
            } else {
                // When corners are rounded, the dashes are laid out clockwise
                // around the whole perimeter.

                let r_tr = quad.corner_radii.top_right;
                let r_br = quad.corner_radii.bottom_right;
                let r_bl = quad.corner_radii.bottom_left;
                let r_tl = quad.corner_radii.top_left;

                let w_t = quad.border_widths.top;
                let w_r = quad.border_widths.right;
                let w_b = quad.border_widths.bottom;
                let w_l = quad.border_widths.left;

                // Straight side dash velocities
                let dv_t = select(dv_numerator / w_t, 0.0, w_t <= 0.0);
                let dv_r = select(dv_numerator / w_r, 0.0, w_r <= 0.0);
                let dv_b = select(dv_numerator / w_b, 0.0, w_b <= 0.0);
                let dv_l = select(dv_numerator / w_l, 0.0, w_l <= 0.0);

                // Straight side lengths in dash space
                let s_t = (size.x - r_tl - r_tr) * dv_t;
                let s_r = (size.y - r_tr - r_br) * dv_r;
                let s_b = (size.x - r_br - r_bl) * dv_b;
                let s_l = (size.y - r_bl - r_tl) * dv_l;

                let corner_dash_velocity_tr = corner_dash_velocity(dv_t, dv_r);
                let corner_dash_velocity_br = corner_dash_velocity(dv_b, dv_r);
                let corner_dash_velocity_bl = corner_dash_velocity(dv_b, dv_l);
                let corner_dash_velocity_tl = corner_dash_velocity(dv_t, dv_l);

                // Corner lengths in dash space
                let c_tr = r_tr * (M_PI_F / 2.0) * corner_dash_velocity_tr;
                let c_br = r_br * (M_PI_F / 2.0) * corner_dash_velocity_br;
                let c_bl = r_bl * (M_PI_F / 2.0) * corner_dash_velocity_bl;
                let c_tl = r_tl * (M_PI_F / 2.0) * corner_dash_velocity_tl;

                // Cumulative dash space upto each segment
                let upto_tr = s_t;
                let upto_r = upto_tr + c_tr;
                let upto_br = upto_r + s_r;
                let upto_b = upto_br + c_br;
                let upto_bl = upto_b + s_b;
                let upto_l = upto_bl + c_bl;
                let upto_tl = upto_l + s_l;
                max_t = upto_tl + c_tl;

                if (is_near_rounded_corner) {
                    let radians = atan2(corner_center_to_point.y,
                                        corner_center_to_point.x);
                    let corner_t = radians * corner_radius;

                    if (center_to_point.x >= 0.0) {
                        if (center_to_point.y < 0.0) {
                            dash_velocity = corner_dash_velocity_tr;
                            // Subtracted because radians is pi/2 to 0 when
                            // going clockwise around the top right corner,
                            // since the y axis has been flipped
                            t = upto_r - corner_t * dash_velocity;
                        } else {
                            dash_velocity = corner_dash_velocity_br;
                            // Added because radians is 0 to pi/2 when going
                            // clockwise around the bottom-right corner
                            t = upto_br + corner_t * dash_velocity;
                        }
                    } else {
                        if (center_to_point.y >= 0.0) {
                            dash_velocity = corner_dash_velocity_bl;
                            // Subtracted because radians is pi/2 to 0 when
                            // going clockwise around the bottom-left corner,
                            // since the x axis has been flipped
                            t = upto_l - corner_t * dash_velocity;
                        } else {
                            dash_velocity = corner_dash_velocity_tl;
                            // Added because radians is 0 to pi/2 when going
                            // clockwise around the top-left corner, since both
                            // axis were flipped
                            t = upto_tl + corner_t * dash_velocity;
                        }
                    }
                } else {
                    // Straight borders
                    let is_horizontal =
                            corner_center_to_point.x <
                            corner_center_to_point.y;
                    if (is_horizontal) {
                        if (center_to_point.y < 0.0) {
                            dash_velocity = dv_t;
                            t = (point.x - r_tl) * dash_velocity;
                        } else {
                            dash_velocity = dv_b;
                            t = upto_bl - (point.x - r_bl) * dash_velocity;
                        }
                    } else {
                        if (center_to_point.x < 0.0) {
                            dash_velocity = dv_l;
                            t = upto_tl - (point.y - r_tl) * dash_velocity;
                        } else {
                            dash_velocity = dv_r;
                            t = upto_r + (point.y - r_tr) * dash_velocity;
                        }
                    }
                }
            }

            let dash_length = dash_length_per_width / dash_period_per_width;
            let desired_dash_gap = dash_gap_per_width / dash_period_per_width;

            // Straight borders should start and end with a dash, so max_t is
            // reduced to cause this.
            max_t -= select(0.0, dash_length, unrounded);
            if (max_t >= 1.0) {
                // Adjust dash gap to evenly divide max_t.
                let dash_count = floor(max_t);
                let dash_period = max_t / dash_count;
                border_color.a *= dash_alpha(
                    t,
                    dash_period,
                    dash_length,
                    dash_velocity,
                    antialias_threshold);
            } else if (unrounded) {
                // When there isn't enough space for the full gap between the
                // two start / end dashes of a straight border, reduce gap to
                // make them fit.
                let dash_gap = max_t - dash_length;
                if (dash_gap > 0.0) {
                    let dash_period = dash_length + dash_gap;
                    border_color.a *= dash_alpha(
                        t,
                        dash_period,
                        dash_length,
                        dash_velocity,
                        antialias_threshold);
                }
            }
        }

        // Blend the border on top of the background and then linearly interpolate
        // between the two as we slide inside the background.
        let blended_border = over(background_color, border_color);
        color = mix(background_color, blended_border,
                    saturate(antialias_threshold - inner_sdf));
    }

    return blend_color(color, saturate(antialias_threshold - outer_sdf));
}

// Returns the dash velocity of a corner given the dash velocity of the two
// sides, by returning the slower velocity (larger dashes).
//
// Since 0 is used for dash velocity when the border width is 0 (instead of
// +inf), this returns the other dash velocity in that case.
//
// An alternative to this might be to appropriately interpolate the dash
// velocity around the corner, but that seems overcomplicated.
fn corner_dash_velocity(dv1: f32, dv2: f32) -> f32 {
    if (dv1 == 0.0) {
        return dv2;
    } else if (dv2 == 0.0) {
        return dv1;
    } else {
        return min(dv1, dv2);
    }
}

// Returns alpha used to render antialiased dashes.
// `t` is within the dash when `fmod(t, period) < length`.
fn dash_alpha(t: f32, period: f32, length: f32, dash_velocity: f32, antialias_threshold: f32) -> f32 {
    let half_period = period / 2;
    let half_length = length / 2;
    // Value in [-half_period, half_period].
    // The dash is in [-half_length, half_length].
    let centered = fmod(t + half_period - half_length, period) - half_period;
    // Signed distance for the dash, negative values are inside the dash.
    let signed_distance = abs(centered) - half_length;
    // Antialiased alpha based on the signed distance.
    return saturate(antialias_threshold - signed_distance / dash_velocity);
}

// This approximates distance to the nearest point to a quarter ellipse in a way
// that is sufficient for anti-aliasing when the ellipse is not very eccentric.
// The components of `point` are expected to be positive.
//
// Negative on the outside and positive on the inside.
fn quarter_ellipse_sdf(point: vec2<f32>, radii: vec2<f32>) -> f32 {
    // Scale the space to treat the ellipse like a unit circle.
    let circle_vec = point / radii;
    let unit_circle_sdf = length(circle_vec) - 1.0;
    // Approximate up-scaling of the length by using the average of the radii.
    //
    // TODO: A better solution would be to use the gradient of the implicit
    // function for an ellipse to approximate a scaling factor.
    return unit_circle_sdf * (radii.x + radii.y) * -0.5;
}

// Modulus that has the same sign as `a`.
fn fmod(a: f32, b: f32) -> f32 {
    return a - b * trunc(a / b);
}
