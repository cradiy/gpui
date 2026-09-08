struct SdfSample { distance: f32, color: vec4<f32> }

fn sdf_round_rect(p: vec2<f32>, half_size: vec2<f32>, radius: f32) -> f32 {
    let q = abs(p) - half_size + radius;
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - radius;
}
fn sdf_capsule(p: vec2<f32>, half_length: f32, radius: f32) -> f32 {
    return length(p - vec2<f32>(clamp(p.x, -half_length, half_length), 0.0)) - radius;
}
fn sdf_local(p: vec2<f32>, transform: vec4<f32>, device_scale: f32) -> vec2<f32> {
    let q = (p - transform.xy * device_scale) / (transform.z * device_scale);
    let c = cos(transform.w); let s = sin(transform.w);
    return vec2<f32>(c * q.x + s * q.y, -s * q.x + c * q.y);
}
fn sdf_union(a: SdfSample, b: SdfSample, k: f32) -> SdfSample {
    if k <= 0.0 {
        if a.distance <= b.distance { return a; }
        return b;
    }
    let h = clamp(0.5 + 0.5 * (b.distance - a.distance) / k, 0.0, 1.0);
    return SdfSample(mix(b.distance, a.distance, h) - k * h * (1.0 - h), mix(b.color, a.color, h));
}
fn sdf_intersect(a: SdfSample, b: SdfSample, k: f32) -> SdfSample {
    let result = sdf_union(SdfSample(-a.distance, a.color), SdfSample(-b.distance, b.color), k);
    return SdfSample(-result.distance, result.color);
}
fn sdf_subtract(a: SdfSample, b: SdfSample, k: f32) -> SdfSample {
    let result = sdf_intersect(a, SdfSample(-b.distance, a.color), k);
    return SdfSample(result.distance, a.color);
}

fn sdf_paint(sample: SdfSample, settings: vec4<f32>, lights: vec4<f32>) -> vec4<f32> {
    let d = sample.distance;
    let aa = max(fwidth(d) * 0.5, 0.5);
    let inside = 1.0 - smoothstep(-aa, aa, d);
    let base_alpha = sample.color.a;
    let color = sample.color.rgb / max(base_alpha, 0.00001);
    let fill_alpha = inside * lights.w * base_alpha;
    var rgb = color * fill_alpha;
    var alpha = fill_alpha;

    if settings.z > 0.0 {
        let stroke = (1.0 - smoothstep(-aa, aa, abs(d) - settings.z * 0.5)) * base_alpha;
        rgb = color * stroke + rgb * (1.0 - stroke);
        alpha = stroke + alpha * (1.0 - stroke);
    }
    if lights.x > 0.0 {
        let inner = exp(-2.0 * max(-d, 0.0) / lights.x) * inside * lights.z * base_alpha;
        rgb += color * inner * alpha;
    }
    if settings.w > 0.0 {
        let outside = 1.0 - inside;
        let glow = exp(-3.0 * max(d, 0.0) / settings.w) * outside * lights.y * base_alpha;
        rgb += color * glow * (1.0 - alpha);
        alpha += glow * (1.0 - alpha);
    }
    return vec4<f32>(rgb / max(alpha, 0.00001), alpha);
}
