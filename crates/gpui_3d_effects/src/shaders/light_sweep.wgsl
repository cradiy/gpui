struct Sweep {
    axis_center: vec4<f32>,
    color_intensity: vec4<f32>,
    shape: vec4<f32>,
}
@group(1) @binding(0) var<uniform> sweep: Sweep;

fn material_surface(input: SurfaceInput, gradients: mat2x2<f32>) -> vec4<f32> {
    let distance = abs(dot(input.world, sweep.axis_center.xyz) - sweep.axis_center.w);
    let band = 1.0 - smoothstep(0.0, sweep.shape.x, distance);
    let alpha = builtin_surface(input, gradients).a * band * band * sweep.shape.y;
    return vec4<f32>(sweep.color_intensity.rgb, alpha);
}

fn material_shading(base: vec3<f32>, input: SurfaceInput,
    gradients: SurfaceGradients, face_sign: f32) -> vec3<f32> {
    let normal = surface_normal(input, gradients.normal) * face_sign;
    let view = material_view_direction(input.world);
    let facing = clamp(dot(normal, view), 0.0, 1.0);
    let surface = clamp(builtin_surface(input, gradients.base).rgb, vec3<f32>(0.0), vec3<f32>(1.0));
    let tint = mix(vec3<f32>(1.0), surface, 0.65);
    return base * tint * (0.4 + 0.6 * facing) * sweep.color_intensity.w;
}
