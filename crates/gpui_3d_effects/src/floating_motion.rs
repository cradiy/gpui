use std::{f64::consts::TAU, time::Duration};

use gpui_3d::Transform;

/// Caller-clocked floating and axial rotation for any 3D object.
/// Sampling is independent of frame rate and does not mutate the mesh.
#[derive(Clone, Copy, Debug)]
pub struct FloatingMotion {
    amplitude: f32,
    period: Duration,
    spin: f32,
}

impl Default for FloatingMotion {
    fn default() -> Self {
        Self {
            amplitude: 0.08,
            period: Duration::from_secs(5),
            spin: 0.2,
        }
    }
}

impl FloatingMotion {
    /// Vertical travel from the center in scene units. Invalid values become zero.
    pub fn amplitude(mut self, amplitude: f32) -> Self {
        self.amplitude = if amplitude.is_finite() {
            amplitude.max(0.)
        } else {
            0.
        };
        self
    }

    /// Duration of one vertical cycle. Zero disables vertical motion.
    pub fn period(mut self, period: Duration) -> Self {
        self.period = period;
        self
    }

    /// Y-axis angular velocity in radians per second. Negative values reverse it.
    pub fn spin(mut self, radians_per_second: f32) -> Self {
        self.spin = if radians_per_second.is_finite() {
            radians_per_second
        } else {
            0.
        };
        self
    }

    /// Returns an origin-centered pose. Apply any application placement separately.
    pub fn sample(self, elapsed: Duration) -> Transform {
        let seconds = elapsed.as_secs_f64();
        let height = if self.period.is_zero() {
            0.
        } else {
            let phase = (seconds % self.period.as_secs_f64()) / self.period.as_secs_f64();
            self.amplitude * (phase * TAU).sin() as f32
        };
        Transform {
            position: [0., height, 0.],
            rotation: [
                0.,
                (seconds * f64::from(self.spin)).rem_euclid(TAU) as f32,
                0.,
            ],
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sampling_is_periodic_bounded_and_handles_disabled_motion() {
        let motion = FloatingMotion::default()
            .amplitude(0.35)
            .period(Duration::from_secs(7))
            .spin(-0.8);
        for ms in [0, 15, 1725, 4534, 6999] {
            let a = motion.sample(Duration::from_millis(ms));
            let b = motion.sample(Duration::from_millis(ms + 7000));
            assert!((a.position[1] - b.position[1]).abs() < 0.00001);
            assert!(a.position[1].abs() <= 0.35);
            assert!(a.rotation[1].is_finite());
        }
        let disabled = motion
            .period(Duration::ZERO)
            .spin(f32::NAN)
            .sample(Duration::MAX);
        assert_eq!(disabled.position, [0.; 3]);
        assert_eq!(disabled.rotation, [0.; 3]);
    }
}
