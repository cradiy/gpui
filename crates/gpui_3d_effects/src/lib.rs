//! Reusable spatial effects built on GPUI's low-level 3D renderer.

#[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
mod curve_light;
mod floating_motion;
mod inertial_tilt;
mod layer_stack;
#[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
mod light_sweep;
mod orbit_light;
#[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
mod rim_light;

#[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
pub use curve_light::CurveLight;
pub use floating_motion::FloatingMotion;
pub use inertial_tilt::InertialTilt;
pub use layer_stack::LayerStack;
#[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
pub use light_sweep::LightSweep;
pub use orbit_light::OrbitLight;
#[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
pub use rim_light::RimLight;
