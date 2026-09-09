use gpui::{
    Bounds, ContentMask, DevicePixels, EffectQuad, EffectShader, Mesh3d, MeshDraw3d, MeshTexture3d,
    MeshVertex3d, Primitive, Quad, ScaledPixels, Scene, Scene3dFrame, SubtreeLayer, point, rgba,
    size,
};
use gpui_wgpu::WgpuOffscreenRenderer;
use std::{rc::Rc, sync::Arc};

const IDENTITY: [[f32; 4]; 4] = [
    [1., 0., 0., 0.],
    [0., 1., 0., 0.],
    [0., 0., 1., 0.],
    [0., 0., 0., 1.],
];
fn bounds(x: f32, y: f32, w: f32, h: f32) -> Bounds<ScaledPixels> {
    Bounds::new(
        point(ScaledPixels(x), ScaledPixels(y)),
        size(ScaledPixels(w), ScaledPixels(h)),
    )
}
fn quad(region: Bounds<ScaledPixels>, color: u32) -> Quad {
    Quad {
        bounds: region,
        content_mask: ContentMask { bounds: region },
        background: rgba(color).into(),
        ..Default::default()
    }
}
fn mesh(z: f32, color: u32, texture: MeshTexture3d) -> MeshDraw3d {
    let vertices = [
        (-0.7, -0.7, [0., 1.]),
        (0.7, -0.7, [1., 1.]),
        (0.7, 0.7, [1., 0.]),
        (-0.7, 0.7, [0., 0.]),
    ]
    .map(|(x, y, uv)| MeshVertex3d {
        position: [x, y, z],
        normal: [0., 0., 1.],
        uv,
    });
    MeshDraw3d {
        mesh: Mesh3d::new(vertices.to_vec(), vec![0, 1, 2, 0, 2, 3]),
        model: IDENTITY,
        normal: IDENTITY,
        color: rgba(color),
        texture,
        unlit: true,
        alpha_cutoff: 0.5,
    }
}
fn layer(
    region: Bounds<ScaledPixels>,
    mut source: Scene,
    objects: Vec<MeshDraw3d>,
    opacity: f32,
) -> SubtreeLayer {
    source.finish();
    SubtreeLayer {
        scene3d: Some(Arc::new(Scene3dFrame {
            view_projection: IDENTITY,
            light_direction: [0., 0., 1.],
            light: [1.; 4],
            ambient: 0.3,
            objects: objects.into(),
        })),
        scene: Rc::new(source),
        second_scene: None,
        intermediate_effects: Arc::default(),
        composite: EffectQuad {
            order: 0,
            bounds: region,
            effect_bounds: region,
            transformation: Default::default(),
            content_mask: ContentMask { bounds: region },
            shader: EffectShader::wgsl_image(
                "fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> { return sample_effect_image(input, input.uv); }",
            ),
            uniforms: Default::default(),
            time: 0.,
            corner_radii: Default::default(),
            opacity,
            image_tile: None,
            second_image_tile: None,
            third_image_tile: None,
            fourth_image_tile: None,
        },
    }
}
fn scene(layer: SubtreeLayer) -> Scene {
    let mut scene = Scene::default();
    scene.insert_primitive(Primitive::SubtreeLayer(layer));
    scene.finish();
    scene
}

#[test]
#[ignore = "requires a GPU adapter"]
fn mesh_depth_capture_clipping_and_nested_composition() -> anyhow::Result<()> {
    let mut renderer = WgpuOffscreenRenderer::new(size(DevicePixels(128), DevicePixels(96)))?;
    let tile = renderer
        .sprite_atlas()
        .get_or_insert_with(
            &gpui::RenderImageParams {
                image_id: gpui::ImageId(98000),
                frame_index: 0,
            }
            .into(),
            &mut || {
                Ok(Some((
                    size(DevicePixels(1), DevicePixels(1)),
                    std::borrow::Cow::Borrowed(&[255, 0, 255, 255]),
                )))
            },
        )?
        .expect("image tile");
    for scale in [1., 1.5, 2.] {
        let width = (128. * scale) as usize;
        renderer.resize(size(
            DevicePixels(width as i32),
            DevicePixels((96. * scale) as i32),
        ));
        let region = bounds(8. * scale, 8. * scale, 112. * scale, 80. * scale);
        let pixel = |x: usize, y: usize| {
            (((y as f32 * scale) as usize) * width + (x as f32 * scale) as usize) * 4
        };
        let near = mesh(0.2, 0xff0000ff, MeshTexture3d::None);
        let textured = renderer.render_rgba(&scene(layer(
            region,
            Scene::default(),
            vec![mesh(0.2, 0xffffffff, MeshTexture3d::Image(tile))],
            1.,
        )))?;
        assert_eq!(&textured[pixel(64, 48)..pixel(64, 48) + 3], &[255, 0, 255]);
        let far = mesh(0.8, 0x0000ffff, MeshTexture3d::None);
        let a = renderer.render_rgba(&scene(layer(
            region,
            Scene::default(),
            vec![near.clone(), far.clone()],
            1.,
        )))?;
        let b = renderer.render_rgba(&scene(layer(
            region,
            Scene::default(),
            vec![far.clone(), near.clone()],
            1.,
        )))?;
        assert_eq!(a, b);
        assert_eq!(&a[pixel(64, 48)..pixel(64, 48) + 3], &[255, 0, 0]);
        let mut source = Scene::default();
        source.insert_primitive(quad(
            bounds(8. * scale, 8. * scale, 56. * scale, 80. * scale),
            0x00ff00ff,
        ));
        let captured = layer(
            region,
            source,
            vec![mesh(0.2, 0xffffffff, MeshTexture3d::Subtree), far.clone()],
            1.,
        );
        let output = renderer.render_rgba(&scene(captured.clone()))?;
        assert_eq!(&output[pixel(40, 48)..pixel(40, 48) + 3], &[0, 255, 0]);
        assert_eq!(&output[pixel(90, 48)..pixel(90, 48) + 3], &[0, 0, 255]);
        let mut clipped = captured.clone();
        clipped.composite.content_mask.bounds = bounds(0., 0., 64. * scale, 96. * scale);
        let output = renderer.render_rgba(&scene(clipped))?;
        assert_eq!(&output[pixel(90, 48)..pixel(90, 48) + 3], &[0, 0, 0]);
        let mut nested = layer(
            region,
            scene(captured),
            vec![mesh(0.2, 0xffffffff, MeshTexture3d::Subtree)],
            0.5,
        );
        let outer = scene(nested.clone());
        assert_eq!(outer.subtree_target_count(), 3);
        let faded = renderer.render_rgba(&outer)?;
        nested.composite.opacity = 1.;
        let bright = renderer.render_rgba(&scene(nested))?;
        assert!(
            faded[pixel(40, 48) + 1] > 0 && faded[pixel(40, 48) + 1] < bright[pixel(40, 48) + 1]
        );
        let clipped_depth = renderer.render_rgba(&scene(layer(
            region,
            Scene::default(),
            vec![mesh(-0.1, 0xff0000ff, MeshTexture3d::None), far],
            1.,
        )))?;
        assert_eq!(
            &clipped_depth[pixel(64, 48)..pixel(64, 48) + 3],
            &[0, 0, 255]
        );
    }
    Ok(())
}
