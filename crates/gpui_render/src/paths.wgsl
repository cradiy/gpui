// --- paths --- //

struct PathSprite {
    bounds: Bounds,
}
@group(1) @binding(0) var<storage, read> b_path_sprites: array<PathSprite>;

struct PathVarying {
    @builtin(position) position: vec4<f32>,
}

@vertex
fn vs_path(@builtin(vertex_index) vertex_id: u32, @builtin(instance_index) instance_id: u32) -> PathVarying {
    let unit_vertex = vec2<f32>(f32(vertex_id & 1u), 0.5 * f32(vertex_id & 2u));
    let sprite = b_path_sprites[instance_id];
    // Don't apply content mask because it was already accounted for when rasterizing the path.
    let device_position = to_device_position(unit_vertex, sprite.bounds);

    var out = PathVarying();
    out.position = device_position;

    return out;
}

@fragment
fn fs_path(input: PathVarying) -> @location(0) vec4<f32> {
    // Both passes use the same physical viewport and origin. The resolved
    // intermediate stores one premultiplied color per destination pixel.
    return textureLoad(t_sprite, vec2<i32>(input.position.xy), 0);
}
