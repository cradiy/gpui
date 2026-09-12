use crate::{
    Camera, CameraError, Projection,
    math::{cross, dot, sub},
};
use gpui::{Bounds, MouseButton, Pixels, Point};
use std::{fmt, ops::RangeInclusive, time::Duration};

mod damping;
use damping::Motion;

/// Camera interaction settings. Angles are in radians and distances
/// are in scene units. `None` disables a drag binding.
#[derive(Clone, Debug)]
pub struct OrbitSettings {
    pub orbit_button: Option<MouseButton>,
    pub pan_button: Option<MouseButton>,
    pub dolly_button: Option<MouseButton>,
    /// Radians per logical pointer pixel.
    pub orbit_speed: f32,
    pub pan_speed: f32,
    /// Logarithmic distance/size change per wheel or dolly-drag pixel.
    pub zoom_speed: f32,
    pub distance: RangeInclusive<f32>,
    pub pitch: RangeInclusive<f32>,
    pub orthographic_size: RangeInclusive<f32>,
    pub vertical_fov: RangeInclusive<f32>,
}
impl Default for OrbitSettings {
    fn default() -> Self {
        Self {
            orbit_button: Some(MouseButton::Right),
            pan_button: Some(MouseButton::Middle),
            dolly_button: None,
            orbit_speed: 0.008,
            pan_speed: 1.,
            zoom_speed: 0.002,
            distance: 0.05..=10_000.,
            pitch: -1.5..=1.5,
            orthographic_size: 0.001..=10_000.,
            vertical_fov: 0.05..=3.,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrbitError {
    Camera(CameraError),
    InvalidSettings,
    InvalidInput,
}
impl fmt::Display for OrbitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Camera(error) => error.fmt(f),
            Self::InvalidSettings => f.write_str("camera controls require valid limits, nonnegative speeds, distinct drag bindings, and a nonzero damping half-life"),
            Self::InvalidInput => f.write_str("camera control input must be finite; scale factors must be positive"),
        }
    }
}
impl std::error::Error for OrbitError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Camera(error) => Some(error),
            _ => None,
        }
    }
}
impl From<CameraError> for OrbitError {
    fn from(error: CameraError) -> Self {
        Self::Camera(error)
    }
}

#[derive(Clone, Copy)]
enum Gesture {
    Orbit,
    Pan,
    Dolly,
}
#[derive(Clone, Copy)]
struct Drag {
    button: MouseButton,
    gesture: Gesture,
    previous: Point<Pixels>,
    viewport: Bounds<Pixels>,
}

/// Camera-owned orbit, target-plane pan, dolly, and optical zoom without a window
/// or animation clock. The caller owns event dispatch and pointer capture.
pub struct OrbitController {
    camera: Camera,
    settings: OrbitSettings,
    drag: Option<Drag>,
    damping: Option<Duration>,
    motion: Option<Motion>,
}

