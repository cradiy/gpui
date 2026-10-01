fn backdrop_sample_texture(texture: texture_2d<f32>, uv: vec2<f32>) -> vec4<f32> {
    let dimensions = vec2<f32>(textureDimensions(texture));
    let position = clamp(
        uv * dimensions - vec2<f32>(0.5),
        vec2<f32>(0.0),
        dimensions - vec2<f32>(1.0),
    );
    let low = vec2<i32>(floor(position));
    let high = min(low + vec2<i32>(1), vec2<i32>(dimensions) - vec2<i32>(1));
    let factor = fract(position);
    let top = mix(
        textureLoad(texture, low, 0),
        textureLoad(texture, vec2<i32>(high.x, low.y), 0),
        factor.x,
    );
    let bottom = mix(
        textureLoad(texture, vec2<i32>(low.x, high.y), 0),
        textureLoad(texture, high, 0),
        factor.x,
    );
    return mix(top, bottom, factor.y);
}
