use super::*;
use gpui::{EffectUniforms, SubtreeDistanceFieldPass, SubtreeEffectPass};
use gpui_effects::{contour_shadow_shader, subtree_identity_shader};

fn source(scale: f32, empty: bool) -> Scene {
    let mut scene = Scene::default();
    if !empty {
        for [x, y, w, h] in [
            [32., 32., 64., 12.],
            [32., 84., 64., 12.],
            [32., 44., 12., 40.],
            [84., 44., 12., 40.],
        ] {
            scene.insert_primitive(quad(
                bounds(x * scale, y * scale, w * scale, h * scale),
                0xffffffff,
            ));
        }
    }
    scene
}

fn scene(scale: f32, offset: f32, softness: f32, alpha: f32, empty: bool) -> Scene {
    let Primitive::SubtreeLayer(mut captured) = layer(
        source(scale, empty),
        bounds(4. * scale, 4. * scale, 152. * scale, 120. * scale),
        1.,
    ) else {
        unreachable!()
    };
    captured.intermediate_effects = vec![SubtreeEffectPass {
        shader: subtree_identity_shader(),
        uniforms: EffectUniforms::new()
            .with_slot(0, [1., 0., 0., alpha])
            .with_slot(1, [offset * scale, 0., scale, softness * scale]),
        time: 0.,
        bloom: None,
        feedback: None,
        particles: None,
        distance_field: Some(SubtreeDistanceFieldPass {
            threshold: 0.5,
            composite: contour_shadow_shader(),
        }),
    }]
    .into();
    let mut scene = Scene::default();
    scene.insert_primitive(Primitive::SubtreeLayer(captured));
    scene.finish();
    scene
}

pub(super) fn check(renderer: &mut WgpuOffscreenRenderer) -> anyhow::Result<()> {
    for scale in [1., 1.5, 2.] {
        let width = (160. * scale) as usize;
        renderer.resize(size(
            DevicePixels(width as i32),
            DevicePixels((128. * scale) as i32),
        ));
        let pixel = |x: f32, y: f32| (((y * scale) as usize) * width + (x * scale) as usize) * 4;
        let forward = renderer.render_rgba(&scene(scale, 24., 8., 0.8, false))?;
        let backward = renderer.render_rgba(&scene(scale, -24., 8., 0.8, false))?;
        assert!(forward[pixel(104., 64.)] > 80);
        assert_eq!(backward[pixel(104., 64.)], 0);
        assert!(backward[pixel(24., 64.)] > 80);
        assert_eq!(forward[pixel(24., 64.)], 0);
        assert_eq!(&forward[pixel(90., 64.)..pixel(90., 64.) + 3], &[255; 3]);
        assert_eq!(&forward[pixel(144., 64.)..pixel(144., 64.) + 3], &[0; 3]);
        let contact = renderer.render_rgba(&scene(scale, 0., 0., 0.8, false))?;
        assert_eq!(
            &contact[pixel(64., 64.)..pixel(64., 64.) + 3],
            &[0; 3],
            "unprojected holes remain open"
        );
        assert!(forward[pixel(49., 64.)] > 50, "inner edges cast into holes");
        let sharp = renderer.render_rgba(&scene(scale, 24., 0., 0.8, false))?;
        assert!(
            forward[pixel(105., 30.)] > sharp[pixel(105., 30.)],
            "far penumbra extends beyond the silhouette"
        );
        assert_eq!(forward[pixel(40., 28.)], 0, "contact region remains tight");
        let mut identity = scene(scale, 24., 8., 0., false);
        identity.subtree_layers[0].intermediate_effects = Arc::default();
        let original = renderer.render_rgba(&identity)?;
        assert_eq!(
            original,
            renderer.render_rgba(&scene(scale, 24., 8., 0., false))?
        );
        let empty = renderer.render_rgba(&scene(scale, 24., 8., 0.8, true))?;
        assert!(empty.chunks_exact(4).all(|pixel| pixel[..3] == [0; 3]));
        let mut faded = scene(scale, 24., 8., 0.8, false);
        faded.subtree_layers[0].composite.opacity = 0.5;
        assert!(renderer.render_rgba(&faded)?[pixel(104., 64.)] < forward[pixel(104., 64.)]);
        let mut clipped = scene(scale, 24., 8., 0.8, false);
        clipped.subtree_layers[0].composite.content_mask.bounds =
            bounds(0., 0., 100. * scale, 128. * scale);
        assert_eq!(renderer.render_rgba(&clipped)?[pixel(104., 64.)], 0);
    }
    Ok(())
}
