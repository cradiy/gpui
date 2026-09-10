use super::{AnimationError, Interpolation, Keyframe, Track};
use std::time::Duration;

/// Immutable absolute-time track for a runtime-sized array of signed weights.
/// Every key has the same nonzero component count. Empty derivative arrays mean
/// zero derivatives; nonempty arrays must match the weight count. Values are not
/// clamped or normalized, and samples outside the track retain the endpoint key.
#[derive(Clone, Debug)]
pub struct WeightTrack(Track<Vec<f32>>);

impl WeightTrack {
    pub fn new(
        keys: impl IntoIterator<Item = Keyframe<Vec<f32>>>,
        interpolation: Interpolation,
    ) -> Result<Self, AnimationError> {
        let mut keys = keys.into_iter().collect::<Vec<_>>();
        let count = keys.first().ok_or(AnimationError::EmptyTrack)?.value.len();
        if count == 0 {
            return Err(AnimationError::EmptyWeights);
        }
        for (index, key) in keys.iter_mut().enumerate() {
            if key.value.len() != count {
                return Err(AnimationError::WeightCount {
                    key: index,
                    expected: count,
                    actual: key.value.len(),
                });
            }
            let incoming = key.in_tangent.len();
            let outgoing = key.out_tangent.len();
            if (incoming != 0 && incoming != count) || (outgoing != 0 && outgoing != count) {
                return Err(AnimationError::WeightTangentCount {
                    key: index,
                    expected: count,
                    incoming,
                    outgoing,
                });
            }
            if incoming == 0 {
                key.in_tangent.resize(count, 0.);
            }
            if outgoing == 0 {
                key.out_tangent.resize(count, 0.);
            }
        }
        Track::new(keys, interpolation).map(Self)
    }

    pub fn weight_count(&self) -> usize {
        self.0.keys[0].value.len()
    }

    /// Shared authored keys, with omitted derivatives expanded to zeros.
    pub fn keyframes(&self) -> &[Keyframe<Vec<f32>>] {
        &self.0.keys
    }

    pub fn interpolation(&self) -> Interpolation {
        self.0.interpolation
    }

    pub fn sample(&self, time: Duration) -> Result<Vec<f32>, AnimationError> {
        self.values(time)
            .map(|value| {
                if value.is_finite() {
                    Ok(value)
                } else {
                    Err(AnimationError::InvalidSample)
                }
            })
            .collect()
    }

    /// Samples into caller-owned storage without allocating. On error, every
    /// output component remains unchanged, including cubic overflow failures.
    pub fn sample_into(&self, time: Duration, output: &mut [f32]) -> Result<(), AnimationError> {
        if output.len() != self.weight_count() {
            return Err(AnimationError::OutputCount {
                expected: self.weight_count(),
                actual: output.len(),
            });
        }
        if !self.values(time).all(|value| value.is_finite()) {
            return Err(AnimationError::InvalidSample);
        }
        for (output, value) in output.iter_mut().zip(self.values(time)) {
            *output = value;
        }
        Ok(())
    }

    fn values(&self, time: Duration) -> impl ExactSizeIterator<Item = f32> + '_ {
        let (left, right, t, seconds) = self.0.segment(time);
        (0..self.weight_count()).map(move |i| self.0.component(left, right, t, seconds, i) as f32)
    }
}
