const VERTEX_WORDS = 24u;
const UV_WORDS = array<u32, 5>(6u, 8u, 14u, 16u, 18u);
const COLOR_WORD = 20u;

@group(0) @binding(0) var<storage, read> updates: array<u32>;
@group(0) @binding(1) var<storage, read_write> vertices: array<u32>;

@compute @workgroup_size(64)
fn update_attributes(@builtin(global_invocation_id) id: vec3<u32>) {
    let vertex = id.x;
    if vertex >= updates[0] { return; }
    let start = vertex * VERTEX_WORDS;
    for (var slot = 0u; slot < 5u; slot++) {
        let offset = updates[1u + slot];
        if offset != 0u {
            vertices[start + UV_WORDS[slot]] = updates[offset + vertex * 2u];
            vertices[start + UV_WORDS[slot] + 1u] = updates[offset + vertex * 2u + 1u];
        }
    }
    let color = updates[6];
    if color != 0u {
        for (var component = 0u; component < 4u; component++) {
            vertices[start + COLOR_WORD + component] = updates[color + vertex * 4u + component];
        }
    }
}