impl OrbitController {
    pub fn new(camera: Camera) -> Result<Self, OrbitError> {
        validate_camera(camera)?;
        Ok(Self {
            camera,
            settings: OrbitSettings::default(),
            drag: None,
            damping: None,
            motion: None,
        })
    }
    pub fn camera(&self) -> Camera {
        self.motion.map_or(self.camera, |motion| motion.current)
    }
    /// Input destination, which may lead the displayed camera while damping.
    pub fn target_camera(&self) -> Camera {
        self.camera
    }
    /// Exponential response half-life. `None` applies input immediately.
    pub fn damping(&self) -> Option<Duration> {
        self.damping
    }
    /// Sets a nonzero response half-life and freezes any pending movement at the
    /// displayed pose. Active gestures are canceled. The default is `None`.
    pub fn set_damping(&mut self, half_life: Option<Duration>) -> Result<(), OrbitError> {
        if half_life.is_some_and(|duration| duration.is_zero()) {
            return Err(OrbitError::InvalidSettings);
        }
        self.cancel_drag();
        self.damping = half_life;
        Ok(())
    }
    /// Whether another time step is needed. A stationary held drag is not animation.
    pub fn is_animating(&self) -> bool {
        self.motion.is_some()
    }
    /// Advances the response by caller-owned elapsed time. Returns whether the
    /// displayed camera changed. Zero time is a no-op; after sixteen half-lives
    /// the destination is exact and `is_animating()` is false. An unrepresentable
    /// intermediate pose stops movement at the last valid displayed camera.
    pub fn advance(&mut self, elapsed: Duration) -> Result<bool, OrbitError> {
        let Some(mut motion) = self.motion else {
            return Ok(false);
        };
        if elapsed.is_zero() {
            return Ok(false);
        }
        let before = motion.current;
        let half_life = self.damping.expect("motion requires damping");
        motion.elapsed_nanos += elapsed.as_nanos();
        if motion.elapsed_nanos >= half_life.as_nanos() * 16 {
            self.motion = None;
            return Ok(before != self.camera);
        }
        let half_lives = motion.elapsed_nanos as f64 / half_life.as_nanos() as f64;
        match motion.sample(
            self.camera,
            -(-half_lives * std::f64::consts::LN_2).exp_m1(),
        ) {
            Ok(next) => {
                motion.current = next;
                self.motion = (next != self.camera).then_some(motion);
                Ok(before != next)
            }
            Err(error) => {
                self.cancel_drag();
                Err(error)
            }
        }
    }
    pub fn settings(&self) -> &OrbitSettings {
        &self.settings
    }
    pub fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }
    pub fn drag_button(&self) -> Option<MouseButton> {
        self.drag.map(|drag| drag.button)
    }

    /// Preserves the supplied pose exactly and cancels an active gesture.
    /// Limits constrain subsequent input; they do not snap an externally supplied pose.
    pub fn set_camera(&mut self, camera: Camera) -> Result<(), OrbitError> {
        validate_camera(camera)?;
        self.cancel_drag();
        self.camera = camera;
        Ok(())
    }
    /// Validates settings atomically and cancels an active gesture on success.
    pub fn set_settings(&mut self, settings: OrbitSettings) -> Result<(), OrbitError> {
        let valid = |range: &RangeInclusive<f32>| {
            range.start().is_finite() && range.end().is_finite() && range.start() <= range.end()
        };
        let bindings = [
            settings.orbit_button,
            settings.pan_button,
            settings.dolly_button,
        ];
        if ![
            settings.orbit_speed,
            settings.pan_speed,
            settings.zoom_speed,
        ]
        .iter()
        .all(|v| v.is_finite() && *v >= 0.)
            || ![
                &settings.distance,
                &settings.pitch,
                &settings.orthographic_size,
                &settings.vertical_fov,
            ]
            .into_iter()
            .all(valid)
            || *settings.distance.start() <= 0.
            || *settings.orthographic_size.start() <= 0.
            || *settings.vertical_fov.start() <= 0.
            || *settings.vertical_fov.end() >= std::f32::consts::PI
            || *settings.pitch.start() < -std::f32::consts::FRAC_PI_2
            || *settings.pitch.end() > std::f32::consts::FRAC_PI_2
            || (0..3)
                .any(|i| (i + 1..3).any(|j| bindings[i].is_some() && bindings[i] == bindings[j]))
        {
            return Err(OrbitError::InvalidSettings);
        }
        self.settings = settings;
        self.cancel_drag();
        Ok(())
    }

    /// Starts a configured drag only inside the viewport. Returns whether input
    /// was claimed, not whether the camera moved. Another drag cannot replace it.
    pub fn begin_drag(
        &mut self,
        button: MouseButton,
        position: Point<Pixels>,
        viewport: Bounds<Pixels>,
    ) -> Result<bool, OrbitError> {
        if self.drag.is_some() {
            return Ok(false);
        }
        let gesture = if self.settings.orbit_button == Some(button) {
            Gesture::Orbit
        } else if self.settings.pan_button == Some(button) {
            Gesture::Pan
        } else if self.settings.dolly_button == Some(button) {
            Gesture::Dolly
        } else {
            return Ok(false);
        };
        self.camera.screen_to_ray(viewport, position)?;
        if position.x < viewport.origin.x
            || position.y < viewport.origin.y
            || position.x >= viewport.right()
            || position.y >= viewport.bottom()
        {
            return Ok(false);
        }
        self.stop_motion();
        self.drag = Some(Drag {
            button,
            gesture,
            previous: position,
            viewport,
        });
        Ok(true)
    }

    /// Updates the owned gesture, including positions outside the viewport.
    /// Button mismatch, viewport changes, or invalid input cancel the gesture.
    pub fn update_drag(
        &mut self,
        position: Point<Pixels>,
        pressed: Option<MouseButton>,
        viewport: Bounds<Pixels>,
    ) -> Result<bool, OrbitError> {
        let Some(drag) = self.drag else {
            return Ok(false);
        };
        if pressed != Some(drag.button) || viewport != drag.viewport {
            self.cancel_drag();
            return Ok(false);
        }
        let delta = position - drag.previous;
        if ![f32::from(delta.x), f32::from(delta.y)]
            .iter()
            .all(|value| value.is_finite())
        {
            self.cancel_drag();
            return Err(OrbitError::InvalidInput);
        }
        let result = match drag.gesture {
            Gesture::Orbit => self.orbit_by([f32::from(delta.x), f32::from(delta.y)]),
            Gesture::Pan => self.pan_by(viewport, delta),
            Gesture::Dolly => self.drag_dolly(f32::from(delta.y)),
        };
        if result.is_ok() {
            self.drag = Some(Drag {
                previous: position,
                ..drag
            });
        } else {
            self.cancel_drag();
        }
        result
    }
    /// Releases the owned gesture without stopping pending motion. An unrelated
    /// button release does not end the gesture.
    pub fn end_drag(&mut self, button: MouseButton) -> bool {
        if self.drag_button() != Some(button) {
            return false;
        }
        self.drag = None;
        true
    }
    /// Cancels gesture ownership and freezes pending motion at the displayed pose.
    pub fn cancel_drag(&mut self) {
        self.drag = None;
        self.stop_motion();
    }
    fn stop_motion(&mut self) {
        self.camera = self.camera();
        self.motion = None;
    }

    /// Applies logical pointer displacement around the current target and up axis.
    pub fn orbit_by(&mut self, delta: [f32; 2]) -> Result<bool, OrbitError> {
        if !delta.iter().all(|v| v.is_finite()) {
            return Err(OrbitError::InvalidInput);
        }
        if delta == [0.; 2] || self.settings.orbit_speed == 0. {
            return Ok(false);
        }
        let camera = self.camera;
        let [fallback_right, _, backward] = camera.axes()?;
        let up = crate::Ray::new([0.; 3], camera.up)
            .map_err(|_| CameraError::InvalidView)?
            .direction();
        let perpendicular = cross(up, backward);
        let right = if dot(perpendicular, perpendicular) > 1e-12 {
            crate::Ray::new([0.; 3], perpendicular)
                .map_err(|_| CameraError::InvalidView)?
                .direction()
        } else {
            fallback_right
        };
        let yaw = -delta[0] * self.settings.orbit_speed;
        let pitch_delta = delta[1] * self.settings.orbit_speed;
        if !yaw.is_finite() || !pitch_delta.is_finite() {
            return Err(OrbitError::InvalidInput);
        }
        let current_pitch = dot(backward, up).clamp(-1., 1.).asin();
        let pitch = constrain(
            current_pitch,
            f64::from(current_pitch) + f64::from(pitch_delta),
            &self.settings.pitch,
        );
        if yaw == 0. && pitch == current_pitch {
            return Ok(false);
        }
        let backward = rotate(backward, up, yaw);
        let right = rotate(right, up, yaw);
        let direction = rotate(backward, right, -(pitch - current_pitch));
        let distance = distance(camera);
        let mut next = camera;
        next.eye = std::array::from_fn(|i| camera.target[i] + direction[i] * distance);
        self.apply(next)
    }

    /// Moves eye and target together so points on the target plane follow the
    /// pointer displacement. Uses the actual projection, viewport aspect, and roll.
    pub fn pan_by(
        &mut self,
        viewport: Bounds<Pixels>,
        delta: Point<Pixels>,
    ) -> Result<bool, OrbitError> {
        if ![f32::from(delta.x), f32::from(delta.y)]
            .iter()
            .all(|v| v.is_finite())
        {
            return Err(OrbitError::InvalidInput);
        }
        let previous = viewport.center();
        let next = previous + delta * self.settings.pan_speed;
        let a = self.camera.screen_to_ray(viewport, previous)?;
        let b = self.camera.screen_to_ray(viewport, next)?;
        let normal = self.camera.axes()?[2];
        let intersect = |ray: crate::Ray| {
            ray.at(dot(sub(self.camera.target, ray.origin()), normal) / dot(ray.direction(), normal))
        };
        let shift = sub(intersect(a), intersect(b));
        let mut camera = self.camera;
        for (i, shift) in shift.into_iter().enumerate() {
            camera.eye[i] += shift;
            camera.target[i] += shift;
        }
        self.apply(camera)
    }

    /// Multiplies eye-to-target distance. Projection and target remain unchanged.
    pub fn dolly(&mut self, factor: f32) -> Result<bool, OrbitError> {
        valid_factor(factor)?;
        let current = distance(self.camera);
        let distance = constrain(
            current,
            f64::from(current) * f64::from(factor),
            &self.settings.distance,
        );
        if distance == current {
            return Ok(false);
        }
        let backward = self.camera.axes()?[2];
        let mut camera = self.camera;
        camera.eye = std::array::from_fn(|i| camera.target[i] + backward[i] * distance);
        self.apply(camera)
    }

    /// Multiplies orthographic span or perspective tangent half-FOV. Factors > 1
    /// zoom out. Eye and target remain unchanged, unlike dolly.
    pub fn zoom(&mut self, factor: f32) -> Result<bool, OrbitError> {
        valid_factor(factor)?;
        if factor == 1. {
            return Ok(false);
        }
        let mut camera = self.camera;
        camera.projection = match camera.projection {
            Projection::Orthographic { vertical_size } => Projection::Orthographic {
                vertical_size: constrain(
                    vertical_size,
                    f64::from(vertical_size) * f64::from(factor),
                    &self.settings.orthographic_size,
                ),
            },
            Projection::Perspective { vertical_fov } => Projection::Perspective {
                vertical_fov: constrain(
                    vertical_fov,
                    ((f64::from(vertical_fov) * 0.5).tan() * f64::from(factor)).atan() * 2.,
                    &self.settings.vertical_fov,
                ),
            },
        };
        self.apply(camera)
    }

    /// Positive pixels zoom out: dolly in perspective, span zoom in orthographic.
    /// Wheel input is ignored during a drag. Convert line deltas before calling.
    pub fn scroll(&mut self, pixels: f32) -> Result<bool, OrbitError> {
        if self.is_dragging() {
            return Ok(false);
        }
        if matches!(self.camera.projection, Projection::Orthographic { .. }) {
            self.zoom(self.factor(pixels)?)
        } else {
            self.drag_dolly(pixels)
        }
    }
    fn factor(&self, pixels: f32) -> Result<f32, OrbitError> {
        if !pixels.is_finite() {
            return Err(OrbitError::InvalidInput);
        }
        Ok((f64::from(pixels) * f64::from(self.settings.zoom_speed))
            .clamp(-60., 60.)
            .exp() as f32)
    }
    fn drag_dolly(&mut self, pixels: f32) -> Result<bool, OrbitError> {
        self.dolly(self.factor(pixels)?)
    }
    fn apply(&mut self, camera: Camera) -> Result<bool, OrbitError> {
        validate_camera(camera)?;
        let changed = self.camera != camera;
        if changed && self.damping.is_some() {
            let displayed = self.camera();
            self.motion = (displayed != camera).then(|| Motion::new(displayed));
        }
        self.camera = camera;
        Ok(changed)
    }
}

