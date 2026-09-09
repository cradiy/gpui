fn transition_mix(first: vec4<f32>, second: vec4<f32>, progress: f32) -> vec4<f32> {
    let alpha = mix(first.a, second.a, progress);
    let rgb = mix(first.rgb * first.a, second.rgb * second.a, progress);
    return vec4<f32>(rgb / max(alpha, 0.000001), alpha);
}
