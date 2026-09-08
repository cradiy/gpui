use std::sync::Arc;

use lyon::{math::Point as LyonPoint, path::PathEvent};

use crate::{FillOptions, Path, PathBuilder, PathStyle, Pixels, Result, StrokeOptions};

#[derive(Clone, Copy)]
struct PointPair(LyonPoint, LyonPoint);

impl PointPair {
    fn new(from: LyonPoint, to: LyonPoint) -> Result<Self> {
        anyhow::ensure!(
            [from.x, from.y, to.x, to.y].iter().all(|v| v.is_finite()),
            "morph coordinates must be finite"
        );
        Ok(Self(from, to))
    }

    fn at(self, progress: f64) -> LyonPoint {
        if progress == 0. {
            return self.0;
        }
        if progress == 1. {
            return self.1;
        }
        let mix =
            |a: f32, b: f32| (f64::from(a) * (1. - progress) + f64::from(b) * progress) as f32;
        lyon::math::point(mix(self.0.x, self.1.x), mix(self.0.y, self.1.y))
    }
}

enum MorphCommand {
    Begin(PointPair),
    Line(PointPair),
    Quadratic {
        control: PointPair,
        to: PointPair,
    },
    Cubic {
        control_a: PointPair,
        control_b: PointPair,
        to: PointPair,
    },
    End(bool),
}

/// A reusable pairing of two vector paths with matching command sequences.
/// Coordinates interpolate linearly; topology, contour order and winding are caller-owned.
/// Clones share the validated pairing. No clock or animation frames are owned by the path.
#[derive(Clone)]
pub struct PathMorph(Arc<[MorphCommand]>);

impl PathMorph {
    /// Applies each builder's transform and pairs its geometry in command order.
    /// Command kinds, counts and contour closure flags must match. Non-finite coordinates
    /// are rejected. Fill/stroke styles and dash settings are not retained.
    /// Degenerate segments are retained so a segment can grow from or shrink to a point.
    pub fn new(from: PathBuilder, to: PathBuilder) -> Result<Self> {
        let from = from.into_geometry();
        let to = to.into_geometry();
        let mut from = from.iter();
        let mut to = to.iter();
        let mut commands = Vec::new();
        loop {
            let command = match (from.next(), to.next()) {
                (None, None) => break,
                (Some(PathEvent::Begin { at: a }), Some(PathEvent::Begin { at: b })) => {
                    MorphCommand::Begin(PointPair::new(a, b)?)
                }
                (Some(PathEvent::Line { to: a, .. }), Some(PathEvent::Line { to: b, .. })) => {
                    MorphCommand::Line(PointPair::new(a, b)?)
                }
                (
                    Some(PathEvent::Quadratic {
                        ctrl: a, to: a_to, ..
                    }),
                    Some(PathEvent::Quadratic {
                        ctrl: b, to: b_to, ..
                    }),
                ) => MorphCommand::Quadratic {
                    control: PointPair::new(a, b)?,
                    to: PointPair::new(a_to, b_to)?,
                },
                (
                    Some(PathEvent::Cubic {
                        ctrl1: a1,
                        ctrl2: a2,
                        to: a_to,
                        ..
                    }),
                    Some(PathEvent::Cubic {
                        ctrl1: b1,
                        ctrl2: b2,
                        to: b_to,
                        ..
                    }),
                ) => MorphCommand::Cubic {
                    control_a: PointPair::new(a1, b1)?,
                    control_b: PointPair::new(a2, b2)?,
                    to: PointPair::new(a_to, b_to)?,
                },
                (Some(PathEvent::End { close: a, .. }), Some(PathEvent::End { close: b, .. }))
                    if a == b =>
                {
                    MorphCommand::End(a)
                }
                _ => anyhow::bail!(
                    "path commands or contour closures differ at index {}",
                    commands.len()
                ),
            };
            commands.push(command);
        }
        Ok(Self(commands.into()))
    }

