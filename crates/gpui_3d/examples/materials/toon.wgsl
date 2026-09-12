struct Controls { settings: vec4<f32> }
@group(1) @binding(0) var<uniform> controls: Controls;

fn material_surface(input: SurfaceInput, gradients: mat2x2<f32>) -> vec4<f32> {
    return builtin_surface(input, gradients);
}

fn material_shading(base: vec3<f32>, input: SurfaceInput,
    gradients: SurfaceGradients, face_sign: f32) -> vec3<f32> {
    let normal = surface_normal(input, gradients.normal) * face_sign;
    let geometric = unit_vector(input.normal) * face_sign;
    let levels = max(controls.settings.x, 1.0);
    var light = material_ambient(normal);
    for (var i = 0u; i < material_light_count(); i++) {
        let direct = material_light(i, input.world, geometric, gradients.shadow_depth);
        let cosine = max(dot(normal, direct.direction), 0.0);
        let band = floor(cosine * levels + 0.5) / levels;
        light += direct.energy * band / 3.14159265;
    }
    let view = material_view_direction(input.world);
    let rim = pow(1.0 - max(dot(normal, view), 0.0), 4.0);
    return base * light + vec3<f32>(rim * controls.settings.y);
}
