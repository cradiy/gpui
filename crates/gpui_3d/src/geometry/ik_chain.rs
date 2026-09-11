use super::rotation::{Rotation, unit};
use super::{AffineTransform, JointRotationLimitError, JointRotationLimits, TransformError};
use crate::TransformPose;
use std::fmt;

/// One local pose in a contiguous root-to-tip chain.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IkChainJoint {
    pub pose: TransformPose,
    pub limits: Option<JointRotationLimits>,
}

impl From<TransformPose> for IkChainJoint {
    fn from(pose: TransformPose) -> Self {
        Self { pose, limits: None }
    }
}

/// Bounded, stateless cyclic coordinate descent in joint-parent coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IkChainSettings {
    /// Maximum CCD sweeps, including deterministic bend-probe sweeps. Zero only projects limits.
    pub max_iterations: usize,
    /// Maximum world-space endpoint distance considered converged.
    pub tolerance: f32,
    /// Maximum rotation of one CCD proposal, in `0..=pi` radians.
    pub max_step_angle: f32,
}

impl Default for IkChainSettings {
    fn default() -> Self {
        Self {
            max_iterations: 64,
            tolerance: 1e-4,
            max_step_angle: std::f32::consts::PI,
        }
    }
}

/// Solver termination, not a proof of geometric reachability.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IkChainStatus {
    Converged,
    Stalled,
    IterationLimit,
}

#[derive(Clone, Debug, PartialEq)]
pub struct IkChainResult {
    /// Limited local poses with unchanged translations and signed scales.
    pub poses: Vec<TransformPose>,
    /// World transforms evaluated from exactly the returned local poses.
    pub transforms: Vec<AffineTransform>,
    pub status: IkChainStatus,
    pub iterations: usize,
    /// Distance from the returned tip origin to the requested world point.
    pub target_error: f64,
    /// Sorted joints clamped during initial projection or accepted solver updates.
    pub limited_joints: Vec<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IkChainError {
    TooFewJoints,
    InvalidTarget,
    InvalidTolerance,
    InvalidStepAngle,
    InvalidPose {
        joint: usize,
        source: TransformError,
    },
    InvalidLimit {
        joint: usize,
        source: JointRotationLimitError,
    },
    Unrepresentable {
        joint: usize,
    },
}

impl fmt::Display for IkChainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooFewJoints => f.write_str("IK chain requires at least two joints"),
            Self::InvalidTarget => f.write_str("IK chain target must be finite"),
            Self::InvalidTolerance => {
                f.write_str("IK chain tolerance must be finite and nonnegative")
            }
            Self::InvalidStepAngle => {
                f.write_str("IK chain step angle must be finite and in 0..=pi")
            }
            Self::InvalidPose { joint, source } => write!(f, "IK chain joint {joint}: {source}"),
            Self::InvalidLimit { joint, source } => {
                write!(f, "IK chain joint {joint} limits: {source}")
            }
            Self::Unrepresentable { joint } => write!(
                f,
                "IK chain joint {joint} world transform is unrepresentable"
            ),
        }
    }
}
impl std::error::Error for IkChainError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidPose { source, .. } => Some(source),
            Self::InvalidLimit { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl IkChainSettings {
    /// Solves the last joint's world origin by changing ancestor local rotations.
    /// The external parent is fixed. All limits, including the terminal joint's,
    /// are projected before iteration. No terminal orientation target or time state is owned.
    pub fn solve(
        self,
        parent: AffineTransform,
        joints: &[IkChainJoint],
        target: [f32; 3],
    ) -> Result<IkChainResult, IkChainError> {
        if joints.len() < 2 {
            return Err(IkChainError::TooFewJoints);
        }
        if !target.iter().all(|v| v.is_finite()) {
            return Err(IkChainError::InvalidTarget);
        }
        if !self.tolerance.is_finite() || self.tolerance < 0. {
            return Err(IkChainError::InvalidTolerance);
        }
        if !self.max_step_angle.is_finite()
            || !(0. ..=std::f32::consts::PI).contains(&self.max_step_angle)
        {
            return Err(IkChainError::InvalidStepAngle);
        }
        let mut state = ChainState {
            poses: joints.iter().map(|joint| joint.pose).collect(),
            world: vec![AffineTransform::IDENTITY; joints.len()],
            limited: vec![false; joints.len()],
        };
        for (index, joint) in joints.iter().enumerate() {
            joint
                .pose
                .affine()
                .map_err(|source| IkChainError::InvalidPose {
                    joint: index,
                    source,
                })?;
            let rotation = Rotation::from_quaternion(joint.pose.rotation.map(f64::from)).unwrap();
            state.set_rotation(index, rotation, joint.limits)?;
        }
        state.evaluate(parent, 0)?;
        let target = target.map(f64::from);
        let mut error = state.error(target);
        let mut iterations = 0;
        let mut status = IkChainStatus::IterationLimit;
        while error > f64::from(self.tolerance) && iterations < self.max_iterations {
            if self.max_step_angle == 0. {
                status = IkChainStatus::Stalled;
                break;
            }
            iterations += 1;
            if self.sweep(&mut state, parent, joints, target)? {
                error = state.error(target);
                continue;
            }
            let mut escaped = false;
            'probes: for joint in (0..joints.len() - 1).rev() {
                for axis in 0..3 {
                    for sign in [-1., 1.] {
                        if iterations == self.max_iterations {
                            break 'probes;
                        }
                        let mut trial = state.clone();
                        let half = f64::from(self.max_step_angle).min(0.1) * sign * 0.5;
                        let mut q = [0., 0., 0., half.cos()];
                        q[axis] = half.sin();
                        let seed = Rotation::from_quaternion(q)
                            .unwrap()
                            .compose(trial.rotation(joint));
                        trial.set_rotation(joint, seed, joints[joint].limits)?;
                        if trial.poses[joint].rotation == state.poses[joint].rotation {
                            continue;
                        }
                        trial.evaluate(parent, joint)?;
                        iterations += 1;
                        self.sweep(&mut trial, parent, joints, target)?;
                        let candidate = trial.error(target);
                        if candidate < error {
                            state = trial;
                            error = candidate;
                            escaped = true;
                            break 'probes;
                        }
                    }
                }
            }
            if !escaped {
                status = if iterations == self.max_iterations {
                    IkChainStatus::IterationLimit
                } else {
                    IkChainStatus::Stalled
                };
                break;
            }
        }
        if error <= f64::from(self.tolerance) {
            status = IkChainStatus::Converged;
        }
        Ok(IkChainResult {
            poses: state.poses,
            transforms: state.world,
            status,
            iterations,
            target_error: error,
            limited_joints: state
                .limited
                .into_iter()
                .enumerate()
                .filter_map(|(i, limited)| limited.then_some(i))
                .collect(),
        })
    }

    fn sweep(
        self,
        state: &mut ChainState,
        parent: AffineTransform,
        joints: &[IkChainJoint],
        target: [f64; 3],
    ) -> Result<bool, IkChainError> {
        let mut improved = false;
        for joint in (0..joints.len() - 1).rev() {
            let error = state.error(target);
            if error <= f64::from(self.tolerance) {
                break;
            }
            let parent_world = if joint == 0 {
                parent
            } else {
                state.world[joint - 1]
            };
            let inverse = parent_world.inverse().matrix();
            let direction = |point: [f64; 3]| {
                unit(std::array::from_fn(|r| {
                    (0..3)
                        .map(|c| f64::from(inverse[c][r]) * point[c])
                        .sum::<f64>()
                        + f64::from(inverse[3][r])
                        - f64::from(state.poses[joint].translation[r])
                }))
            };
            let (Some(from), Some(to)) = (direction(state.tip()), direction(target)) else {
                continue;
            };
            let correction = Rotation::between(from, to);
            let angle = correction.angle();
            if angle == 0. {
                continue;
            }
            let source = state.rotation(joint);
            let old_pose = state.poses[joint];
            let old_limited = state.limited[joint];
            let mut accepted = false;
            let mut fraction = f64::from(self.max_step_angle).min(angle) / angle;
            for _ in 0..12 {
                state.set_rotation(
                    joint,
                    correction.scaled(fraction).compose(source),
                    joints[joint].limits,
                )?;
                state.evaluate(parent, joint)?;
                if state.error(target) < error {
                    improved = true;
                    accepted = true;
                    break;
                }
                state.limited[joint] = old_limited;
                fraction *= 0.5;
            }
            if !accepted {
                state.poses[joint] = old_pose;
                state.limited[joint] = old_limited;
                state.evaluate(parent, joint)?;
            }
        }
        Ok(improved)
    }
}

