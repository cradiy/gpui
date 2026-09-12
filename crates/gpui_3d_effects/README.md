# gpui_3d_effects

Spatial effects for GPUI interfaces, built on `gpui_3d`.

## Floating motion

```rust
use std::time::Duration;
use gpui_3d_effects::FloatingMotion;

let motion = FloatingMotion::default();
let pose = motion.sample(Duration::from_secs_f32(1.5));
```

Apply the pose with `Object::transform`. `.amplitude(...)` sets vertical travel
in scene units, `.period(...)` controls the cycle duration and `.spin(...)`
sets Y-axis radians per second. The returned pose is origin-centered; add any
application placement before applying it. Sampling does not own an animation loop.

## Orbit light

Retain an `OrbitLight` and add its sampled object to a scene:

```rust
use gpui_3d::Scene;
use gpui_3d_effects::OrbitLight;

let orbit = OrbitLight::default();
let scene = Scene::new().object(orbit.object(0.5).scale([1.4; 3]));
```

The phase is in radians. `.color(...)` and `.tilt([x, y])` configure the strand.
The mesh is shared between sampled frames. Geometry in the same scene occludes
the light; ordinary 2D overlays follow GPUI paint order, not the scene's depth
buffer. The strand is unlit and does not cast light onto other objects.

Apply `gpui_effects` Bloom to the transparent viewport for a soft halo. Keep
normal text and controls outside that effect wrapper to preserve their clarity.
The application owns the animation clock and redraw scheduling.

## Example

```sh
cargo run -p gpui_3d_effects --example floating
```

The example applies the effects to simple geometry alongside ordinary 2D
controls. Pause stops the animation. The orbit and glow can be toggled separately.
Inactive windows suspend animation.
