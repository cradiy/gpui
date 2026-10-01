// --- shadows --- //

struct Shadow {
    order: u32,
    blur_radius: f32,
    // The shadow rect for drop shadows; the "hole" rect for inset shadows.
    bounds: Bounds,
    corner_radii: Corners,
    content_mask: Bounds,
    color: Hsla,
    // Only consulted when `inset == 1u`: the element's own bounds, used as a rounded-rect
    // clip so the shadow never escapes the element.
    element_bounds: Bounds,
    element_corner_radii: Corners,
    // 0 = drop shadow, 1 = inset shadow.
    inset: u32,
    pad: u32, // align to 8 bytes
}
@group(1) @binding(0) var<storage, read> b_shadows: array<Shadow>;

struct ShadowVarying {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) color: vec4<f32>,
    @location(1) @interpolate(flat) shadow_id: u32,
    //TODO: use `clip_distance` once Naga supports it
    @location(3) clip_distances: vec4<f32>,
}

@vertex
fn vs_shadow(@builtin(vertex_index) vertex_id: u32, @builtin(instance_index) instance_id: u32) -> ShadowVarying {
    let unit_vertex = vec2<f32>(f32(vertex_id & 1u), 0.5 * f32(vertex_id & 2u));
    var shadow = b_shadows[instance_id];

    var geometry: Bounds;
    if (shadow.inset != 0u) {
        geometry = shadow.element_bounds;
    } else {
        // Leave room for the gaussian tail outside the shadow rect.
        let margin = 3.0 * shadow.blur_radius;
        geometry = shadow.bounds;
        geometry.origin -= vec2<f32>(margin);
        geometry.size += 2.0 * vec2<f32>(margin);
    }

    var out = ShadowVarying();
    out.position = to_device_position(unit_vertex, geometry);
    out.color = hsla_to_rgba(shadow.color);
    out.shadow_id = instance_id;
    out.clip_distances = distance_from_clip_rect(unit_vertex, geometry, shadow.content_mask);
    return out;
}

@fragment
fn fs_shadow(input: ShadowVarying) -> @location(0) vec4<f32> {
    // Alpha clip first, since we don't have `clip_distance`.
    if (any(input.clip_distances < vec4<f32>(0.0))) {
        return vec4<f32>(0.0);
    }

    let shadow = b_shadows[input.shadow_id];
    let half_size = shadow.bounds.size / 2.0;
    let center = shadow.bounds.origin + half_size;
    let center_to_point = (input.position.xy + globals.viewport_origin) - center;

    let corner_radius = pick_corner_radius(center_to_point, shadow.corner_radii);

    var alpha: f32;
    if (shadow.blur_radius == 0.0) {
        let distance = quad_sdf((input.position.xy + globals.viewport_origin), shadow.bounds, shadow.corner_radii);
        alpha = saturate(0.5 - distance);
    } else {
        // The signal is only non-zero in a limited range, so don't waste samples
        let low = center_to_point.y - half_size.y;
        let high = center_to_point.y + half_size.y;
        let start = clamp(-3.0 * shadow.blur_radius, low, high);
        let end = clamp(3.0 * shadow.blur_radius, low, high);

        // Accumulate samples (we can get away with surprisingly few samples)
        let step = (end - start) / 4.0;
        var y = start + step * 0.5;
        alpha = 0.0;
        for (var i = 0; i < 4; i += 1) {
            let blur = blur_along_x(center_to_point.x, center_to_point.y - y,
                shadow.blur_radius, corner_radius, half_size);
            alpha +=  blur * gaussian(y, shadow.blur_radius) * step;
            y += step;
        }
    }

    if (shadow.inset != 0u) {
        // The inset shadow is the complement of the (blurred) hole rect, clipped to the element.
        // `saturate(0.5 - d)` gives a 1-pixel antialiased edge: d <= -0.5 -> 1, d >= 0.5 -> 0.
        alpha = 1.0 - alpha;
        let element_distance = quad_sdf((input.position.xy + globals.viewport_origin), shadow.element_bounds,
                                        shadow.element_corner_radii);
        alpha *= saturate(0.5 - element_distance);
    }

    return blend_color(input.color, alpha);
}

