struct Params {
    bounds: vec4<f32>,
    viewport: vec4<f32>,
    motion: vec4<f32>,
    shape: vec4<f32>,
    grid: vec4<u32>,
}
@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var source: texture_2d<f32>;
@group(0) @binding(2) var source_sampler: sampler;

fn random(seed: u32) -> f32 {
    var value = seed;
    value = (value ^ (value >> 16u)) * 0x7feb352du;
    value = (value ^ (value >> 15u)) * 0x846ca68bu;
    value = value ^ (value >> 16u);
    return f32(value >> 8u) / 16777216.0;
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) @interpolate(flat) home: vec2<f32>,
    @location(2) @interpolate(flat) axis: vec2<f32>,
    @location(3) @interpolate(flat) motion: vec4<f32>,
    @location(4) @interpolate(flat) color: vec4<f32>,
    @location(5) @interpolate(flat) half_size: vec2<f32>,
}

fn sampled(position: vec2<f32>) -> vec4<f32> {
    if (any(position < params.bounds.xy) || any(position >= params.bounds.xy + params.bounds.zw)) { return vec4<f32>(0.0); }
    if (any(position < vec2<f32>(0.0)) || any(position >= params.viewport.xy)) { return vec4<f32>(0.0); }
    return textureSampleLevel(source, source_sampler, position / params.viewport.xy, 0.0);
}

@vertex
fn vertex(@builtin(vertex_index) vertex: u32, @builtin(instance_index) index: u32) -> VertexOutput {
    let cell = params.viewport.z;
    let grid = vec2<u32>(index % params.grid.x, index / params.grid.x);
    let start = vec2<f32>(grid) * cell;
    let extent = min(vec2<f32>(cell), params.bounds.zw - start);
    let home = params.bounds.xy + start + extent * 0.5;
    let seed = grid.x * 73856093u ^ grid.y * 19349663u ^ params.grid.z;
    let delay = random(seed) * 0.4;
    let t = clamp((params.viewport.w - delay) / (1.0 - delay), 0.0, 1.0);
    let travel = t * t * (3.0 - 2.0 * t);
    let angle = random(seed + 1u) * 6.2831853;
    let radial = vec2<f32>(cos(angle), sin(angle));
    let spread = params.motion.z * sqrt(random(seed + 2u));
    let drift = params.motion.xy + radial * spread;
    let curl = vec2<f32>(-radial.y, radial.x) * params.motion.z * 0.25;
    let position = home + drift * travel + curl * sin(travel * 3.14159265);
    let tangent = drift + curl * cos(travel * 3.14159265) * 3.14159265;
    let axis = select(vec2<f32>(0.0, -1.0), tangent / max(length(tangent), 0.001), length(tangent) > 0.001);
    let morph = smoothstep(0.0, 0.22, t);
    let tail = params.shape.y * sin(t * 3.14159265) * morph;
    let radius = params.shape.x * mix(1.0, 0.65, t);
    let half_size = mix(extent * 0.5, max(extent * 0.5, vec2<f32>(radius * 3.0 + tail)), morph);
    let corners = array<vec2<f32>, 4>(vec2<f32>(-1.0,-1.0), vec2<f32>(1.0,-1.0), vec2<f32>(-1.0,1.0), vec2<f32>(1.0,1.0));
    let local = corners[vertex] * half_size;
    var output: VertexOutput;
    output.position = vec4<f32>((position + local) / params.viewport.xy * vec2<f32>(2.0,-2.0) + vec2<f32>(-1.0,1.0), 0.0, 1.0);
    output.local = local;
    output.home = home;
    output.axis = axis;
    output.motion = vec4<f32>(morph, 1.0 - smoothstep(0.65, 1.0, t), radius, tail);
    output.color = sampled(home);
    if (morph >= 1.0 && output.color.a == 0.0) {
        output.position = vec4<f32>(2.0, 2.0, 0.0, 1.0);
    }
    output.half_size = extent * 0.5;
    return output;
}

@fragment
fn fragment(input: VertexOutput) -> @location(0) vec4<f32> {
    if (any(input.position.xy < params.bounds.xy) || any(input.position.xy >= params.bounds.xy + params.bounds.zw)) { discard; }
    var fragment_color = vec4<f32>(0.0);
    if (all(input.local >= -input.half_size) && all(input.local < input.half_size)) {
        fragment_color = sampled(input.home + input.local);
    }
    let along = dot(input.local, input.axis);
    let closest = input.axis * clamp(along, -input.motion.w, 0.0);
    let distance = length(input.local - closest) / input.motion.z;
    let glow = exp(-1.8 * distance * distance) * (1.0 - smoothstep(2.0, 3.0, distance));
    let particle = input.color * glow;
    return mix(fragment_color, particle, input.motion.x) * input.motion.y;
}
