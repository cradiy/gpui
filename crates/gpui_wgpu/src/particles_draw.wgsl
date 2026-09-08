@group(0) @binding(1) var<storage, read> particles: array<Particle>;
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) shape: vec2<f32>,
}
@vertex
fn vertex(@builtin(vertex_index) vertex: u32, @builtin(instance_index) index: u32) -> VertexOutput {
    let particle = particles[index];
    var output: VertexOutput;
    if (particle.life.y <= 0.0 || particle.life.x >= particle.life.y) {
        output.position = vec4<f32>(2.0, 2.0, 0.0, 1.0);
        output.local = vec2<f32>(0.0);
        output.color = vec4<f32>(0.0);
        output.shape = vec2<f32>(1.0, 0.0);
        return output;
    }
    let age = particle.life.x / particle.life.y;
    let radius = max(particle.life.z * mix(1.0, 0.5, age), 0.25);
    let speed = length(particle.motion.zw);
    let axis = select(vec2<f32>(1.0, 0.0), particle.motion.zw / max(speed, 0.001), speed > 0.001);
    let across = vec2<f32>(-axis.y, axis.x);
    let tail = min(speed * particle.life.w, 32.0);
    let corners = array<vec2<f32>, 4>(vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0),
        vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, 1.0));
    let corner = corners[vertex];
    let local = vec2<f32>(corner.x * (radius * 3.0 + tail * 0.5) - tail * 0.5, corner.y * radius * 3.0);
    let position = params.bounds.xy + (particle.motion.xy + axis * local.x + across * local.y) * params.viewport.z;
    output.position = vec4<f32>(position / params.viewport.xy * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
    output.local = local;
    let fade = (1.0 - smoothstep(0.05, 1.0, age)) * smoothstep(0.0, 0.035, particle.life.x);
    output.color = vec4<f32>(particle.color.rgb, particle.color.a * fade * params.viewport.w);
    output.shape = vec2<f32>(radius, tail);
    return output;
}
@fragment
fn fragment(input: VertexOutput) -> @location(0) vec4<f32> {
    let low = max(params.clip.xy, params.bounds.xy);
    let high = min(params.clip.xy + params.clip.zw, params.bounds.xy + params.bounds.zw);
    if (any(input.position.xy < low) || any(input.position.xy >= high)) { discard; }
    let offset = vec2<f32>(input.local.x - clamp(input.local.x, -input.shape.y, 0.0), input.local.y) / input.shape.x;
    let distance = length(offset);
    let alpha = exp(-1.8 * dot(offset, offset)) * (1.0 - smoothstep(2.0, 3.0, distance)) * input.color.a;
    return vec4<f32>(input.color.rgb * alpha, alpha);
}