// --- path rasterization --- //

struct PathRasterizationVertex {
    xy_position: vec2<f32>,
    st_position: vec2<f32>,
    color: Background,
    bounds: Bounds,
}

@group(1) @binding(0) var<storage, read> b_path_vertices: array<PathRasterizationVertex>;

struct PathRasterizationVarying {
    @builtin(position) position: vec4<f32>,
    @location(0) st_position: vec2<f32>,
    @location(1) @interpolate(flat) vertex_id: u32,
    //TODO: use `clip_distance` once Naga supports it
    @location(3) clip_distances: vec4<f32>,
}

@vertex
fn vs_path_rasterization(@builtin(vertex_index) vertex_id: u32) -> PathRasterizationVarying {
    let v = b_path_vertices[vertex_id];

    var out = PathRasterizationVarying();
    out.position = to_device_position_impl(v.xy_position);
    out.st_position = v.st_position;
    out.vertex_id = vertex_id;
    out.clip_distances = distance_from_clip_rect_impl(v.xy_position, v.bounds);
    return out;
}

@fragment
fn fs_path_rasterization(input: PathRasterizationVarying) -> @location(0) vec4<f32> {
    let dx = dpdx(input.st_position);
    let dy = dpdy(input.st_position);
    if (any(input.clip_distances < vec4<f32>(0.0))) {
        return vec4<f32>(0.0);
    }

    let v = b_path_vertices[input.vertex_id];
    let background = v.color;
    let bounds = v.bounds;

    var alpha: f32;
    if (length(vec2<f32>(dx.x, dy.x)) < 0.001) {
        // If the gradient is too small, return a solid color.
        alpha = 1.0;
    } else {
        let gradient = 2.0 * input.st_position.xx * vec2<f32>(dx.x, dy.x) - vec2<f32>(dx.y, dy.y);
        let f = input.st_position.x * input.st_position.x - input.st_position.y;
        let distance = f / length(gradient);
        alpha = saturate(0.5 - distance);
    }
    let prepared_gradient = prepare_gradient_color(
        background.tag,
        background.color_space,
        background.solid,
        background.colors,
    );
    let color = gradient_color(
        background,
        (input.position.xy + globals.viewport_origin),
        bounds,
        prepared_gradient.solid,
        prepared_gradient.colors,
    );
    return vec4<f32>(color.rgb * color.a * alpha, color.a * alpha);
}

// --- paths --- //

struct PathSprite {
    bounds: Bounds,
}
@group(1) @binding(0) var<storage, read> b_path_sprites: array<PathSprite>;

struct PathVarying {
    @builtin(position) position: vec4<f32>,
    @location(0) texture_coords: vec2<f32>,
}

@vertex
fn vs_path(@builtin(vertex_index) vertex_id: u32, @builtin(instance_index) instance_id: u32) -> PathVarying {
    let unit_vertex = vec2<f32>(f32(vertex_id & 1u), 0.5 * f32(vertex_id & 2u));
    let sprite = b_path_sprites[instance_id];
    // Don't apply content mask because it was already accounted for when rasterizing the path.
    let device_position = to_device_position(unit_vertex, sprite.bounds);
    // For screen-space intermediate texture, convert screen position to texture coordinates
    let screen_position = sprite.bounds.origin + unit_vertex * sprite.bounds.size;
    let texture_coords = (screen_position - globals.viewport_origin) / globals.viewport_size;

    var out = PathVarying();
    out.position = device_position;
    out.texture_coords = texture_coords;

    return out;
}

@fragment
fn fs_path(input: PathVarying) -> @location(0) vec4<f32> {
    let sample = textureSample(t_sprite, s_sprite, input.texture_coords);
    return sample;
}

