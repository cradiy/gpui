#![cfg(all(feature = "wgpu", not(target_family = "wasm")))]

use anyhow::Result;
use gpui_3d::*;
use gpui_wgpu::{Scene3dGpuOutput, wgpu};

fn plane(left: f32, right: f32, z: f32) -> Mesh {
    Mesh::new(
        [
            [left, -1., z],
            [right, -1., z],
            [right, 1., z],
            [left, 1., z],
        ]
        .into_iter()
        .map(|position| Vertex {
            position,
            normal: [0., 0., 1.],
            uv: [0.; 2],
        })
        .collect(),
        vec![0, 1, 2, 0, 2, 3],
    )
}

fn scene(camera: Camera) -> Scene {
    let material = Material::color(gpui::rgb(0x80a0c0)).double_sided(true);
    Scene::new()
        .camera(camera)
        .object(Object::new(plane(-1., 1., 0.7), material.clone()).id("surface"))
        .object(Object::new(plane(0., 1., 0.4), material).id("other"))
}

fn read(output: &Scene3dGpuOutput, context: &WgpuContext) -> Result<Scene3dPixels> {
    let mut pending = output.readback()?;
    context.device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(std::time::Duration::from_secs(15)),
    })?;
    Ok(pending
        .try_read()?
        .expect("submitted readback must complete"))
}

#[test]
fn validates_membership_without_changing_primary_geometry() -> Result<()> {
    let source = scene(Camera::default());
    let group = EditOcclusionGroup::new(42, ["surface".into()])
        .mesh(plane(-1., 1., 0.), AffineTransform::IDENTITY);
    let prepared = source
        .clone()
        .edit_occlusion([group.clone()])
        .prepare(1., None, |_| Ok(TextureState::Ready(ResolvedTexture::None)))?;
    let frame = prepared.frame();
    assert_eq!(frame.objects.len(), 2);
    assert_eq!(&*frame.occlusion_groups[0].members, &[1]);
    assert_eq!(frame.occlusion_groups[0].occluders[0].output_id, 3);
    for groups in [
        vec![EditOcclusionGroup::new(1, ["missing".into()])],
        vec![group.clone(), group.clone()],
        vec![group, EditOcclusionGroup::new(7, ["surface".into()])],
    ] {
        assert!(
            source
                .clone()
                .edit_occlusion(groups)
                .prepare(1., None, |_| Ok(TextureState::Ready(ResolvedTexture::None)))
                .is_err()
        );
    }
    Ok(())
}

#[test]
fn rejects_invalid_overlay_elements_before_rendering() {
    let point = EditPoint {
        id: 5,
        position: [0.; 3],
        style: EditStyle::new(7., gpui::rgb(0xffffff)),
    };
    let prepare = |group| {
        scene(Camera::default())
            .edit_occlusion([group])
            .prepare(1., None, |_| Ok(TextureState::Ready(ResolvedTexture::None)))
    };
    for points in [
        vec![point, point],
        vec![EditPoint { id: 0, ..point }],
        vec![EditPoint {
            position: [f32::NAN, 0., 0.],
            ..point
        }],
        vec![EditPoint {
            style: EditStyle {
                size: 0.,
                ..point.style
            },
            ..point
        }],
    ] {
        assert!(prepare(EditOcclusionGroup::new(1, ["surface".into()]).points(points)).is_err());
    }
    assert!(
        prepare(
            EditOcclusionGroup::new(1, ["surface".into()])
                .points([point])
                .pixel_scale(f32::INFINITY)
        )
        .is_err()
    );
}

