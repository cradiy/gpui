use super::*;
use gpui::{Path, PathBuilder, Pixels, StrokeOptions, px};

fn scene(mut path: Path<Pixels>) -> Scene {
    path.content_mask = ContentMask {
        bounds: Bounds::new(point(px(0.), px(0.)), size(px(64.), px(48.))),
    };
    path.color = rgba(0xffffffff).into();
    let mut scene = Scene::default();
    scene.insert_primitive(quad(bounds(0., 0., 64., 48.), 0x000000ff));
    scene.insert_primitive(path.scale(1.));
    scene.finish();
    scene
}
fn red(pixels: &[u8], x: usize) -> u8 {
    pixels[(24 * 64 + x) * 4]
}

pub(super) fn check(renderer: &mut WgpuOffscreenRenderer) -> anyhow::Result<()> {
    renderer.resize(size(DevicePixels(64), DevicePixels(48)));
    let mut builder = PathBuilder::stroke(px(4.));
    builder.move_to(point(px(8.), px(24.)));
    builder.line_to(point(px(56.), px(24.)));
    let path = builder.measure();
    let style = StrokeOptions::default().with_line_width(4.);
    let prefix = renderer.render_rgba(&scene(path.stroke_range(0.0..0.5, &style)?))?;
    assert!(red(&prefix, 20) > 200 && red(&prefix, 44) == 0);
    let pattern = [px(8.), px(8.)];
    let original = renderer.render_rgba(&scene(path.stroke_dashed(
        0.0..1.,
        &pattern,
        px(0.),
        &style,
    )?))?;
    assert!(red(&original, 11) > 200 && red(&original, 19) == 0 && red(&original, 27) > 200);
    let shifted = renderer.render_rgba(&scene(path.stroke_dashed(
        0.0..1.,
        &pattern,
        px(4.),
        &style,
    )?))?;
    assert!(red(&shifted, 15) == 0 && red(&shifted, 23) > 200 && red(&shifted, 31) == 0);
    let backwards = renderer.render_rgba(&scene(path.stroke_dashed(
        0.0..1.,
        &pattern,
        px(-4.),
        &style,
    )?))?;
    assert!(red(&backwards, 9) == 0 && red(&backwards, 15) > 200);
    assert_eq!(
        original,
        renderer.render_rgba(&scene(path.stroke_dashed(
            0.0..1.,
            &pattern,
            px(16.),
            &style
        )?))?
    );
    let trimmed = renderer.render_rgba(&scene(path.stroke_dashed(
        0.25..0.75,
        &pattern,
        px(4.),
        &style,
    )?))?;
    for x in 22..42 {
        assert_eq!(
            red(&trimmed, x),
            red(&shifted, x),
            "trim must preserve the full path's dash phase"
        );
    }
    assert_eq!(red(&trimmed, 10), 0);
    assert_eq!(red(&trimmed, 54), 0);
    for offset in [0., 3., -4.] {
        let odd = renderer.render_rgba(&scene(path.stroke_dashed(
            0.0..1.,
            &[px(6.), px(4.), px(2.)],
            px(offset),
            &style,
        )?))?;
        let repeated = renderer.render_rgba(&scene(path.stroke_dashed(
            0.0..1.,
            &[px(6.), px(4.), px(2.), px(6.), px(4.), px(2.)],
            px(offset),
            &style,
        )?))?;
        assert_eq!(odd, repeated);
    }
    let empty = renderer.render_rgba(&scene(path.stroke_range(0.7..0.2, &style)?))?;
    assert!(empty.chunks_exact(4).all(|p| p[..3] == [0, 0, 0]));
    let mut builder = PathBuilder::stroke(px(4.))
        .dash_array(&pattern)
        .dash_offset(px(4.));
    builder.move_to(point(px(8.), px(24.)));
    builder.line_to(point(px(56.), px(24.)));
    assert_eq!(shifted, renderer.render_rgba(&scene(builder.build()?))?);
    let solid = renderer.render_rgba(&scene(path.stroke(&style)?))?;
    assert_eq!(
        solid,
        renderer.render_rgba(&scene(path.stroke_dashed(0.0..1., &[], px(0.), &style)?))?
    );
    Ok(())
}
