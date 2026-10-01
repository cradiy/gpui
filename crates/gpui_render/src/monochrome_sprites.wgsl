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
    let gradient = prepare_gradient_color(sprite.background);
    out.background_solid = gradient.solid;
    out.background_color0 = gradient.colors[0];
    out.background_color1 = gradient.colors[1];
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
        array<vec4<f32>, 2>(
            input.background_color0,
            input.background_color1,
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