#[test]
#[ignore = "requires a GPU adapter"]
fn edit_projection_clips_depth_and_preserves_perspective_interpolation() -> Result<()> {
    let context = WgpuContext::new_headless()?;
    let mut renderer = HeadlessRenderer::with_context(context.clone())?;
    let camera = Camera {
        eye: [0., 0., 4.],
        near: 0.1,
        far: 10.,
        projection: Projection::Perspective { vertical_fov: 1. },
        ..Default::default()
    };
    let style = EditStyle::new(3., gpui::rgb(0xffffff));
    let source = Scene::new().camera(camera).object(
        Object::new(plane(-1., 1., 0.), Material::color(gpui::rgb(0x102030))).id("surface"),
    );
    let group = EditOcclusionGroup::new(1, ["surface".into()])
        .lines([
            EditLine {
                id: 1,
                start: [-0.1, 0.2, 4.2],
                end: [0.5, 0.2, 0.],
                style,
            },
            EditLine {
                id: 2,
                start: [-0.6, -0.3, 1.],
                end: [0.7, -0.3, -1.],
                style,
            },
            EditLine {
                id: 3,
                start: [0., 0.4, 0.],
                end: [0., 0.4, 0.],
                style,
            },
        ])
        .points([
            EditPoint {
                id: 4,
                position: [0., 0., 5.],
                style,
            },
            EditPoint {
                id: 5,
                position: [0., 0., -20.],
                style,
            },
        ]);
    let frame = renderer.render(
        &source.edit_occlusion([group]),
        Scene3dOutputConfig {
            size: [128; 2],
            channels: Scene3dChannels::COLOR,
            color_samples: 4,
        },
    )?;
    let data = read(
        frame.gpu().occlusion_groups()[0].elements().unwrap(),
        &context,
    )?;
    let ids = data.object_ids.unwrap();
    let depths = data.linear_depth.unwrap();
    for id in [1, 2, 3] {
        assert!(ids.contains(&id), "missing clipped element {id}");
    }
    assert!(!ids.contains(&4) && !ids.contains(&5));
    let mut count = 0;
    for (index, id) in ids.iter().enumerate() {
        if *id == 2 {
            let project = |x: f32, y: f32, depth: f32| {
                [
                    64. * (1. + x / (depth * 0.5_f32.tan())),
                    64. * (1. - y / (depth * 0.5_f32.tan())),
                ]
            };
            let a = project(-0.6, -0.3, 3.);
            let b = project(0.7, -0.3, 5.);
            let delta = [b[0] - a[0], b[1] - a[1]];
            let p = [(index % 128) as f32 + 0.5, (index / 128) as f32 + 0.5];
            let t = (((p[0] - a[0]) * delta[0] + (p[1] - a[1]) * delta[1])
                / (delta[0] * delta[0] + delta[1] * delta[1]))
                .clamp(0., 1.);
            let expected = 1. / ((1. - t) / 3. + t / 5.);
            assert!(
                (depths[index] - expected).abs() < 1e-4,
                "incorrect perspective depth"
            );
            count += 1;
        }
    }
    assert!(count > 20);
    assert!(
        ids.iter().filter(|id| **id != 0).count() < 2000,
        "clipped segments must not cover the viewport"
    );
    Ok(())
}

#[test]
#[ignore = "requires a GPU adapter"]
fn self_occlusion_preserves_width_on_slopes_without_revealing_back_lines() -> Result<()> {
    let context = WgpuContext::new_headless()?;
    let mut renderer = HeadlessRenderer::with_context(context.clone())?;
    let camera = Camera {
        eye: [0., 0., 4.],
        near: 0.1,
        far: 20.,
        projection: Projection::Orthographic { vertical_size: 2. },
        ..Default::default()
    };
    let base = plane(-1., 1., 0.);
    let auxiliary = Mesh::new(
        base.vertices()
            .iter()
            .map(|v| Vertex {
                position: [
                    v.position[0],
                    v.position[1],
                    v.position[0] * 0.5 + v.position[1] * 0.4,
                ],
                ..*v
            })
            .collect(),
        base.indices().to_vec(),
    );
    let style = EditStyle::new(6., gpui::rgb(0xffffff));
    let source = scene(camera).edit_occlusion([EditOcclusionGroup::new(
        1,
        ["surface".into(), "other".into()],
    )
    .mesh(auxiliary, AffineTransform::IDENTITY)
    .lines([
        EditLine {
            id: 1,
            start: [-0.8, 0., -0.4],
            end: [0.8, 0., 0.4],
            style,
        },
        EditLine {
            id: 2,
            start: [-0.8, 0.5, -0.7],
            end: [0.8, 0.5, 0.1],
            style,
        },
    ])]);
    let frame = renderer.render(
        &source,
        Scene3dOutputConfig {
            size: [128; 2],
            channels: Scene3dChannels::COLOR,
            color_samples: 4,
        },
    )?;
    let ids = read(
        frame.gpu().occlusion_groups()[0].elements().unwrap(),
        &context,
    )?
    .object_ids
    .unwrap();
    assert!(!ids.contains(&2));
    for x in 16..112 {
        assert_eq!(
            (0..128).filter(|y| ids[y * 128 + x] == 1).count(),
            6,
            "self-occluded width at {x}"
        );
    }
    Ok(())
}

