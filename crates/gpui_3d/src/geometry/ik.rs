use super::{
    AffineTransform,
    rotation::{Rotation, basis, dot, unit},
};
use std::fmt;

/// Geometric target reachability before pose blending.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IkReach {
    Reachable,
    TooClose,
    TooFar,
}

/// Stateless two-segment inverse kinematics from three world-space joint transforms.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TwoBoneIkSettings {
    /// Rotation blend in `0..=1`. Zero retains the supplied pose exactly.
    pub weight: f32,
}
impl Default for TwoBoneIkSettings {
    fn default() -> Self {
        Self { weight: 1. }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TwoBoneIkResult {
    /// Root, middle, and tip world transforms, including any terminal orientation correction.
    pub transforms: [AffineTransform; 3],
    /// Closest target in the chain's radial reach interval, before blending.
    pub reachable_target: [f32; 3],
    pub reach: IkReach,
    /// Distance from the returned tip to the requested target, including blending error.
    pub target_error: f64,
    /// Present only when an orientation target was supplied.
    pub orientation: Option<IkOrientationStatus>,
}

/// World-space terminal orientation with an independent blend weight.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IkOrientationTarget {
    /// Desired local-to-world quaternion in `[x, y, z, w]` order.
    /// Finite nonzero inputs are normalized.
    pub rotation: [f32; 4],
    /// Shortest rigid rotation blend in `0..=1`, independent of position IK weight.
    pub weight: f32,
    /// Terminal joint-local forward axis. Need not be normalized.
    pub local_forward: [f32; 3],
    /// Terminal joint-local up axis, projected perpendicular to forward.
    pub local_up: [f32; 3],
}

impl Default for IkOrientationTarget {
    fn default() -> Self {
        Self {
            rotation: [0., 0., 0., 1.],
            weight: 1.,
            local_forward: [0., 0., -1.],
            local_up: [0., 1., 0.],
        }
    }
}

/// Shortest frame correction in radians, measured after position IK.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IkOrientationStatus {
    pub requested_angle: f64,
    /// Angular correction before matrix rounding.
    pub applied_angle: f64,
    /// Remaining frame angle from the returned, rounded tip to the target.
    pub target_error: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TwoBoneIkError {
    InvalidWeight,
    InvalidTarget,
    InvalidPole,
    InvalidOrientation,
    InvalidOrientationWeight,
    InvalidOrientationAxes,
    ZeroLengthBone { bone: usize },
    DegeneratePole,
    Unrepresentable,
}
impl fmt::Display for TwoBoneIkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidWeight => f.write_str("IK weight must be finite and in 0..=1"),
            Self::InvalidTarget => f.write_str("IK target must be finite"),
            Self::InvalidPole => f.write_str("IK pole must be finite and distinct from the root"),
            Self::InvalidOrientation => f.write_str("IK orientation must be finite and nonzero"),
            Self::InvalidOrientationWeight => f.write_str("IK orientation weight must be finite and in 0..=1"),
            Self::InvalidOrientationAxes => f.write_str("IK orientation axes must be finite, nonzero, and nonparallel before and after transformation"),
            Self::ZeroLengthBone { bone } => write!(f, "IK bone {bone} has coincident joints"),
            Self::DegeneratePole => {
                f.write_str("IK pole is parallel to the target direction for a bent solution")
            }
            Self::Unrepresentable => {
                f.write_str("IK pose or bone lengths cannot be represented at f32 precision")
            }
        }
    }
}
impl std::error::Error for TwoBoneIkError {}