fn distance(camera: Camera) -> f32 {
    camera
        .eye
        .iter()
        .zip(camera.target)
        .map(|(a, b)| (f64::from(*a) - f64::from(b)).powi(2))
        .sum::<f64>()
        .sqrt() as f32
}
fn validate_camera(camera: Camera) -> Result<(), OrbitError> {
    camera.view_projection(1.)?;
    if !distance(camera).is_finite() || distance(camera) <= 0. {
        return Err(CameraError::Unrepresentable.into());
    }
    Ok(())
}
fn valid_factor(factor: f32) -> Result<(), OrbitError> {
    if factor.is_finite() && factor > 0. {
        Ok(())
    } else {
        Err(OrbitError::InvalidInput)
    }
}
fn constrain(current: f32, candidate: f64, limit: &RangeInclusive<f32>) -> f32 {
    candidate.clamp(
        f64::from(current.min(*limit.start())),
        f64::from(current.max(*limit.end())),
    ) as f32
}
fn rotate(v: [f32; 3], axis: [f32; 3], angle: f32) -> [f32; 3] {
    let cross = cross(axis, v);
    let projection = dot(axis, v);
    std::array::from_fn(|i| {
        v[i] * angle.cos() + cross[i] * angle.sin() + axis[i] * projection * (1. - angle.cos())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{point, px, size};

    fn viewport() -> Bounds<Pixels> {
        Bounds::new(point(px(80.), px(50.)), size(px(800.), px(450.)))
    }
    fn close(a: f32, b: f32) {
        assert!((a - b).abs() < 0.0002, "{a} != {b}");
    }

    #[test]
    fn orbit_preserves_target_radius_and_custom_up() {
        let camera = Camera {
            eye: [8., 3., 7.],
            target: [2., -1., 3.],
            up: [0., 0., 1.],
            ..Camera::default()
        };
        let mut controls = OrbitController::new(camera).unwrap();
        assert!(!controls.orbit_by([0., 0.]).unwrap());
        assert!(!controls.zoom(1.).unwrap());
        assert!(!controls.scroll(0.).unwrap());
        assert_eq!(controls.camera(), camera);
        let pitch = dot(camera.axes().unwrap()[2], camera.up).asin();
        assert!(controls.orbit_by([25., 10.]).unwrap());
        let next = controls.camera();
        assert_eq!(next.target, camera.target);
        assert_eq!(next.up, camera.up);
        close(distance(next), distance(camera));
        close(dot(next.axes().unwrap()[2], camera.up).asin(), pitch + 0.08);
        for _ in 0..1000 {
            controls.orbit_by([1., 0.]).unwrap();
        }
        close(distance(controls.camera()), distance(camera));
    }

    #[test]
    fn orbit_leaves_poles_and_respects_pitch_limits() {
        for pitch in [
            1.55_f32,
            std::f32::consts::FRAC_PI_2,
            -std::f32::consts::FRAC_PI_2,
        ] {
            let mut controls = OrbitController::new(Camera {
                eye: [0., pitch.sin() * 6., pitch.cos() * 6.],
                ..Camera::default()
            })
            .unwrap();
            controls.orbit_by([0., -pitch.signum() * 20.]).unwrap();
            close(
                controls.camera().axes().unwrap()[2][1].asin(),
                pitch - pitch.signum() * 0.16,
            );
        }
        let mut controls = OrbitController::new(Camera::default()).unwrap();
        controls
            .set_settings(OrbitSettings {
                pitch: -0.5..=0.6,
                ..Default::default()
            })
            .unwrap();
        controls.orbit_by([20., 1000.]).unwrap();
        close(controls.camera().axes().unwrap()[2][1].asin(), 0.6);
        controls.orbit_by([0., -1000.]).unwrap();
        close(controls.camera().axes().unwrap()[2][1].asin(), -0.5);
    }

    #[test]
    fn pan_tracks_logical_pointer_in_both_projections() {
        for projection in [
            Projection::default(),
            Projection::Orthographic { vertical_size: 8. },
        ] {
            for viewport in [
                viewport(),
                Bounds::new(point(px(25.), px(100.)), size(px(320.), px(900.))),
            ] {
                let camera = Camera {
                    eye: [8., 3., 7.],
                    target: [2., -1., 3.],
                    up: [0.4, 1., 0.2],
                    projection,
                    ..Camera::default()
                };
                let mut controls = OrbitController::new(camera).unwrap();
                let right = camera.axes().unwrap()[0];
                let world = std::array::from_fn(|i| camera.target[i] + right[i]);
                let before = camera
                    .world_to_screen(viewport, world)
                    .unwrap()
                    .unwrap()
                    .position;
                let delta = point(px(38.), px(-27.));
                controls.pan_by(viewport, delta).unwrap();
                let next = controls.camera();
                let after = next
                    .world_to_screen(viewport, world)
                    .unwrap()
                    .unwrap()
                    .position;
                close(f32::from(after.x - before.x), 38.);
                close(f32::from(after.y - before.y), -27.);
                close(distance(next), distance(camera));
                for i in 0..3 {
                    close(
                        next.eye[i] - camera.eye[i],
                        next.target[i] - camera.target[i],
                    );
                }
                assert_eq!(next.projection, projection);
                assert!(!controls.pan_by(viewport, point(px(0.), px(0.))).unwrap());
            }
        }
    }

    #[test]
    fn dolly_and_zoom_have_distinct_optics() {
        for projection in [
            Projection::default(),
            Projection::Orthographic { vertical_size: 8. },
        ] {
            let camera = Camera {
                projection,
                ..Camera::orbit(0.3, 0.2, 6.)
            };
            let mut controls = OrbitController::new(camera).unwrap();
            controls.dolly(0.5).unwrap();
            close(distance(controls.camera()), 3.);
            assert_eq!(controls.camera().projection, projection);
            assert_eq!(controls.camera().target, camera.target);
            controls.set_camera(camera).unwrap();
            let before = camera
                .world_to_screen(viewport(), [1., 0., 0.])
                .unwrap()
                .unwrap()
                .position;
            controls.zoom(0.5).unwrap();
            let next = controls.camera();
            assert_eq!(next.eye, camera.eye);
            assert_eq!(next.target, camera.target);
            assert_eq!((next.near, next.far), (camera.near, camera.far));
            let after = next
                .world_to_screen(viewport(), [1., 0., 0.])
                .unwrap()
                .unwrap()
                .position;
            close(
                f32::from(after.x - viewport().center().x),
                2. * f32::from(before.x - viewport().center().x),
            );
            controls.set_camera(camera).unwrap();
            controls.scroll(100.).unwrap();
            if matches!(projection, Projection::Orthographic { .. }) {
                assert_eq!(controls.camera().eye, camera.eye);
                assert_ne!(controls.camera().projection, projection);
            } else {
                assert!(distance(controls.camera()) > distance(camera));
                assert_eq!(controls.camera().projection, projection);
            }
        }
    }

    #[test]
    fn limits_do_not_snap_external_poses_and_invalid_updates_are_atomic() {
        let mut controls = OrbitController::new(Camera::default()).unwrap();
        controls
            .set_settings(OrbitSettings {
                distance: 1.0..=4.,
                orthographic_size: 1.0..=10.,
                vertical_fov: 0.2..=1.5,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(controls.camera(), Camera::default());
        assert!(!controls.dolly(2.).unwrap());
        controls.dolly(0.8).unwrap();
        close(distance(controls.camera()), 4.8);
        controls.dolly(f32::MIN_POSITIVE).unwrap();
        close(distance(controls.camera()), 1.);
        controls.dolly(f32::MAX).unwrap();
        close(distance(controls.camera()), 4.);
        controls.zoom(f32::MAX).unwrap();
        assert_eq!(
            controls.camera().projection,
            Projection::Perspective { vertical_fov: 1.5 }
        );
        controls.zoom(f32::MIN_POSITIVE).unwrap();
        assert_eq!(
            controls.camera().projection,
            Projection::Perspective { vertical_fov: 0.2 }
        );
        let camera = Camera {
            projection: Projection::Orthographic { vertical_size: 5. },
            ..controls.camera()
        };
        controls.set_camera(camera).unwrap();
        controls.scroll(f32::MAX).unwrap();
        assert_eq!(
            controls.camera().projection,
            Projection::Orthographic { vertical_size: 10. }
        );
        controls.scroll(-f32::MAX).unwrap();
        assert_eq!(
            controls.camera().projection,
            Projection::Orthographic { vertical_size: 1. }
        );
        controls
            .begin_drag(MouseButton::Right, viewport().center(), viewport())
            .unwrap();
        let before = controls.camera();
        assert!(
            controls
                .set_camera(Camera {
                    eye: before.target,
                    ..before
                })
                .is_err()
        );
        assert!(
            controls
                .set_settings(OrbitSettings {
                    pan_button: Some(MouseButton::Right),
                    ..Default::default()
                })
                .is_err()
        );
        assert!(controls.dolly(f32::NAN).is_err());
        assert!(controls.zoom(-1.).is_err());
        assert_eq!(controls.camera(), before);
        assert!(controls.is_dragging());
        controls.set_camera(camera).unwrap();
        assert!(!controls.is_dragging());
    }

    #[test]
    fn gestures_keep_button_ownership_and_cancel_stale_input() {
        let mut controls = OrbitController::new(Camera::default()).unwrap();
        let bounds = viewport();
        let start = bounds.center();
        assert!(
            !controls
                .begin_drag(MouseButton::Left, start, bounds)
                .unwrap()
        );
        assert!(
            !controls
                .begin_drag(MouseButton::Right, point(px(0.), px(0.)), bounds)
                .unwrap()
        );
        assert!(
            controls
                .begin_drag(MouseButton::Right, start, bounds)
                .unwrap()
        );
        assert!(
            !controls
                .begin_drag(MouseButton::Middle, start, bounds)
                .unwrap()
        );
        assert!(!controls.end_drag(MouseButton::Left));
        assert!(!controls.scroll(80.).unwrap());
        assert!(
            controls
                .update_drag(point(px(0.), px(0.)), Some(MouseButton::Right), bounds)
                .unwrap()
        );
        assert!(controls.is_dragging());
        assert!(!controls.update_drag(start, None, bounds).unwrap());
        assert!(!controls.is_dragging());
        controls
            .begin_drag(MouseButton::Middle, start, bounds)
            .unwrap();
        let before = controls.camera();
        assert!(
            !controls
                .update_drag(
                    start,
                    Some(MouseButton::Middle),
                    Bounds::new(bounds.origin, size(px(400.), px(300.)))
                )
                .unwrap()
        );
        assert_eq!(controls.camera(), before);
        assert!(!controls.is_dragging());
        controls
            .begin_drag(MouseButton::Middle, start, bounds)
            .unwrap();
        assert!(
            controls
                .update_drag(
                    point(px(f32::NAN), px(10.)),
                    Some(MouseButton::Middle),
                    bounds
                )
                .is_err()
        );
        assert_eq!(controls.camera(), before);
        assert!(!controls.is_dragging());
        controls
            .begin_drag(MouseButton::Right, start, bounds)
            .unwrap();
        controls.set_settings(OrbitSettings::default()).unwrap();
        assert!(!controls.is_dragging());
        assert!(
            !controls
                .update_drag(start, Some(MouseButton::Right), bounds)
                .unwrap()
        );
        controls
            .begin_drag(MouseButton::Right, start, bounds)
            .unwrap();
        assert!(controls.end_drag(MouseButton::Right));
        assert!(!controls.is_dragging());
    }

    #[test]
    fn remapped_pan_and_dolly_bindings_drive_the_selected_operation() {
        let mut controls = OrbitController::new(Camera::default()).unwrap();
        controls
            .set_settings(OrbitSettings {
                orbit_button: None,
                pan_button: Some(MouseButton::Right),
                dolly_button: Some(MouseButton::Left),
                ..Default::default()
            })
            .unwrap();
        let bounds = viewport();
        let start = bounds.center();
        assert!(
            !controls
                .begin_drag(MouseButton::Middle, start, bounds)
                .unwrap()
        );
        controls
            .begin_drag(MouseButton::Right, start, bounds)
            .unwrap();
        let before = controls.camera();
        controls
            .update_drag(
                start + point(px(50.), px(0.)),
                Some(MouseButton::Right),
                bounds,
            )
            .unwrap();
        assert_ne!(controls.camera().target, before.target);
        close(distance(controls.camera()), distance(before));
        controls.end_drag(MouseButton::Right);
        controls
            .begin_drag(MouseButton::Left, start, bounds)
            .unwrap();
        let before = controls.camera();
        controls
            .update_drag(
                start + point(px(0.), px(80.)),
                Some(MouseButton::Left),
                bounds,
            )
            .unwrap();
        assert!(distance(controls.camera()) > distance(before));
        assert_eq!(controls.camera().target, before.target);
        assert_eq!(controls.camera().projection, before.projection);
        let before = controls.camera();
        assert!(
            controls
                .update_drag(
                    point(px(f32::NAN), start.y),
                    Some(MouseButton::Left),
                    bounds
                )
                .is_err()
        );
        assert!(!controls.is_dragging());
        assert_eq!(controls.camera(), before);
    }
}
