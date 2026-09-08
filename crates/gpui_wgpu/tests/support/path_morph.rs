use super::*;
use gpui::{
    FillOptions, FillRule, LineCap, Path, PathBuilder, PathMorph, Pixels, StrokeOptions, px,
};

fn scene(mut path: Path<Pixels>, scale: f32, clip_width: f32) -> Scene {
    path.content_mask = ContentMask {
        bounds: Bounds::new(point(px(0.), px(0.)), size(px(clip_width), px(48.))),
    };
    path.color = rgba(0xffffffff).into();
    let mut scene = Scene::default();
    scene.insert_primitive(quad(bounds(0., 0., 64. * scale, 48. * scale), 0x000000ff));
    scene.insert_primitive(path.scale(scale));
    scene.finish();
    scene
}

fn frame(x: f32) -> PathBuilder {
    let mut path = PathBuilder::fill();
    for (inset, width, height) in [(0., 28., 32.), (8., 12., 16.)] {
        path.add_polygon(
            &[
                point(px(x + inset), px(6. + inset)),
                point(px(x + inset + width), px(6. + inset)),
                point(px(x + inset + width), px(6. + inset + height)),
                point(px(x + inset), px(6. + inset + height)),
            ],
            true,
        );
    }
    path
}

pub(super) fn check(renderer: &mut WgpuOffscreenRenderer) -> anyhow::Result<()> {
    let morph = PathMorph::new(frame(8.), frame(28.))?;
    let fill = FillOptions::default().with_fill_rule(FillRule::EvenOdd);
    for scale in [1., 2.] {
        renderer.resize(size(
            DevicePixels((64. * scale) as i32),
            DevicePixels((48. * scale) as i32),
        ));
        for progress in [0., 0.25, 0.5, 1.] {
            let actual = renderer.render_rgba(&scene(morph.fill(progress, &fill)?, scale, 64.))?;
            let expected = frame(8. + progress * 20.)
                .with_style(gpui::PathStyle::Fill(fill))
                .build()?;
            assert_eq!(actual, renderer.render_rgba(&scene(expected, scale, 64.))?);
            let hole = ((24. * scale) as usize * (64. * scale) as usize
                + ((22. + progress * 20.) * scale) as usize)
                * 4;
            assert_eq!(actual[hole], 0, "the inner contour must remain a hole");
        }
    }
    renderer.resize(size(DevicePixels(64), DevicePixels(48)));
    let lines = |length: f32| {
        let mut path = PathBuilder::fill();
        for y in [10., 36.] {
            path.move_to(point(px(8.), px(y)));
            path.line_to(point(px(8. + length), px(y)));
        }
        path
    };
    let morph = PathMorph::new(lines(0.), lines(48.))?;
    let stroke = StrokeOptions::default()
        .with_line_width(4.)
        .with_line_cap(LineCap::Round);
    let pixels = renderer.render_rgba(&scene(morph.stroke(0.5, &stroke)?, 1., 64.))?;
    let red = |x: usize, y: usize| pixels[(y * 64 + x) * 4];
    assert!(red(20, 10) > 200 && red(20, 36) > 200);
    assert_eq!(red(20, 23), 0);
    assert_eq!(red(42, 10), 0);
    let clipped = renderer.render_rgba(&scene(morph.stroke(1., &stroke)?, 1., 32.))?;
    assert!(
        clipped
            .chunks_exact(4)
            .enumerate()
            .all(|(i, p)| i % 64 < 32 || p[..3] == [0, 0, 0])
    );
    Ok(())
}
