fn material_surface(input: SurfaceInput, gradients: mat2x2<f32>) -> vec4<f32> {
    return vec4<f32>(0.015, 0.025, 0.04, 1.0);
}
fn material_shading(base: vec3<f32>, input: SurfaceInput,
    gradients: SurfaceGradients, face_sign: f32) -> vec3<f32> {
    return base;
}
