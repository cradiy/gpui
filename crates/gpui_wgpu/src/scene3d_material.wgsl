fn material_surface(input: SurfaceInput, gradients: mat2x2<f32>) -> vec4<f32> {
    return builtin_surface(input, gradients);
}

fn material_shading(base: vec3<f32>, input: SurfaceInput, gradients: SurfaceGradients, face_sign: f32) -> vec3<f32> {
    return builtin_shading(base, input, gradients, face_sign);
}