#[test]
#[ignore = "requires a GPU adapter"]
fn edit_elements_share_fragment_visibility_and_keep_primary_picking() -> Result<()> {
    let context = WgpuContext::new_headless()?;
    let mut renderer = HeadlessRenderer::with_context(context.clone())?;
    let camera = Camera {
        eye: [0., 0., 4.],
        near: 0.1,
        far: 20.,
        projection: Projection::Orthographic { vertical_size: 2. },
        ..Default::default()
    };
    let source = scene(camera);
    let style = EditStyle::new(4., gpui::rgb(0xff0000));
    let line = EditLine {
        id: 10,
        start: [-0.85, 0., 0.1],
        end: [0.85, 0., 0.1],
        style,
    };
    let points = [
        EditPoint {
            id: 20,
            position: [-0.5, 0.4, 0.1],
            style: EditStyle::new(8., gpui::rgb(0xff0000)),
        },
        EditPoint {
            id: 21,
            position: [0.5, 0.4, 0.1],
            style: EditStyle::new(8., gpui::rgb(0xff0000)),
        },
        EditPoint {
            id: 30,
            position: [0.02, 0.6, 0.1],
            style: EditStyle::new(12., gpui::rgb(0xff0000)),
        },
    ];
    let group = |hidden, pixel_scale| {
        EditOcclusionGroup::new(42, ["surface".into()])
            .mesh(plane(-1., 1., 0.), AffineTransform::IDENTITY)
            .lines([EditLine {
                style: EditStyle {
                    hidden,
                    hidden_color: gpui::rgb(0x00ff00),
                    ..style
                },
                ..line
            }])
            .points(points.map(|p| EditPoint {
                style: EditStyle {
                    hidden,
                    hidden_color: gpui::rgb(0x00ff00),
                    ..p.style
                },
                ..p
            }))
            .pixel_scale(pixel_scale)
    };
    for extent in [64, 128] {
        let config = Scene3dOutputConfig {
            size: [extent; 2],
            channels: Scene3dChannels::all(),
            color_samples: 4,
        };
        let original = renderer.render(&source, config)?;
        let baseline = read(original.gpu(), &context)?;
        let mut previous_ids = None;
        for samples in [1, 4] {
            let frame = renderer.render(
                &source
                    .clone()
                    .edit_occlusion([group(EditHiddenStyle::Hide, 1.)]),
                Scene3dOutputConfig {
                    color_samples: samples,
                    ..config
                },
            )?;
            let main = read(frame.gpu(), &context)?;
            assert_eq!(main.object_ids, baseline.object_ids);
            assert_eq!(main.linear_depth, baseline.linear_depth);
            assert_eq!(main.world_normals, baseline.world_normals);
            let output = frame.gpu().occlusion_groups()[0].elements().unwrap();
            let elements = read(output, &context)?;
            let ids = elements.object_ids.unwrap();
            if let Some(previous) = previous_ids {
                assert_eq!(ids, previous);
            }
            let n = extent as usize;
            assert_eq!(ids[n / 2 * n + n / 4], 10);
            assert_eq!(ids[n / 2 * n + n * 3 / 4], 0);
            assert!(ids.contains(&20));
            assert!(!ids.contains(&21));
            assert_eq!(ids[(n / 5) * n + n / 2 - 2], 30);
            assert_eq!(ids[(n / 5) * n + n / 2 + 2], 0);
            let rows = (0..n).filter(|y| ids[y * n + n / 4] == 10).count();
            assert_eq!(rows, 4, "width must not depend on output size or MSAA");
            assert!((elements.linear_depth.unwrap()[n / 2 * n + n / 4] - 3.9).abs() < 1e-4);
            let color = main.rgba.unwrap();
            assert_eq!(
                main.linear_rgba.as_ref().unwrap()[n / 2 * n + n / 4],
                [1., 0., 0., 1.]
            );
            assert!(
                color[(n / 2 * n + n / 4) * 4] > 240 && color[(n / 2 * n + n / 4) * 4 + 1] < 10
            );
            assert_eq!(
                &color[(n / 2 * n + n * 3 / 4) * 4..(n / 2 * n + n * 3 / 4) * 4 + 4],
                &baseline.rgba.as_ref().unwrap()
                    [(n / 2 * n + n * 3 / 4) * 4..(n / 2 * n + n * 3 / 4) * 4 + 4]
            );
            previous_ids = Some(ids);
        }
        let solid = renderer.render(
            &source
                .clone()
                .edit_occlusion([group(EditHiddenStyle::Solid, 1.)]),
            config,
        )?;
        let solid_ids = read(
            solid.gpu().occlusion_groups()[0].elements().unwrap(),
            &context,
        )?
        .object_ids
        .unwrap();
        assert!(solid_ids.contains(&21));
        let dashed = renderer.render(
            &source
                .clone()
                .edit_occlusion([group(EditHiddenStyle::Dashed { dash: 4., gap: 4. }, 1.)]),
            config,
        )?;
        let dash_ids = read(
            dashed.gpu().occlusion_groups()[0].elements().unwrap(),
            &context,
        )?
        .object_ids
        .unwrap();
        let n = extent as usize;
        let count = |ids: &[u32]| {
            (n / 2..n * 9 / 10)
                .filter(|x| ids[n / 2 * n + x] == 10)
                .count()
        };
        assert!(count(&dash_ids) > 0 && count(&dash_ids) < count(&solid_ids));
        assert!((n / 8..n / 2 - 1).all(|x| dash_ids[n / 2 * n + x] == 10));
        let scaled = renderer.render(
            &source
                .clone()
                .edit_occlusion([group(EditHiddenStyle::Hide, 2.)]),
            config,
        )?;
        let ids = read(
            scaled.gpu().occlusion_groups()[0].elements().unwrap(),
            &context,
        )?
        .object_ids
        .unwrap();
        assert_eq!((0..n).filter(|y| ids[y * n + n / 4] == 10).count(), 8);
        let budget = scaled.gpu().target_memory().total_bytes;
        renderer.set_target_byte_limit(Some(budget - 1));
        assert!(
            renderer
                .render(
                    &source
                        .clone()
                        .edit_occlusion([group(EditHiddenStyle::Hide, 2.)]),
                    config
                )
                .is_err()
        );
        renderer.set_target_byte_limit(Some(budget));
        assert!(
            renderer
                .render(
                    &source
                        .clone()
                        .edit_occlusion([group(EditHiddenStyle::Hide, 2.)]),
                    config
                )
                .is_ok()
        );
        renderer.set_target_byte_limit(None);
    }
    Ok(())
}

