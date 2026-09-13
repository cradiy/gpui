struct Curve { state: vec4<f32> }
@group(1) @binding(0) var<uniform> curve: Curve;

fn material_surface(input: SurfaceInput, gradients: mat2x2<f32>) -> vec4<f32> {
    let visible = curve.state.y > 0.0 && input.uv.x <= curve.state.y;
    let surface = builtin_surface(input, gradients);
    return vec4<f32>(surface.rgb, select(0.0, surface.a, visible));
}

fn material_shading(base: vec3<f32>, input: SurfaceInput,
    gradients: SurfaceGradients, face_sign: f32) -> vec3<f32> {
    let distance = curve.state.x - input.uv.x;
    let extent = select(0.018, curve.state.z, distance >= 0.0);
    let t = clamp(1.0 - abs(distance) / extent, 0.0, 1.0);
    let envelope = t * t * (3.0 - 2.0 * t);
    return base * mix(curve.state.w, 1.0, envelope * envelope);
}
