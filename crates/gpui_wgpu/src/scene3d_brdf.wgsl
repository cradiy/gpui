@vertex
fn vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let p = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    return vec4<f32>(p[index], 0.0, 1.0);
}

@fragment
fn fragment(@builtin(position) position: vec4<f32>) -> @location(0) vec2<f32> {
    let nv = position.x / 128.0;
    let roughness = max(position.y / 128.0, 0.045);
    let a2 = pow(roughness, 4.0);
    let view = vec3<f32>(sqrt(max(1.0 - nv * nv, 0.0)), 0.0, nv);
    var result = vec2<f32>(0.0);
    for (var i = 0u; i < 512u; i += 1u) {
        let u = f32(i) / 512.0;
        let phi = 6.28318530718 * f32(reverseBits(i)) * 2.32830643654e-10;
        let hz = sqrt((1.0 - u) / (1.0 + (a2 - 1.0) * u));
        let radius = sqrt(max(1.0 - hz * hz, 0.0));
        let h = vec3<f32>(radius * cos(phi), radius * sin(phi), hz);
        let vh = max(dot(view, h), 0.0);
        let light = 2.0 * vh * h - view;
        let nl = max(light.z, 0.0);
        if (nl > 0.0 && vh > 0.0) {
            let denominator = nl * sqrt(nv * nv * (1.0 - a2) + a2)
                + nv * sqrt(nl * nl * (1.0 - a2) + a2);
            let weight = 2.0 * nl * vh / max(hz * denominator, 1e-8);
            let fc = pow(1.0 - vh, 5.0);
            result += vec2<f32>(1.0 - fc, fc) * weight;
        }
    }
    return result / 512.0;
}
