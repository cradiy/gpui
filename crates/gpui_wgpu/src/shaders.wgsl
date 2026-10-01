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
