use std::{ops::Range, sync::Arc};

use lyon::algorithms::measure::{PathMeasurements, SampleType};
use lyon::path::PathEvent;

use crate::{Path, PathBuilder, Pixels, Point, Result, StrokeOptions, point, px};

/// A position and forward direction sampled by distance along a path.
#[derive(Clone, Copy, Debug)]
pub struct PathPosition {
    /// Position in the path's logical coordinate space.
    pub position: Point<Pixels>,
    /// Unit tangent, or zero if the direction cannot be determined.
    pub tangent: Point<f32>,
}

struct MeasuredPathData {
    path: lyon::path::Path,
    measurements: PathMeasurements,
    length: f32,
}

/// Immutable vector geometry with cached arc-length measurements.
/// Cloning shares the geometry and measurements. Construct with [`PathBuilder::measure`].
#[derive(Clone)]
pub struct MeasuredPath(Arc<MeasuredPathData>);

impl MeasuredPath {
    pub(crate) fn from_path(path: lyon::path::Path) -> Self {
        let path = without_empty_segments(&path);
        let measurements = PathMeasurements::from_path(&path, 0.01);
        let length = measurements
            .create_sampler(&path, SampleType::Distance)
            .length();
        Self(Arc::new(MeasuredPathData {
            path,
            measurements,
            length,
        }))
    }

    /// Approximate length in logical pixels, including closing edges but excluding move-to gaps.
    pub fn length(&self) -> Pixels {
        px(self.0.length)
    }

    /// Samples normalized arc length, clamped to `0..=1`.
    /// Empty or zero-length paths and non-finite input produce `None`.
    pub fn sample(&self, progress: f32) -> Option<PathPosition> {
        if !progress.is_finite() {
            return None;
        }
        self.sample_at(px(progress.clamp(0., 1.) * self.0.length))
    }

    /// Samples a logical distance, clamped to the beginning and end of the path.
    pub fn sample_at(&self, distance: Pixels) -> Option<PathPosition> {
        let length = self.0.length;
        if length <= 0. || !length.is_finite() || !f32::from(distance).is_finite() {
            return None;
        }
        let distance = f32::from(distance).clamp(0., length);
        let mut sampler = self
            .0
            .measurements
            .create_sampler(&self.0.path, SampleType::Distance);
        let sample = sampler.sample(distance);
        let position = sample.position();
        if !position.x.is_finite() || !position.y.is_finite() {
            return None;
        }
        let mut tangent = sample.tangent();
        if !tangent.x.is_finite() || !tangent.y.is_finite() {
            let step = (length * 0.00001).max(0.001).min(length * 0.5);
            tangent = if distance + step <= length {
                sampler.sample(distance + step).position() - position
            } else {
                position - sampler.sample((distance - step).max(0.)).position()
            };
            let norm = tangent.x.hypot(tangent.y);
            tangent = if norm > 0. && norm.is_finite() {
                tangent / norm
            } else {
                lyon::math::vector(0., 0.)
            };
        }
        Some(PathPosition {
            position: position.into(),
            tangent: point(tangent.x, tangent.y),
        })
    }

    /// Tessellates the complete path with its original contour joins and closures.
    pub fn stroke(&self, options: &StrokeOptions) -> Result<Path<Pixels>> {
        PathBuilder::tessellate_stroke(None, px(0.), &self.0.path, options)
    }

    /// Tessellates a normalized arc-length range. Reversed or empty ranges draw nothing.
    /// Partial ranges use the configured end caps; a full range retains closed joins.
    pub fn stroke_range(&self, range: Range<f32>, options: &StrokeOptions) -> Result<Path<Pixels>> {
        let range = checked_range(range)?;
        if range.start == 0. && range.end == 1. {
            return self.stroke(options);
        }
        let mut builder = lyon::path::Path::builder();
        if range.start < range.end && self.0.length > 0. {
            self.0
                .measurements
                .create_sampler(&self.0.path, SampleType::Normalized)
                .split_range(range, &mut builder);
        }
        PathBuilder::tessellate_stroke(None, px(0.), &builder.build(), options)
    }