#[test]
#[ignore = "requires a GPU adapter"]
fn grouped_depth_preserves_foreign_occluders_hidden_by_the_final_surface() -> Result<()> {
    let context = WgpuContext::new_headless()?;
    let mut renderer = HeadlessRenderer::with_context(context.clone())?;
    let camera = Camera {
        eye: [0., 0., 4.],
        near: 0.1,
        far: 20.,
        projection: Projection::Orthographic { vertical_size: 2. },
        ..Default::default()
    };
    let source = scene(camera);
    let grouped = source.clone().edit_occlusion([
        EditOcclusionGroup::new(42, ["surface".into()])
            .mesh(plane(-1., 1., 0.), AffineTransform::IDENTITY),
        EditOcclusionGroup::new(99, ["other".into()]),
    ]);
    let config = Scene3dOutputConfig {
        size: [64; 2],
        channels: Scene3dChannels::COLOR
            | Scene3dChannels::OBJECT_ID
            | Scene3dChannels::LINEAR_DEPTH,
        color_samples: 4,
    };
    let original = renderer.render(&source, config)?;
    let original_pixels = read(original.gpu(), &context)?;
    let frame = renderer.render(&grouped, config)?;
    let primary = read(frame.gpu(), &context)?;
    assert_eq!(primary.rgba, original_pixels.rgba);
    assert_eq!(primary.object_ids, original_pixels.object_ids);
    assert_eq!(primary.linear_depth, original_pixels.linear_depth);
    let occlusion = &frame.gpu().occlusion_groups()[0];
    assert_eq!(occlusion.group_id(), 42);
    assert_eq!(occlusion.parent_frame_id(), frame.frame_id());
    let pixels = read(occlusion.gpu(), &context)?;
    let left = 32 * 64 + 16;
    let right = 32 * 64 + 48;
    assert_eq!(pixels.object_ids.as_ref().unwrap()[left], 3);
    assert_eq!(occlusion.auxiliary_index(3), Some(0));
    assert_eq!(pixels.object_ids.as_ref().unwrap()[right], 2);
    assert!((pixels.linear_depth.as_ref().unwrap()[left] - 4.).abs() < 1e-4);
    assert!((pixels.linear_depth.as_ref().unwrap()[right] - 3.6).abs() < 1e-4);
    assert_eq!(primary.object_ids.as_ref().unwrap()[right], 1);
    let retained = occlusion.clone();
    let repeated = renderer.render(&grouped, config)?;
    assert_ne!(retained.parent_frame_id(), repeated.frame_id());
    drop(frame);
    let retained_pixels = read(retained.gpu(), &context)?;
    assert_eq!(retained_pixels.linear_depth, pixels.linear_depth);

    let exact_budget = repeated.gpu().target_memory().total_bytes;
    renderer.set_target_byte_limit(Some(exact_budget - 1));
    assert!(renderer.render(&grouped, config).is_err());
    renderer.set_target_byte_limit(Some(exact_budget));
    assert!(renderer.render(&grouped, config).is_ok());
    renderer.set_target_byte_limit(None);

    for eye in [[0., 0., 4.], [1., 0.5, 4.]] {
        for projection in [
            camera.projection,
            Projection::Perspective { vertical_fov: 0.6 },
        ] {
            let camera = Camera {
                eye,
                projection,
                ..camera
            };
            let grouped =
                scene(camera).edit_occlusion([EditOcclusionGroup::new(42, ["surface".into()])
                    .mesh(plane(-1., 1., 0.), AffineTransform::IDENTITY)]);
            let reference = Scene::new()
                .camera(camera)
                .object(Object::new(
                    plane(0., 1., 0.4),
                    Material::color(gpui::rgb(0xffffff)).double_sided(true),
                ))
                .object(Object::new(
                    plane(-1., 1., 0.),
                    Material::color(gpui::rgb(0xffffff)).double_sided(true),
                ));
            for size in [[64; 2], [128; 2]] {
                let config = Scene3dOutputConfig {
                    size,
                    channels: Scene3dChannels::LINEAR_DEPTH,
                    color_samples: 1,
                };
                let actual = renderer.render(&grouped, config)?;
                let expected = renderer.render(&reference, config)?;
                assert_eq!(
                    read(actual.gpu().occlusion_groups()[0].gpu(), &context)?.linear_depth,
                    read(expected.gpu(), &context)?.linear_depth
                );
            }
        }
    }
    Ok(())
}
