struct Controls { settings: vec4<f32> }
@group(1) @binding(0) var<uniform> controls: Controls;
@group(1) @binding(1) var sphere_map: texture_2d<f32>;
@group(1) @binding(2) var sphere_sampler: sampler;

fn material_surface(input: SurfaceInput, gradients: mat2x2<f32>) -> vec4<f32> {
    return builtin_surface(input, gradients);
}

fn material_shading(base: vec3<f32>, input: SurfaceInput,
    gradients: SurfaceGradients, face_sign: f32) -> vec3<f32> {
    let normal = surface_normal(input, gradients.normal) * face_sign;
    let incident = -material_view_direction(input.world);
    let reflected = unit_vector(material_view_vector(reflect(incident, normal)));
    let denominator = max(2.0 * length(vec3<f32>(reflected.xy, reflected.z + 1.0)), 0.00001);
    let uv = reflected.xy * vec2<f32>(1.0, -1.0) / denominator + vec2<f32>(0.5);
    let environment = textureSampleLevel(sphere_map, sphere_sampler, uv, 0.0).rgb;
    return environment * mix(vec3<f32>(1.0), base, 0.2) * controls.settings.x;
}