// --- underlines --- //

struct Underline {
    order: u32,
    pad: u32,
    bounds: Bounds,
    content_mask: Bounds,
    color: Hsla,
    thickness: f32,
    wavy: u32,
}
@group(1) @binding(0) var<storage, read> b_underlines: array<Underline>;

struct UnderlineVarying {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) color: vec4<f32>,
    @location(1) @interpolate(flat) underline_id: u32,
    //TODO: use `clip_distance` once Naga supports it
    @location(3) clip_distances: vec4<f32>,
}

@vertex
fn vs_underline(@builtin(vertex_index) vertex_id: u32, @builtin(instance_index) instance_id: u32) -> UnderlineVarying {
    let unit_vertex = vec2<f32>(f32(vertex_id & 1u), 0.5 * f32(vertex_id & 2u));
    let underline = b_underlines[instance_id];

    var out = UnderlineVarying();
    out.position = to_device_position(unit_vertex, underline.bounds);
    out.color = hsla_to_rgba(underline.color);
    out.underline_id = instance_id;
    out.clip_distances = distance_from_clip_rect(unit_vertex, underline.bounds, underline.content_mask);
    return out;
}

@fragment
fn fs_underline(input: UnderlineVarying) -> @location(0) vec4<f32> {
    const WAVE_FREQUENCY: f32 = 2.0;
    const WAVE_HEIGHT_RATIO: f32 = 0.8;

    // Alpha clip first, since we don't have `clip_distance`.
    if (any(input.clip_distances < vec4<f32>(0.0))) {
        return vec4<f32>(0.0);
    }

    let underline = b_underlines[input.underline_id];
    if (underline.wavy == 0u)
    {
        return blend_color(input.color, input.color.a);
    }

    let half_thickness = underline.thickness * 0.5;

    let st = ((input.position.xy + globals.viewport_origin) - underline.bounds.origin) / underline.bounds.size.y - vec2<f32>(0.0, 0.5);
    let frequency = M_PI_F * WAVE_FREQUENCY * underline.thickness / underline.bounds.size.y;
    let amplitude = (underline.thickness * WAVE_HEIGHT_RATIO) / underline.bounds.size.y;

    let sine = sin(st.x * frequency) * amplitude;
    let dSine = cos(st.x * frequency) * amplitude * frequency;
    let distance = (st.y - sine) / sqrt(1.0 + dSine * dSine);
    let distance_in_pixels = distance * underline.bounds.size.y;
    let distance_from_top_border = distance_in_pixels - half_thickness;
    let distance_from_bottom_border = distance_in_pixels + half_thickness;
    let alpha = saturate(0.5 - max(-distance_from_bottom_border, distance_from_top_border));
    return blend_color(input.color, alpha * input.color.a);
}

// --- monochrome sprites --- //

struct MonochromeSprite {
    order: u32,
    pad: u32,
    bounds: Bounds,
    content_mask: Bounds,
    background: Background,
    background_bounds: Bounds,
    tile: AtlasTile,
    transformation: TransformationMatrix,
}
@group(1) @binding(0) var<storage, read> b_mono_sprites: array<MonochromeSprite>;

struct MonoSpriteVarying {
    @builtin(position) position: vec4<f32>,
    @location(0) tile_position: vec2<f32>,
    @location(1) @interpolate(flat) sprite_id: u32,
    @location(2) local_position: vec2<f32>,
    @location(3) clip_distances: vec4<f32>,
    @location(4) @interpolate(flat) background_solid: vec4<f32>,
    @location(5) @interpolate(flat) background_color0: vec4<f32>,
    @location(6) @interpolate(flat) background_color1: vec4<f32>,
    @location(7) @interpolate(flat) background_color2: vec4<f32>,
    @location(8) @interpolate(flat) background_color3: vec4<f32>,
}

