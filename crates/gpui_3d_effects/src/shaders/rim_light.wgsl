struct Rim {
    color_intensity: vec4<f32>,
    shape: vec4<f32>,
}
@group(1) @binding(0) var<uniform> rim: Rim;

fn material_surface(input: SurfaceInput, gradients: mat2x2<f32>) -> vec4<f32> {
    return vec4<f32>(rim.color_intensity.rgb, builtin_surface(input, gradients).a * rim.shape.y);
}

fn material_shading(base: vec3<f32>, input: SurfaceInput,
    gradients: SurfaceGradients, face_sign: f32) -> vec3<f32> {
    let normal = surface_normal(input, gradients.normal) * face_sign;
    let view = material_view_direction(input.world);
    let grazing = 1.0 - clamp(abs(dot(normal, view)), 0.0, 1.0);
    let light = pow(grazing, rim.shape.x);
    let surface = clamp(builtin_surface(input, gradients.base).rgb, vec3<f32>(0.0), vec3<f32>(1.0));
    return base * mix(vec3<f32>(1.0), surface, 0.25) * light * rim.color_intensity.w;
}