    /// Tessellates dashes clipped to a normalized arc-length range.
    /// Pattern entries must be finite and positive; an empty pattern draws a solid stroke.
    /// Odd-length patterns repeat twice. Positive offset advances into the pattern.
    /// Phase is anchored to the complete path, independent of the visible range.
    pub fn stroke_dashed(
        &self,
        range: Range<f32>,
        pattern: &[Pixels],
        offset: Pixels,
        options: &StrokeOptions,
    ) -> Result<Path<Pixels>> {
        let range = checked_range(range)?;
        if pattern.is_empty() {
            return self.stroke_range(range, options);
        }
        anyhow::ensure!(
            pattern
                .iter()
                .all(|v| f32::from(*v).is_finite() && *v > px(0.)),
            "dash lengths must be finite and positive"
        );
        anyhow::ensure!(f32::from(offset).is_finite(), "dash offset must be finite");
        let count = pattern
            .len()
            .checked_mul(if pattern.len() % 2 == 1 { 2 } else { 1 })
            .ok_or_else(|| anyhow::anyhow!("dash pattern is too large"))?;
        let period = pattern
            .iter()
            .map(|v| f64::from(f32::from(*v)))
            .sum::<f64>()
            * (count / pattern.len()) as f64;
        let length = f64::from(self.0.length);
        let start = f64::from(range.start) * length;
        let end = f64::from(range.end) * length;
        let mut phase = (f64::from(f32::from(offset)) + start).rem_euclid(period);
        let mut index = 0;
        while index + 1 < count && phase >= f64::from(f32::from(pattern[index % pattern.len()])) {
            phase -= f64::from(f32::from(pattern[index % pattern.len()]));
            index += 1;
        }
        let mut position = start;
        let mut remaining = f64::from(f32::from(pattern[index % pattern.len()])) - phase;
        let mut builder = lyon::path::Path::builder();
        let mut sampler = self
            .0
            .measurements
            .create_sampler(&self.0.path, SampleType::Distance);
        let mut segments = 0;
        while position < end {
            let next = (position + remaining).min(end);
            anyhow::ensure!(
                next > position && segments < 16_384,
                "dash pattern is too dense to tessellate"
            );
            if index % 2 == 0 {
                sampler.split_range(position as f32..next as f32, &mut builder);
            }
            position = next;
            index = (index + 1) % count;
            remaining = f64::from(f32::from(pattern[index % pattern.len()]));
            segments += 1;
        }
        PathBuilder::tessellate_stroke(None, px(0.), &builder.build(), options)
    }
}

fn checked_range(range: Range<f32>) -> Result<Range<f32>> {
    anyhow::ensure!(
        range.start.is_finite() && range.end.is_finite(),
        "path range must be finite"
    );
    Ok(range.start.clamp(0., 1.)..range.end.clamp(0., 1.))
}

fn without_empty_segments(path: &lyon::path::Path) -> lyon::path::Path {
    let mut builder = lyon::path::Path::builder();
    let mut started = false;
    for event in path.iter() {
        match event {
            PathEvent::Begin { .. } => {
                started = false;
            }
            PathEvent::Line { from, to } if from != to => {
                if !started {
                    builder.begin(from);
                    started = true;
                }
                builder.line_to(to);
            }
            PathEvent::Quadratic { from, ctrl, to } if from != ctrl || from != to => {
                if !started {
                    builder.begin(from);
                    started = true;
                }
                builder.quadratic_bezier_to(ctrl, to);
            }
            PathEvent::Cubic {
                from,
                ctrl1,
                ctrl2,
                to,
            } if from != ctrl1 || from != ctrl2 || from != to => {
                if !started {
                    builder.begin(from);
                    started = true;
                }
                builder.cubic_bezier_to(ctrl1, ctrl2, to);
            }
            PathEvent::End { close, .. } if started => {
                builder.end(close);
                started = false;
            }
            _ => {}
        }
    }
    builder.build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sampling_uses_arc_length_and_skips_move_gaps() {
        let mut builder = PathBuilder::stroke(px(2.));
        builder.move_to(point(px(0.), px(0.)));
        builder.line_to(point(px(30.), px(0.)));
        builder.line_to(point(px(30.), px(40.)));
        builder.move_to(point(px(200.), px(100.)));
        builder.line_to(point(px(230.), px(100.)));
        let path = builder.measure();
        assert!((f32::from(path.length()) - 100.).abs() < 0.01);
        let sample = path.sample(0.5).unwrap();
        assert!((f32::from(sample.position.y) - 20.).abs() < 0.01);
        assert_eq!(sample.tangent, point(0., 1.));
        assert_eq!(
            path.sample_at(px(85.)).unwrap().position,
            point(px(215.), px(100.))
        );
        let stroke = path
            .stroke_range(0.65..0.85, &StrokeOptions::default())
            .unwrap();
        assert!(
            stroke
                .vertices
                .iter()
                .all(|v| v.xy_position.x < px(40.) || v.xy_position.x >= px(199.))
        );
    }

    #[test]
    fn sampling_handles_empty_segments_and_stationary_curve_endpoints() {
        let mut builder = PathBuilder::stroke(px(2.));
        builder.move_to(point(px(90.), px(90.)));
        builder.move_to(point(px(0.), px(0.)));
        builder.line_to(point(px(0.), px(0.)));
        builder.cubic_bezier_to(
            point(px(100.), px(0.)),
            point(px(0.), px(0.)),
            point(px(0.), px(0.)),
        );
        let path = builder.measure();
        assert_eq!(path.sample(0.).unwrap().tangent, point(1., 0.));
        assert!((f32::from(path.sample(0.5).unwrap().position.x) - 50.).abs() < 0.2);
        assert!(path.sample(f32::NAN).is_none());
        assert!(PathBuilder::fill().measure().sample(0.).is_none());
        assert!(
            path.stroke_range(0.5..0.5, &StrokeOptions::default())
                .unwrap()
                .vertices
                .is_empty()
        );
        assert!(
            path.stroke_dashed(0.0..1., &[px(0.)], px(0.), &StrokeOptions::default())
                .is_err()
        );
        assert!(
            path.stroke_dashed(
                0.0..1.,
                &[px(f32::MIN_POSITIVE)],
                px(0.),
                &StrokeOptions::default()
            )
            .is_err()
        );
    }
}