@vertex
fn vs_mono_sprite(@builtin(vertex_index) vertex_id: u32, @builtin(instance_index) instance_id: u32) -> MonoSpriteVarying {
    let unit_vertex = vec2<f32>(f32(vertex_id & 1u), 0.5 * f32(vertex_id & 2u));
    let sprite = b_mono_sprites[instance_id];

    var out = MonoSpriteVarying();
    out.position = to_device_position_transformed(unit_vertex, sprite.bounds, sprite.transformation);

    out.tile_position = to_tile_position(unit_vertex, sprite.tile);
    out.sprite_id = instance_id;
    out.local_position = unit_vertex * sprite.bounds.size + sprite.bounds.origin;
    out.clip_distances = distance_from_clip_rect_transformed(unit_vertex, sprite.bounds, sprite.content_mask, sprite.transformation);
    let gradient = prepare_gradient_color(
        sprite.background.tag,
        sprite.background.color_space,
        sprite.background.solid,
        sprite.background.colors,
    );
    out.background_solid = gradient.solid;
    out.background_color0 = gradient.colors[0];
    out.background_color1 = gradient.colors[1];
    out.background_color2 = gradient.colors[2];
    out.background_color3 = gradient.colors[3];
    return out;
}

@fragment
fn fs_mono_sprite(input: MonoSpriteVarying) -> @location(0) vec4<f32> {
    let sprite = b_mono_sprites[input.sprite_id];
    let color = gradient_color(
        sprite.background,
        input.local_position,
        sprite.background_bounds,
        input.background_solid,
        array<vec4<f32>, 4>(
            input.background_color0,
            input.background_color1,
            input.background_color2,
            input.background_color3,
        ),
    );
    let sample = textureSample(t_sprite, s_sprite, input.tile_position).r;
    let alpha_corrected = apply_contrast_and_gamma_correction(sample, color.rgb, gamma_params.grayscale_enhanced_contrast, gamma_params.gamma_ratios);

    // Alpha clip after using the derivatives.
    if (any(input.clip_distances < vec4<f32>(0.0))) {
        return vec4<f32>(0.0);
    }

    return blend_color(color, alpha_corrected);
}

// --- polychrome sprites --- //

struct PolychromeSprite {
    order: u32,
    pad: u32,
    grayscale: u32,
    opacity: f32,
    bounds: Bounds,
    clip_bounds: Bounds,
    content_mask: Bounds,
    corner_radii: Corners,
    tile: AtlasTile,
    transformation: TransformationMatrix,
}
@group(1) @binding(0) var<storage, read> b_poly_sprites: array<PolychromeSprite>;

struct PolySpriteVarying {
    @builtin(position) position: vec4<f32>,
    @location(0) tile_position: vec2<f32>,
    @location(1) @interpolate(flat) sprite_id: u32,
    @location(2) local_position: vec2<f32>,
    @location(3) clip_distances: vec4<f32>,
}

@vertex
fn vs_poly_sprite(@builtin(vertex_index) vertex_id: u32, @builtin(instance_index) instance_id: u32) -> PolySpriteVarying {
    let unit_vertex = vec2<f32>(f32(vertex_id & 1u), 0.5 * f32(vertex_id & 2u));
    let sprite = b_poly_sprites[instance_id];

    var out = PolySpriteVarying();
    out.position = to_device_position_transformed(unit_vertex, sprite.bounds, sprite.transformation);
    out.tile_position = to_tile_position(unit_vertex, sprite.tile);
    out.sprite_id = instance_id;
    out.local_position = unit_vertex * sprite.bounds.size + sprite.bounds.origin;
    out.clip_distances = distance_from_clip_rect_transformed(
        unit_vertex,
        sprite.bounds,
        sprite.content_mask,
        sprite.transformation,
    );
    return out;
}