impl TwoBoneIkSettings {
    /// Solves root/middle/tip world poses using their current joint distances as bone
    /// lengths. The pole is a world-space point selecting the bend side. Rotation
    /// blending preserves segment lengths; it does not interpolate joint positions.
    /// No stretching, joint limits, terminal orientation target, or time state is applied.
    pub fn solve(
        self,
        transforms: [AffineTransform; 3],
        target: [f32; 3],
        pole: [f32; 3],
    ) -> Result<TwoBoneIkResult, TwoBoneIkError> {
        if !self.weight.is_finite() || !(0. ..=1.).contains(&self.weight) {
            return Err(TwoBoneIkError::InvalidWeight);
        }
        if !target.iter().all(|value| value.is_finite()) {
            return Err(TwoBoneIkError::InvalidTarget);
        }
        if !pole.iter().all(|value| value.is_finite()) {
            return Err(TwoBoneIkError::InvalidPole);
        }
        let positions = transforms.map(position);
        let upper = sub(positions[1], positions[0]);
        let lower = sub(positions[2], positions[1]);
        let lengths = [length(upper), length(lower)];
        for (bone, length) in lengths.into_iter().enumerate() {
            if length == 0. {
                return Err(TwoBoneIkError::ZeroLengthBone { bone });
            }
        }
        let root = positions[0];
        let pole_direction =
            unit(sub(pole.map(f64::from), root)).ok_or(TwoBoneIkError::InvalidPole)?;
        let toward = sub(target.map(f64::from), root);
        let distance = length(toward);
        let [a, b] = lengths;
        let minimum = (a - b).abs();
        let maximum = a + b;
        let reach = if distance < minimum {
            IkReach::TooClose
        } else if distance > maximum {
            IkReach::TooFar
        } else {
            IkReach::Reachable
        };
        let clamped = distance.clamp(minimum, maximum);
        let (joint, tip) = if clamped == 0. {
            (add(root, pole_direction.map(|value| value * a)), root)
        } else {
            let axis = unit(toward).unwrap_or_else(|| {
                unit(sub(positions[2], root)).unwrap_or_else(|| unit(upper).unwrap())
            });
            let (along, height) = if clamped == maximum {
                (a, 0.)
            } else if clamped == minimum {
                (if a >= b { a } else { -a }, 0.)
            } else {
                let along = ((clamped + (a - b) * (a + b) / clamped) * 0.5).clamp(-a, a);
                (along, ((a - along) * (a + along)).max(0.).sqrt())
            };
            let bend = if height == 0. {
                [0.; 3]
            } else {
                let projected = sub(
                    pole_direction,
                    axis.map(|value| value * dot(axis, pole_direction)),
                );
                if length(projected) <= 1e-6 {
                    return Err(TwoBoneIkError::DegeneratePole);
                }
                unit(projected).unwrap()
            };
            (
                std::array::from_fn(|i| root[i] + axis[i] * along + bend[i] * height),
                add(root, axis.map(|value| value * clamped)),
            )
        };
        let reachable_target = tip.map(|value| value as f32);
        if !reachable_target.iter().all(|value| value.is_finite()) {
            return Err(TwoBoneIkError::Unrepresentable);
        }
        let result = if self.weight == 0. {
            transforms
        } else {
            let root_rotation = Rotation::between(
                unit(upper).unwrap(),
                unit(sub(joint, root)).ok_or(TwoBoneIkError::Unrepresentable)?,
            );
            let joint_rotation = Rotation::between(
                unit(lower).unwrap(),
                root_rotation
                    .inverse()
                    .apply(unit(sub(tip, joint)).ok_or(TwoBoneIkError::Unrepresentable)?),
            );
            let root_rotation = root_rotation.scaled(f64::from(self.weight));
            let total_rotation =
                root_rotation.compose(joint_rotation.scaled(f64::from(self.weight)));
            let (middle, end) = if self.weight == 1. {
                (joint, tip)
            } else {
                let middle = add(root, root_rotation.apply(upper));
                (middle, add(middle, total_rotation.apply(lower)))
            };
            [
                rotated(transforms[0], root_rotation, root)?,
                rotated(transforms[1], total_rotation, middle)?,
                rotated(transforms[2], total_rotation, end)?,
            ]
        };
        let actual = result.map(position);
        for bone in 0..2 {
            let actual_length = length(sub(actual[bone + 1], actual[bone]));
            if (actual_length / lengths[bone] - 1.).abs() > 1e-4 {
                return Err(TwoBoneIkError::Unrepresentable);
            }
        }
        Ok(TwoBoneIkResult {
            transforms: result,
            reachable_target,
            reach,
            target_error: length(sub(actual[2], target.map(f64::from))),
            orientation: None,
        })
    }

