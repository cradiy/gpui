struct Spawn {
    range: vec4<u32>,
    line: vec4<f32>,
    velocity: vec4<f32>,
    life: vec4<f32>,
    color: vec4<f32>,
    shape: vec4<f32>,
}
@group(0) @binding(1) var<storage, read> previous: array<Particle>;
@group(0) @binding(2) var<storage, read_write> next: array<Particle>;
@group(0) @binding(3) var<storage, read> spawns: array<Spawn>;

fn random(seed: u32) -> f32 {
    var value = seed;
    value = (value ^ (value >> 16u)) * 0x7feb352du;
    value = (value ^ (value >> 15u)) * 0x846ca68bu;
    value = value ^ (value >> 16u);
    return f32(value >> 8u) / 16777216.0;
}

@compute @workgroup_size(64)
fn update(@builtin(global_invocation_id) invocation: vec3<u32>) {
    let index = invocation.x;
    let capacity = params.counts.x;
    if (index >= capacity) { return; }
    var particle = previous[index];
    if (params.timing.y > 0.5) {
        particle = Particle(vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0));
    }
    let offset = (index + capacity - params.counts.y) % capacity;
    if (offset < params.counts.z) {
        let emitted = offset + ((params.counts.z - 1u - offset) / capacity) * capacity;
        for (var command = 0u; command < u32(params.timing.z); command += 1u) {
            let spawn = spawns[command];
            if (emitted < spawn.range.x || emitted >= spawn.range.x + spawn.range.y) { continue; }
            let seed = params.counts.w + emitted * 17u;
            let angle = random(seed) * 6.2831853;
            let speed = mix(spawn.velocity.z, spawn.velocity.w, random(seed + 1u));
            let along = (f32(emitted - spawn.range.x) + random(seed + 2u)) / f32(spawn.range.y);
            let position = mix(spawn.line.xy, spawn.line.zw, along);
            let velocity = spawn.velocity.xy + vec2<f32>(cos(angle), sin(angle)) * speed;
            particle = Particle(
                vec4<f32>(position, velocity),
                vec4<f32>(0.0, mix(spawn.life.x, spawn.life.y, random(seed + 3u)),
                    mix(spawn.life.z, spawn.life.w, random(seed + 4u)), spawn.shape.x),
                spawn.color,
            );
            next[index] = particle;
            return;
        }
    }
    let elapsed = max(params.timing.x, 0.0);
    particle.life.x += elapsed;
    if (particle.life.x >= particle.life.y) {
        particle.life.y = 0.0;
        next[index] = particle;
        return;
    }
    let delta = min(elapsed, 0.13333334);
    let steps = max(1u, u32(ceil(delta * 60.0)));
    let dt = delta / f32(steps);
    for (var step = 0u; step < steps; step += 1u) {
        let direction = params.field.xy - particle.motion.xy;
        let distance = length(direction);
        let weight = 1.0 - smoothstep(0.0, max(params.field.w, 1.0), distance);
        let acceleration = params.acceleration.xy
            + direction / max(distance, 8.0) * params.field.z * weight;
        let velocity = (particle.motion.zw + acceleration * dt) * exp(-params.acceleration.z * dt);
        particle.motion = vec4<f32>(particle.motion.xy + velocity * dt, velocity);
    }
    next[index] = particle;
}