@fragment
fn fs_poly_sprite(input: PolySpriteVarying) -> @location(0) vec4<f32> {
    let sprite = b_poly_sprites[input.sprite_id];
    let atlas_size = vec2<f32>(textureDimensions(t_sprite, 0));
    let tile_min = (vec2<f32>(sprite.tile.bounds.origin) + 0.5) / atlas_size;
    let tile_max = (vec2<f32>(sprite.tile.bounds.origin + sprite.tile.bounds.size) - 0.5) / atlas_size;
    let sample = textureSample(t_sprite, s_sprite, clamp(input.tile_position, tile_min, tile_max));
    // Alpha clip after using the derivatives.
    if (any(input.clip_distances < vec4<f32>(0.0))) {
        return vec4<f32>(0.0);
    }

    let distance = quad_sdf(input.local_position, sprite.clip_bounds, sprite.corner_radii);

    var color = sample;
    if (sprite.grayscale != 0u) {
        let grayscale = dot(color.rgb, GRAYSCALE_FACTORS);
        color = vec4<f32>(vec3<f32>(grayscale), sample.a);
    }
    return blend_color(color, sprite.opacity * saturate(0.5 - distance));
}

// --- surfaces --- //

struct SurfaceParams {
    bounds: Bounds,
    clip_bounds: Bounds,
    content_mask: Bounds,
    uv_bounds: Bounds,
    corner_radii: Corners,
    color_rows: array<vec4<f32>, 3>,
    opacity: f32,
    _pad: array<f32, 3>,
}

@group(1) @binding(0) var<storage, read> surface_locals: SurfaceParams;
@group(1) @binding(1) var t_surface_0: texture_2d<f32>;
@group(1) @binding(2) var t_surface_1: texture_2d<f32>;
@group(1) @binding(3) var s_surface: sampler;

struct SurfaceVarying {
    @builtin(position) position: vec4<f32>,
    @location(0) texture_position: vec2<f32>,
    @location(3) clip_distances: vec4<f32>,
}

@vertex
fn vs_surface(@builtin(vertex_index) vertex_id: u32) -> SurfaceVarying {
    let unit_vertex = vec2<f32>(f32(vertex_id & 1u), 0.5 * f32(vertex_id & 2u));

    var out = SurfaceVarying();
    out.position = to_device_position(unit_vertex, surface_locals.bounds);
    out.texture_position = surface_locals.uv_bounds.origin + unit_vertex * surface_locals.uv_bounds.size;
    out.clip_distances = distance_from_clip_rect(unit_vertex, surface_locals.bounds, surface_locals.content_mask);
    return out;
}

@fragment
fn fs_surface_rgba(input: SurfaceVarying) -> @location(0) vec4<f32> {
    let sample = textureSample(t_surface_0, s_surface, input.texture_position);
    if (any(input.clip_distances < vec4<f32>(0.0))) {
        return vec4<f32>(0.0);
    }
    let distance = quad_sdf((input.position.xy + globals.viewport_origin), surface_locals.clip_bounds, surface_locals.corner_radii);
    return blend_color(sample, surface_locals.opacity * saturate(0.5 - distance));
}

@fragment
fn fs_surface_nv12(input: SurfaceVarying) -> @location(0) vec4<f32> {
    let yuv = vec4<f32>(
        textureSample(t_surface_0, s_surface, input.texture_position).r,
        textureSample(t_surface_1, s_surface, input.texture_position).rg,
        1.0);
    if (any(input.clip_distances < vec4<f32>(0.0))) {
        return vec4<f32>(0.0);
    }
    let rgb = vec3<f32>(
        dot(surface_locals.color_rows[0], yuv),
        dot(surface_locals.color_rows[1], yuv),
        dot(surface_locals.color_rows[2], yuv),
    );
    let distance = quad_sdf((input.position.xy + globals.viewport_origin), surface_locals.clip_bounds, surface_locals.corner_radii);
    return blend_color(vec4<f32>(rgb, 1.0), surface_locals.opacity * saturate(0.5 - distance));
}
