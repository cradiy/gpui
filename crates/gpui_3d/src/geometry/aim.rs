use super::AffineTransform;
use super::rotation::{Rotation, basis, unit};
use std::fmt;

/// Stateless orientation settings. Vectors need not be normalized.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AimSettings {
    pub local_forward: [f32; 3],
    pub local_up: [f32; 3],
    /// Preferred world-space up direction, projected perpendicular to the target direction.
    pub world_up: [f32; 3],
    /// Maximum total orientation correction, including roll, in radians in `0..=pi`.
    /// Measured from the supplied transform, not from a previous solver result.
    pub max_angle: f32,
}

impl Default for AimSettings {
    fn default() -> Self {
        Self {
            local_forward: [0., 0., -1.],
            local_up: [0., 1., 0.],
            world_up: [0., 1., 0.],
            max_angle: std::f32::consts::PI,
        }
    }
}

/// Total shortest orientation correction before and after the configured limit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AimStatus {
    pub requested_angle: f32,
    pub applied_angle: f32,
    pub limited: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AimResult {
    pub transform: AffineTransform,
    pub status: AimStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AimError {
    InvalidAxes,
    InvalidUp,
    InvalidTarget,
    CoincidentTarget,
    ParallelUp,
    InvalidLimit,
    Unrepresentable,
}

impl fmt::Display for AimError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidAxes => {
                "aim axes must be finite, nonzero, and nonparallel after transformation"
            }
            Self::InvalidUp => "aim world up must be finite and nonzero",
            Self::InvalidTarget => "aim target must be finite",
            Self::CoincidentTarget => "aim target coincides with the transform origin",
            Self::ParallelUp => "aim target direction is parallel to world up",
            Self::InvalidLimit => "aim angle limit must be finite and in 0..=pi",
            Self::Unrepresentable => "aim result exceeds finite affine precision",
        })
    }
}
impl std::error::Error for AimError {}

impl AimSettings {
    /// Rotates about the transform origin to face a world-space point. Translation,
    /// affine shape, and handedness are preserved; no TRS decomposition is performed.
    /// The transformed local up component perpendicular to forward aligns with the
    /// projected world up. Parallel pairs within a sine angle of `1e-6` are rejected.
    /// An angular limit follows the shortest total orientation rotation, including roll.
    pub fn solve(
        self,
        transform: AffineTransform,
        target: [f32; 3],
    ) -> Result<AimResult, AimError> {
        if !self.max_angle.is_finite() || !(0. ..=std::f32::consts::PI).contains(&self.max_angle) {
            return Err(AimError::InvalidLimit);
        }
        if !target.iter().all(|value| value.is_finite()) {
            return Err(AimError::InvalidTarget);
        }
        let matrix = transform.matrix();
        let direction = |local: [f32; 3]| -> Result<[f64; 3], AimError> {
            let local = unit(local.map(f64::from)).ok_or(AimError::InvalidAxes)?;
            unit(std::array::from_fn(|r| {
                (0..3).map(|c| f64::from(matrix[c][r]) * local[c]).sum()
            }))
            .ok_or(AimError::InvalidAxes)
        };
        let source = basis(direction(self.local_forward)?, direction(self.local_up)?)
            .ok_or(AimError::InvalidAxes)?;
        let forward = unit(std::array::from_fn(|i| {
            f64::from(target[i]) - f64::from(matrix[3][i])
        }))
        .ok_or(AimError::CoincidentTarget)?;
        let up = unit(self.world_up.map(f64::from)).ok_or(AimError::InvalidUp)?;
        let destination = basis(forward, up).ok_or(AimError::ParallelUp)?;
        let rotation: [[f64; 3]; 3] = std::array::from_fn(|c| {
            std::array::from_fn(|r| (0..3).map(|k| destination[k][r] * source[k][c]).sum())
        });
        let rotation = Rotation::from_matrix(rotation);
        let requested = rotation.angle();
        let applied = requested.min(f64::from(self.max_angle));
        let status = AimStatus {
            requested_angle: requested as f32,
            applied_angle: applied as f32,
            limited: applied < requested,
        };
        if applied == 0. {
            return Ok(AimResult { transform, status });
        }
        let rotation = rotation.scaled(applied / requested).matrix();
        let mut result = matrix;
        for c in 0..3 {
            for r in 0..3 {
                result[c][r] = (0..3)
                    .map(|k| rotation[k][r] * f64::from(matrix[c][k]))
                    .sum::<f64>() as f32;
            }
        }
        let transform =
            AffineTransform::from_matrix(result).map_err(|_| AimError::Unrepresentable)?;
        Ok(AimResult { transform, status })
    }
}
