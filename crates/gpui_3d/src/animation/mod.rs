//! Absolute-time transform sampling without playback state.

use crate::{AffineTransform, TransformError};
use std::{fmt, sync::Arc, time::Duration};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Interpolation {
    Step,
    #[default]
    Linear,
    CubicSpline,
}

/// A timestamp, value, and optional component derivatives per second.
/// Tangents are used only by `CubicSpline` interpolation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Keyframe<T> {
    pub time: Duration,
    pub value: T,
    pub in_tangent: T,
    pub out_tangent: T,
}

impl<T: Default> Keyframe<T> {
    pub fn new(time: Duration, value: T) -> Self {
        Self {
            time,
            value,
            in_tangent: T::default(),
            out_tangent: T::default(),
        }
    }

    pub fn tangents(mut self, incoming: T, outgoing: T) -> Self {
        self.in_tangent = incoming;
        self.out_tangent = outgoing;
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AnimationError {
    EmptyTrack,
    NonIncreasingTime {
        key: usize,
    },
    NonFiniteKey {
        key: usize,
    },
    InvalidRotation {
        key: usize,
    },
    /// Interpolation produced an unrepresentable value or a zero quaternion.
    InvalidSample,
    InvalidTransform(TransformError),
}

impl fmt::Display for AnimationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyTrack => f.write_str("an animation track needs at least one keyframe"),
            Self::NonIncreasingTime { key } => {
                write!(f, "keyframe {key} is not strictly after its predecessor")
            }
            Self::NonFiniteKey { key } => write!(f, "keyframe {key} contains nonfinite components"),
            Self::InvalidRotation { key } => write!(f, "keyframe {key} contains a zero quaternion"),
            Self::InvalidSample => {
                f.write_str("animation sample is nonfinite, unrepresentable, or a zero quaternion")
            }
            Self::InvalidTransform(source) => write!(f, "animation transform: {source}"),
        }
    }
}
impl std::error::Error for AnimationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidTransform(source) => Some(source),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
struct Track<const N: usize> {
    keys: Arc<[Keyframe<[f32; N]>]>,
    interpolation: Interpolation,
}

impl<const N: usize> Track<N> {
    fn new(
        keys: impl IntoIterator<Item = Keyframe<[f32; N]>>,
        interpolation: Interpolation,
    ) -> Result<Self, AnimationError> {
        let keys = keys.into_iter().collect::<Vec<_>>();
        if keys.is_empty() {
            return Err(AnimationError::EmptyTrack);
        }
        for (index, key) in keys.iter().enumerate() {
            if index > 0 && keys[index - 1].time >= key.time {
                return Err(AnimationError::NonIncreasingTime { key: index });
            }
            if !key
                .value
                .iter()
                .chain(&key.in_tangent)
                .chain(&key.out_tangent)
                .all(|v| v.is_finite())
            {
                return Err(AnimationError::NonFiniteKey { key: index });
            }
        }
        Ok(Self {
            keys: keys.into(),
            interpolation,
        })
    }

    fn segment(&self, time: Duration) -> (usize, usize, f64, f64) {
        let right = self.keys.partition_point(|key| key.time <= time);
        if right == 0 {
            return (0, 0, 0., 0.);
        }
        let left = right - 1;
        if right == self.keys.len() || self.keys[left].time == time {
            return (left, left, 0., 0.);
        }
        let seconds = (self.keys[right].time - self.keys[left].time).as_secs_f64();
        let t = (time - self.keys[left].time).as_secs_f64() / seconds;
        (left, right, t, seconds)
    }

    fn components(&self, left: usize, right: usize, t: f64, seconds: f64) -> [f64; N] {
        let a = &self.keys[left];
        let b = &self.keys[right];
        if left == right || self.interpolation == Interpolation::Step {
            return a.value.map(f64::from);
        }
        std::array::from_fn(|i| match self.interpolation {
            Interpolation::Linear => (1. - t) * f64::from(a.value[i]) + t * f64::from(b.value[i]),
            Interpolation::CubicSpline => {
                let t2 = t * t;
                let t3 = t2 * t;
                (2. * t3 - 3. * t2 + 1.) * f64::from(a.value[i])
                    + seconds * (t3 - 2. * t2 + t) * f64::from(a.out_tangent[i])
                    + (-2. * t3 + 3. * t2) * f64::from(b.value[i])
                    + seconds * (t3 - t2) * f64::from(b.in_tangent[i])
            }
            Interpolation::Step => unreachable!(),
        })
    }
}

/// Immutable XYZ track. Times must be strictly increasing and all components finite.
/// Sampling clamps to the first/last key, including single-key tracks.
#[derive(Clone, Debug)]
pub struct VectorTrack(Track<3>);

impl VectorTrack {
    pub fn new(
        keys: impl IntoIterator<Item = Keyframe<[f32; 3]>>,
        interpolation: Interpolation,
    ) -> Result<Self, AnimationError> {
        Track::new(keys, interpolation).map(Self)
    }

    pub fn keyframes(&self) -> &[Keyframe<[f32; 3]>] {
        &self.0.keys
    }