    /// Solves position IK, then rotates only the tip about its solved origin.
    /// Aligns the transformed forward/projected-up frame with the target rotation
    /// applied to the same local frame, preserving affine shape and handedness.
    /// Both local and transformed axis pairs require sine angle greater than `1e-6`.
    /// Zero orientation weight preserves the position-only result but still validates inputs.
    pub fn solve_with_orientation(
        self,
        transforms: [AffineTransform; 3],
        target: [f32; 3],
        pole: [f32; 3],
        orientation: IkOrientationTarget,
    ) -> Result<TwoBoneIkResult, TwoBoneIkError> {
        let desired = Rotation::from_quaternion(orientation.rotation.map(f64::from))
            .ok_or(TwoBoneIkError::InvalidOrientation)?;
        if !orientation.weight.is_finite() || !(0. ..=1.).contains(&orientation.weight) {
            return Err(TwoBoneIkError::InvalidOrientationWeight);
        }
        let mut result = self.solve(transforms, target, pole)?;
        let tip = result.transforms[2];
        let correction = orientation_correction(tip, desired, orientation)?;
        let requested_angle = correction.angle();
        let applied_angle = requested_angle * f64::from(orientation.weight);
        if applied_angle > 0. {
            result.transforms[2] = rotated(
                tip,
                correction.scaled(f64::from(orientation.weight)),
                position(tip),
            )?;
        }
        result.orientation = Some(IkOrientationStatus {
            requested_angle,
            applied_angle,
            target_error: orientation_correction(result.transforms[2], desired, orientation)?
                .angle(),
        });
        Ok(result)
    }
}

fn orientation_correction(
    transform: AffineTransform,
    desired: Rotation,
    target: IkOrientationTarget,
) -> Result<Rotation, TwoBoneIkError> {
    let invalid = TwoBoneIkError::InvalidOrientationAxes;
    let forward = unit(target.local_forward.map(f64::from)).ok_or(invalid)?;
    let up = unit(target.local_up.map(f64::from)).ok_or(invalid)?;
    let destination = basis(desired.apply(forward), desired.apply(up)).ok_or(invalid)?;
    let matrix = transform.matrix();
    let direction = |axis: [f64; 3]| {
        unit(std::array::from_fn(|r| {
            (0..3).map(|c| f64::from(matrix[c][r]) * axis[c]).sum()
        }))
        .ok_or(invalid)
    };
    let source = basis(direction(forward)?, direction(up)?).ok_or(invalid)?;
    Ok(Rotation::from_matrix(std::array::from_fn(|c| {
        std::array::from_fn(|r| (0..3).map(|k| destination[k][r] * source[k][c]).sum())
    })))
}

fn rotated(
    source: AffineTransform,
    rotation: Rotation,
    position: [f64; 3],
) -> Result<AffineTransform, TwoBoneIkError> {
    let mut matrix = source.matrix();
    for column in matrix.iter_mut().take(3) {
        let rotated = rotation.apply([column[0], column[1], column[2]].map(f64::from));
        for i in 0..3 {
            column[i] = rotated[i] as f32;
        }
    }
    for i in 0..3 {
        matrix[3][i] = position[i] as f32;
    }
    AffineTransform::from_matrix(matrix).map_err(|_| TwoBoneIkError::Unrepresentable)
}
fn position(transform: AffineTransform) -> [f64; 3] {
    let matrix = transform.matrix();
    [matrix[3][0], matrix[3][1], matrix[3][2]].map(f64::from)
}
fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| a[i] + b[i])
}
fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| a[i] - b[i])
}
fn length(value: [f64; 3]) -> f64 {
    dot(value, value).sqrt()
}
