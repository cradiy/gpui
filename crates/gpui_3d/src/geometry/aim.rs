use super::AffineTransform;
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
        let quaternion = rotation_quaternion(rotation);
        let sine = dot(
            [quaternion[0], quaternion[1], quaternion[2]],
            [quaternion[0], quaternion[1], quaternion[2]],
        )
        .sqrt();
        let requested = 2. * sine.atan2(quaternion[3]);
        let applied = requested.min(f64::from(self.max_angle));
        let status = AimStatus {
            requested_angle: requested as f32,
            applied_angle: applied as f32,
            limited: applied < requested,
        };
        if applied == 0. {
            return Ok(AimResult { transform, status });
        }
        let factor = (applied * 0.5).sin() / sine;
        let [x, y, z] = [quaternion[0], quaternion[1], quaternion[2]].map(|value| value * factor);
        let w = (applied * 0.5).cos();
        let rotation = [
            [
                1. - 2. * (y * y + z * z),
                2. * (x * y + z * w),
                2. * (x * z - y * w),
            ],
            [
                2. * (x * y - z * w),
                1. - 2. * (x * x + z * z),
                2. * (y * z + x * w),
            ],
            [
                2. * (x * z + y * w),
                2. * (y * z - x * w),
                1. - 2. * (x * x + y * y),
            ],
        ];
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

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}
fn unit(value: [f64; 3]) -> Option<[f64; 3]> {
    let length = dot(value, value).sqrt();
    (length.is_finite() && length > 0.).then(|| value.map(|value| value / length))
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn basis(forward: [f64; 3], up: [f64; 3]) -> Option<[[f64; 3]; 3]> {
    let right = cross(forward, up);
    if dot(right, right) <= 1e-12 {
        return None;
    }
    let right = unit(right)?;
    Some([right, cross(right, forward), forward])
}

fn rotation_quaternion(m: [[f64; 3]; 3]) -> [f64; 4] {
    let trace = m[0][0] + m[1][1] + m[2][2];
    let mut q = if trace > 0. {
        let s = 2. * (1. + trace).sqrt();
        [
            (m[1][2] - m[2][1]) / s,
            (m[2][0] - m[0][2]) / s,
            (m[0][1] - m[1][0]) / s,
            s * 0.25,
        ]
    } else {
        let i = (0..3).max_by(|&a, &b| m[a][a].total_cmp(&m[b][b])).unwrap();
        let j = (i + 1) % 3;
        let k = (i + 2) % 3;
        let s = 2. * (1. + m[i][i] - m[j][j] - m[k][k]).max(0.).sqrt();
        let mut q = [0.; 4];
        q[i] = s * 0.25;
        q[j] = (m[i][j] + m[j][i]) / s;
        q[k] = (m[i][k] + m[k][i]) / s;
        q[3] = (m[j][k] - m[k][j]) / s;
        q
    };
    let length = q.iter().map(|value| value * value).sum::<f64>().sqrt();
    let scale = if q[3] < 0. { -1. / length } else { 1. / length };
    q.iter_mut().for_each(|value| *value *= scale);
    q
}