    pub fn interpolation(&self) -> Interpolation {
        self.0.interpolation
    }

    pub fn sample(&self, time: Duration) -> Result<[f32; 3], AnimationError> {
        let (left, right, t, seconds) = self.0.segment(time);
        let value = self.0.components(left, right, t, seconds).map(|v| v as f32);
        if value.iter().all(|v| v.is_finite()) {
            Ok(value)
        } else {
            Err(AnimationError::InvalidSample)
        }
    }
}

/// Immutable XYZW quaternion track with normalized samples.
/// Linear uses shortest-arc spherical interpolation. CubicSpline interpolates
/// the supplied components and derivatives without sign changes, then normalizes.
/// Keys must be finite and nonzero; cubic curves crossing zero return an error.
#[derive(Clone, Debug)]
pub struct RotationTrack(Track<4>);

impl RotationTrack {
    pub fn new(
        keys: impl IntoIterator<Item = Keyframe<[f32; 4]>>,
        interpolation: Interpolation,
    ) -> Result<Self, AnimationError> {
        let track = Track::new(keys, interpolation)?;
        for (key, value) in track.keys.iter().enumerate() {
            if value.value.iter().all(|v| *v == 0.) {
                return Err(AnimationError::InvalidRotation { key });
            }
        }
        Ok(Self(track))
    }

    pub fn keyframes(&self) -> &[Keyframe<[f32; 4]>] {
        &self.0.keys
    }

    pub fn interpolation(&self) -> Interpolation {
        self.0.interpolation
    }

    pub fn sample(&self, time: Duration) -> Result<[f32; 4], AnimationError> {
        let (left, right, t, seconds) = self.0.segment(time);
        let value = if left != right && self.0.interpolation == Interpolation::Linear {
            let a = normalize(self.0.keys[left].value.map(f64::from))?;
            let mut b = normalize(self.0.keys[right].value.map(f64::from))?;
            let mut dot = a.iter().zip(b).map(|(a, b)| a * b).sum::<f64>();
            if dot < 0. {
                b = b.map(|v| -v);
                dot = -dot;
            }
            let (wa, wb) = if dot > 0.9995 {
                (1. - t, t)
            } else {
                let angle = dot.clamp(0., 1.).acos();
                (
                    ((1. - t) * angle).sin() / angle.sin(),
                    (t * angle).sin() / angle.sin(),
                )
            };
            std::array::from_fn(|i| wa * a[i] + wb * b[i])
        } else {
            self.0.components(left, right, t, seconds)
        };
        Ok(normalize(value)?.map(|v| v as f32))
    }
}

fn normalize(value: [f64; 4]) -> Result<[f64; 4], AnimationError> {
    let length = value.iter().map(|v| v * v).sum::<f64>().sqrt();
    if length == 0. || !length.is_finite() {
        return Err(AnimationError::InvalidSample);
    }
    Ok(value.map(|v| v / length))
}

/// Local translation, XYZW quaternion rotation, and signed scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransformPose {
    pub translation: [f32; 3],
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
}

impl Default for TransformPose {
    fn default() -> Self {
        Self {
            translation: [0.; 3],
            rotation: [0., 0., 0., 1.],
            scale: [1.; 3],
        }
    }
}

impl TransformPose {
    pub fn affine(self) -> Result<AffineTransform, TransformError> {
        AffineTransform::from_trs(self.translation, self.rotation, self.scale)
    }
}

/// Independent channels sampled at one absolute time. Missing channels retain
/// the explicit base pose; no affine decomposition, clock, or loop policy is applied.
#[derive(Clone, Debug, Default)]
pub struct TransformTrack {
    base: TransformPose,
    translation: Option<VectorTrack>,
    rotation: Option<RotationTrack>,
    scale: Option<VectorTrack>,
}

impl TransformTrack {
    pub fn new(base: TransformPose) -> Result<Self, AnimationError> {
        base.affine().map_err(AnimationError::InvalidTransform)?;
        Ok(Self {
            base,
            ..Default::default()
        })
    }

    pub fn translation(mut self, track: VectorTrack) -> Self {
        self.translation = Some(track);
        self
    }

    pub fn rotation(mut self, track: RotationTrack) -> Self {
        self.rotation = Some(track);
        self
    }

    pub fn scale(mut self, track: VectorTrack) -> Self {
        self.scale = Some(track);
        self
    }

    /// Returns an invertible pose or an error, including scale curves crossing zero.
    pub fn sample(&self, time: Duration) -> Result<TransformPose, AnimationError> {
        let mut pose = self.base;
        if let Some(track) = &self.translation {
            pose.translation = track.sample(time)?;
        }
        if let Some(track) = &self.rotation {
            pose.rotation = track.sample(time)?;
        }
        if let Some(track) = &self.scale {
            pose.scale = track.sample(time)?;
        }
        pose.affine().map_err(AnimationError::InvalidTransform)?;
        Ok(pose)
    }

    pub fn sample_transform(&self, time: Duration) -> Result<AffineTransform, AnimationError> {
        self.sample(time)?
            .affine()
            .map_err(AnimationError::InvalidTransform)
    }
}

#[cfg(test)]
mod tests;