    /// Interpolates endpoints and Bézier controls at a progress clamped to `0..=1`.
    /// Non-finite progress returns an error. The returned builder defaults to fill and
    /// can be styled, transformed, measured, or used to construct another morph.
    pub fn interpolate(&self, progress: f32) -> Result<PathBuilder> {
        anyhow::ensure!(progress.is_finite(), "morph progress must be finite");
        let progress = f64::from(progress.clamp(0., 1.));
        let mut builder = lyon::path::Path::builder();
        for command in self.0.iter() {
            match command {
                MorphCommand::Begin(at) => {
                    builder.begin(at.at(progress));
                }
                MorphCommand::Line(to) => {
                    builder.line_to(to.at(progress));
                }
                MorphCommand::Quadratic { control, to } => {
                    builder.quadratic_bezier_to(control.at(progress), to.at(progress));
                }
                MorphCommand::Cubic {
                    control_a,
                    control_b,
                    to,
                } => {
                    builder.cubic_bezier_to(
                        control_a.at(progress),
                        control_b.at(progress),
                        to.at(progress),
                    );
                }
                MorphCommand::End(close) => builder.end(*close),
            }
        }
        Ok(builder.into())
    }

    /// Tessellates an interpolated outline with the given cap, join and width settings.
    pub fn stroke(&self, progress: f32, options: &StrokeOptions) -> Result<Path<Pixels>> {
        self.interpolate(progress)?
            .with_style(PathStyle::Stroke(*options))
            .build()
    }

    /// Tessellates an interpolated fill with the given fill rule and tolerance.
    pub fn fill(&self, progress: f32, options: &FillOptions) -> Result<Path<Pixels>> {
        self.interpolate(progress)?
            .with_style(PathStyle::Fill(*options))
            .build()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{point, px};

    fn contour(x: f32, close: bool) -> PathBuilder {
        let mut path = PathBuilder::fill();
        path.move_to(point(px(x), px(10.)));
        path.line_to(point(px(x + 10.), px(10.)));
        path.curve_to(point(px(x + 20.), px(20.)), point(px(x + 20.), px(10.)));
        path.cubic_bezier_to(
            point(px(x), px(20.)),
            point(px(x + 20.), px(30.)),
            point(px(x), px(30.)),
        );
        if close {
            path.close();
        }
        path
    }

    #[test]
    fn interpolation_preserves_endpoints_controls_and_transforms() {
        let mut target = contour(0., true);
        target.translate(point(px(40.), px(0.)));
        let morph = PathMorph::new(contour(0., true), target).unwrap();
        for (progress, x) in [(-2., 0.), (0., 0.), (0.25, 10.), (1., 40.), (2., 40.)] {
            let actual = morph.interpolate(progress).unwrap().into_geometry();
            let expected = contour(x, true).into_geometry();
            assert_eq!(
                actual.iter().collect::<Vec<_>>(),
                expected.iter().collect::<Vec<_>>()
            );
        }
        assert!(morph.interpolate(f32::NAN).is_err());
        assert!(morph.interpolate(f32::INFINITY).is_err());
    }

    #[test]
    fn mismatched_topology_and_non_finite_geometry_are_rejected() {
        assert!(PathMorph::new(contour(0., true), contour(0., false)).is_err());
        assert!(PathMorph::new(contour(0., true), PathBuilder::fill()).is_err());
        let mut line = PathBuilder::fill();
        line.move_to(point(px(0.), px(0.)));
        line.line_to(point(px(10.), px(0.)));
        let mut curve = PathBuilder::fill();
        curve.move_to(point(px(0.), px(0.)));
        curve.curve_to(point(px(10.), px(0.)), point(px(5.), px(0.)));
        assert!(PathMorph::new(line, curve).is_err());
        let mut invalid = contour(0., true);
        invalid.scale(f32::NAN);
        assert!(PathMorph::new(contour(0., true), invalid).is_err());
    }

    #[test]
    fn collapsed_segments_and_separate_contours_survive_pairing() {
        let make = |extent: f32| {
            let mut path = PathBuilder::fill();
            for y in [10., 50.] {
                path.move_to(point(px(0.), px(y)));
                path.line_to(point(px(extent), px(y)));
            }
            path
        };
        let morph = PathMorph::new(make(0.), make(40.)).unwrap();
        let middle = morph.interpolate(0.5).unwrap().into_geometry();
        assert_eq!(
            middle.iter().collect::<Vec<_>>(),
            make(20.).into_geometry().iter().collect::<Vec<_>>()
        );
        assert_eq!(morph.interpolate(0.5).unwrap().measure().length(), px(40.));
        assert!(
            PathMorph::new(PathBuilder::fill(), PathBuilder::fill())
                .unwrap()
                .fill(0.5, &FillOptions::default())
                .unwrap()
                .vertices
                .is_empty()
        );
    }
}