#[derive(Clone)]
struct ChainState {
    poses: Vec<TransformPose>,
    world: Vec<AffineTransform>,
    limited: Vec<bool>,
}

impl ChainState {
    fn rotation(&self, joint: usize) -> Rotation {
        Rotation::from_quaternion(self.poses[joint].rotation.map(f64::from)).unwrap()
    }

    fn set_rotation(
        &mut self,
        joint: usize,
        rotation: Rotation,
        limits: Option<JointRotationLimits>,
    ) -> Result<(), IkChainError> {
        let mut rotation = rotation.quaternion().map(|value| value as f32);
        if let Some(limits) = limits {
            let result = limits
                .solve(rotation)
                .map_err(|source| IkChainError::InvalidLimit { joint, source })?;
            rotation = result.rotation;
            self.limited[joint] |= result.swing.limited || result.twist.limited;
        }
        self.poses[joint].rotation = rotation;
        Ok(())
    }

    fn evaluate(&mut self, parent: AffineTransform, start: usize) -> Result<(), IkChainError> {
        for joint in start..self.poses.len() {
            let local = self.poses[joint]
                .affine()
                .map_err(|source| IkChainError::InvalidPose { joint, source })?;
            let parent = if joint == 0 {
                parent
            } else {
                self.world[joint - 1]
            };
            self.world[joint] = parent
                .compose(local)
                .map_err(|_| IkChainError::Unrepresentable { joint })?;
        }
        Ok(())
    }

    fn tip(&self) -> [f64; 3] {
        let matrix = self.world.last().unwrap().matrix();
        [matrix[3][0], matrix[3][1], matrix[3][2]].map(f64::from)
    }

    fn error(&self, target: [f64; 3]) -> f64 {
        self.tip()
            .into_iter()
            .zip(target)
            .map(|(a, b)| (a - b).powi(2))
            .sum::<f64>()
            .sqrt()
    }
}
