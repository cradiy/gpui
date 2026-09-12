use super::rotation::{Rotation, dot, unit};
use std::{f64::consts::PI, fmt};

/// Stateless circular swing cone and signed twist interval around a reference pose.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JointRotationLimits {
    /// Joint-local-to-parent reference orientation, in `[x, y, z, w]` order.
    pub reference_rotation: [f32; 4],
    /// Twist axis in the reference joint's local space. Need not be normalized.
    pub twist_axis: [f32; 3],
    /// Circular swing cone half-angle in `0..=pi` radians.
    pub max_swing: f32,
    /// Signed principal twist interval `[min, max]` within `[-pi, pi]` radians.
    /// The interval does not wrap across the half-turn seam.
    pub twist_range: [f32; 2],
}

impl Default for JointRotationLimits {
    fn default() -> Self {
        Self {
            reference_rotation: [0., 0., 0., 1.],
            twist_axis: [1., 0., 0.],
            max_swing: std::f32::consts::PI,
            twist_range: [-std::f32::consts::PI, std::f32::consts::PI],
        }
    }
}

/// Angular component before and after limiting, in radians.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JointAngleLimitStatus {
    pub requested_angle: f64,
    pub applied_angle: f64,
    pub limited: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JointRotationLimitResult {
    /// Normalized joint-local-to-parent rotation in `[x, y, z, w]` order.
    pub rotation: [f32; 4],
    pub swing: JointAngleLimitStatus,
    pub twist: JointAngleLimitStatus,
    /// The twist projection norm was at most `1e-12`; zero twist was selected.
    /// At a half-turn swing the twist decomposition is not unique.
    pub twist_degenerate: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JointRotationLimitError {
    InvalidRotation,
    InvalidReferenceRotation,
    InvalidTwistAxis,
    InvalidSwingLimit,
    InvalidTwistRange,
}

impl fmt::Display for JointRotationLimitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidRotation => "joint rotation must be finite and nonzero",
            Self::InvalidReferenceRotation => "reference rotation must be finite and nonzero",
            Self::InvalidTwistAxis => "twist axis must be finite and nonzero",
            Self::InvalidSwingLimit => "swing limit must be finite and in 0..=pi",
            Self::InvalidTwistRange => "twist limits must be finite, ordered, and within -pi..=pi",
        })
    }
}
impl std::error::Error for JointRotationLimitError {}

impl JointRotationLimits {
    /// Limits `reference.inverse() * rotation = swing * twist` independently.
    /// Finite nonzero quaternion inputs are normalized. Twist uses `(-pi, pi]`;
    /// exact half-turn twist is positive. No time state or affine decomposition is used.
    pub fn solve(
        self,
        rotation: [f32; 4],
    ) -> Result<JointRotationLimitResult, JointRotationLimitError> {
        let input = Rotation::from_quaternion(rotation.map(f64::from))
            .ok_or(JointRotationLimitError::InvalidRotation)?;
        let reference = Rotation::from_quaternion(self.reference_rotation.map(f64::from))
            .ok_or(JointRotationLimitError::InvalidReferenceRotation)?;
        let axis = unit(self.twist_axis.map(f64::from))
            .ok_or(JointRotationLimitError::InvalidTwistAxis)?;
        let pi = std::f32::consts::PI;
        if !self.max_swing.is_finite() || !(0. ..=pi).contains(&self.max_swing) {
            return Err(JointRotationLimitError::InvalidSwingLimit);
        }
        let [minimum, maximum] = self.twist_range;
        if !minimum.is_finite()
            || !maximum.is_finite()
            || minimum < -pi
            || maximum > pi
            || minimum > maximum
        {
            return Err(JointRotationLimitError::InvalidTwistRange);
        }
        let relative = reference.inverse().compose(input);
        let [x, y, z, w] = relative.quaternion();
        let projection = dot([x, y, z], axis);
        let twist_degenerate = projection.hypot(w) <= 1e-12;
        let twist_angle = if twist_degenerate {
            0.
        } else {
            let angle = 2. * projection.atan2(w);
            if angle <= -PI { PI } else { angle }
        };
        let twist_rotation = axis_rotation(axis, twist_angle);
        let swing_rotation = relative.compose(twist_rotation.inverse());
        let swing_angle = swing_rotation.angle();
        let swing = status(
            swing_angle,
            swing_angle.min(f64::from(self.max_swing).min(PI)),
        );
        let twist = status(
            twist_angle,
            twist_angle.clamp(
                f64::from(minimum).clamp(-PI, PI),
                f64::from(maximum).clamp(-PI, PI),
            ),
        );
        let constrained = if !swing.limited && !twist.limited {
            input
        } else {
            let swing_rotation = if swing_angle == 0. {
                Rotation::IDENTITY
            } else {
                swing_rotation.scaled(swing.applied_angle / swing_angle)
            };
            reference.compose(swing_rotation.compose(axis_rotation(axis, twist.applied_angle)))
        };
        Ok(JointRotationLimitResult {
            rotation: constrained.quaternion().map(|value| value as f32),
            swing,
            twist,
            twist_degenerate,
        })
    }
}

fn axis_rotation(axis: [f64; 3], angle: f64) -> Rotation {
    let (sine, cosine) = (angle * 0.5).sin_cos();
    Rotation::from_quaternion([axis[0] * sine, axis[1] * sine, axis[2] * sine, cosine]).unwrap()
}

fn status(requested_angle: f64, applied_angle: f64) -> JointAngleLimitStatus {
    JointAngleLimitStatus {
        requested_angle,
        applied_angle,
        limited: requested_angle != applied_angle,
    }
}
