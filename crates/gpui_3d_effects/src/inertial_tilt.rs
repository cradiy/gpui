use std::time::Duration;

use gpui_3d::Transform;

/// A damped angular spring for pointer-driven surfaces and objects.
/// Targets use X/Y Euler angles in radians. Changing the target preserves velocity;
/// a zero target returns to rest. Poses have zero translation and unit scale.
///
/// ```
/// use std::time::Duration;
/// use gpui_3d_effects::InertialTilt;
///
/// let mut tilt = InertialTilt::default();
/// tilt.set_target([0.1, -0.2]);
/// let pose = tilt.advance(Duration::from_secs_f64(1.0 / 60.0));
/// tilt.set_target([0.0, 0.0]);
/// // Keep advancing while !tilt.is_settled().
/// ```
#[derive(Clone, Copy, Debug)]
pub struct InertialTilt {
    limit: f64,
    frequency: f64,
    damping: f64,
    target: [f64; 2],
    angles: [f64; 2],
    velocity: [f64; 2],
}

impl Default for InertialTilt {
    fn default() -> Self {
        Self {
            limit: 0.3,
            frequency: std::f64::consts::TAU / 0.65,
            damping: 0.75,
            target: [0.; 2],
            angles: [0.; 2],
            velocity: [0.; 2],
        }
    }
}

impl InertialTilt {
    /// Limits each target angle. The spring may briefly overshoot the target.
    /// Panics unless radians is finite and in (0, PI / 2].
    #[track_caller]
    pub fn max_tilt(mut self, radians: f32) -> Self {
        assert!(radians.is_finite() && radians > 0. && radians <= std::f32::consts::FRAC_PI_2);
        self.limit = f64::from(radians);
        self.target = self.target.map(|v| v.clamp(-self.limit, self.limit));
        self
    }

    /// Sets the spring's natural period. Shorter periods respond faster.
    /// Defaults to 650 ms. Panics if the duration is zero.
    #[track_caller]
    pub fn response(mut self, period: Duration) -> Self {
        assert!(!period.is_zero(), "tilt response must be nonzero");
        self.frequency = std::f64::consts::TAU / period.as_secs_f64();
        self
    }

    /// Sets the damping ratio in (0, 1]. One is critically damped; lower values
    /// allow more rebound. Defaults to 0.75. Panics outside the finite range.
    #[track_caller]
    pub fn damping(mut self, ratio: f32) -> Self {
        assert!(ratio.is_finite() && ratio > 0. && ratio <= 1.);
        self.damping = f64::from(ratio);
        self
    }

    /// Sets the bounded X/Y target without changing the current pose or velocity.
    /// Non-finite components select zero.
    pub fn set_target(&mut self, angles: [f32; 2]) {
        self.target = angles.map(|v| {
            if v.is_finite() {
                f64::from(v).clamp(-self.limit, self.limit)
            } else {
                0.
            }
        });
    }

    /// Advances the spring for a constant target using elapsed time, without
    /// frame-rate-dependent integration. Zero duration leaves the pose unchanged.
    pub fn advance(&mut self, elapsed: Duration) -> Transform {
        if !elapsed.is_zero() && !self.is_settled() {
            let dt = elapsed.as_secs_f64();
            let decay_rate = self.frequency * self.damping;
            let frequency = self.frequency * (1. - self.damping * self.damping).sqrt();
            let decay = (-decay_rate * dt).exp();
            let phase = frequency * dt;
            let cosine = phase.cos();
            let sine_over_frequency = if frequency == 0. {
                dt
            } else {
                phase.sin() / frequency
            };
            for axis in 0..2 {
                let offset = self.angles[axis] - self.target[axis];
                let velocity = self.velocity[axis];
                self.angles[axis] = self.target[axis]
                    + decay
                        * (offset * cosine
                            + (velocity + decay_rate * offset) * sine_over_frequency);
                self.velocity[axis] = decay
                    * (velocity * cosine
                        - (decay_rate * velocity + self.frequency * self.frequency * offset)
                            * sine_over_frequency);
                if (self.angles[axis] - self.target[axis]).abs() < 1e-5
                    && self.velocity[axis].abs() < 1e-5
                {
                    self.angles[axis] = self.target[axis];
                    self.velocity[axis] = 0.;
                }
            }
        }
        self.pose()
    }

    /// Returns the current pose without advancing time.
    pub fn pose(&self) -> Transform {
        Transform {
            rotation: [self.angles[0] as f32, self.angles[1] as f32, 0.],
            ..Default::default()
        }
    }

    /// Whether animation frames can stop until the target changes.
    pub fn is_settled(&self) -> bool {
        self.angles == self.target && self.velocity == [0.; 2]
    }

    /// Immediately clears the target, rotation, and angular velocity.
    pub fn reset(&mut self) {
        self.target = [0.; 2];
        self.angles = [0.; 2];
        self.velocity = [0.; 2];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spring_motion_is_independent_of_frame_partition() {
        for damping in [0.35, 0.75, 0.99999, 1.] {
            let mut whole = InertialTilt::default().damping(damping);
            whole.set_target([0.2, -0.15]);
            let mut partitioned = whole;
            whole.advance(Duration::from_millis(240));
            for _ in 0..24 {
                partitioned.advance(Duration::from_millis(10));
            }
            for axis in 0..2 {
                assert!((whole.angles[axis] - partitioned.angles[axis]).abs() < 1e-10);
                assert!((whole.velocity[axis] - partitioned.velocity[axis]).abs() < 1e-10);
            }
            let pose = whole.pose();
            assert_eq!(pose.position, [0.; 3]);
            assert_eq!(pose.scale, [1.; 3]);
        }
    }

    #[test]
    fn reversing_target_preserves_momentum_and_returns_to_rest() {
        let mut tilt = InertialTilt::default();
        tilt.set_target([0.2, -0.2]);
        tilt.advance(Duration::from_millis(60));
        let before = tilt;
        tilt.set_target([-0.2, 0.2]);
        assert_eq!(tilt.pose().rotation, before.pose().rotation);
        assert_eq!(tilt.velocity, before.velocity);
        tilt.advance(Duration::ZERO);
        assert_eq!(tilt.angles, before.angles);
        tilt.advance(Duration::from_micros(100));
        assert!(tilt.angles[0] > before.angles[0]);
        assert!(tilt.angles[1] < before.angles[1]);
        tilt.set_target([0.; 2]);
        for _ in 0..300 {
            tilt.advance(Duration::from_millis(16));
        }
        assert!(tilt.is_settled());
        assert_eq!(tilt.pose().rotation, [0.; 3]);
    }

    #[test]
    fn input_limits_and_long_pauses_remain_finite() {
        let mut tilt = InertialTilt::default().max_tilt(0.2);
        tilt.set_target([f32::MAX, -f32::MAX]);
        tilt.advance(Duration::MAX);
        assert!(tilt.is_settled());
        assert_eq!(tilt.pose().rotation, [0.2, -0.2, 0.]);
        tilt.set_target([f32::NAN, f32::INFINITY]);
        tilt.advance(Duration::MAX);
        assert!(tilt.is_settled());
        assert_eq!(tilt.pose().rotation, [0.; 3]);
        tilt.set_target([0.1, 0.1]);
        tilt.advance(Duration::from_millis(10));
        tilt.reset();
        assert!(tilt.is_settled());
        assert_eq!(tilt.pose().rotation, [0.; 3]);
    }
}
