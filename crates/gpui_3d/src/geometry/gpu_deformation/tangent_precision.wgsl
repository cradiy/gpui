// Decode finite f32 bits without arithmetic on f32 subnormals.
fn wide_component(bits: u32) -> f64 {
    let exponent = (bits >> 23u) & 255u;
    let mantissa = (bits & 0x7fffffu) | select(0u, 0x800000u, exponent != 0u);
    let magnitude = ldexp(f64(mantissa), i32(max(exponent, 1u)) - 150);
    return select(magnitude, -magnitude, (bits & 0x80000000u) != 0u);
}

fn rounded_mantissa(value: f64) -> u32 {
    let integral = floor(value);
    let fraction = value - integral;
    let mantissa = u32(integral);
    let increment = fraction > f64(0.5) ||
        (fraction == f64(0.5) && (mantissa & 1u) != 0u);
    return mantissa + select(0u, 1u, increment);
}

// Encode a finite component in [-1, 1], including signed zeros and subnormals.
fn component_key(value: f64, sign: u32) -> u32 {
    let magnitude = abs(value);
    if magnitude == f64(0.0) { return sign; }
    if magnitude < ldexp(f64(1.0), -126) {
        return sign | rounded_mantissa(ldexp(magnitude, 149));
    }
    var mantissa = magnitude;
    var exponent = 0;
    for (var shift = 64; shift > 0; shift /= 2) {
        if mantissa < ldexp(f64(1.0), -shift) {
            mantissa = ldexp(mantissa, shift);
            exponent -= shift;
        }
    }
    if mantissa < f64(1.0) {
        mantissa *= f64(2.0);
        exponent -= 1;
    }
    return sign | ((u32(exponent + 126) << 23u) + rounded_mantissa(ldexp(mantissa, 23)));
}

fn wide_vector(value: vec3<f32>) -> vec3<f64> {
    let bits = bitcast<vec3<u32>>(value);
    return vec3(wide_component(bits.x), wide_component(bits.y), wide_component(bits.z));
}

fn narrow_vector(value: vec3<f64>) -> vec3<f32> {
    let signs = select(vec3(0u), vec3(0x80000000u), value < vec3(f64(0.0)));
    return bitcast<vec3<f32>>(vec3(
        component_key(value.x, signs.x), component_key(value.y, signs.y), component_key(value.z, signs.z),
    ));
}

fn wide_dot(a: vec3<f64>, b: vec3<f64>) -> f64 { return (a.x * b.x + a.y * b.y) + a.z * b.z; }
fn wide_unit(value: vec3<f64>) -> vec3<f64> {
    let squared = wide_dot(value, value);
    if squared == f64(0.0) { return vec3(f64(0.0)); }
    return value / sqrt(squared);
}
