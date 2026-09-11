use std::{hint::black_box, time::Duration};

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use gpui_3d::{
    Camera, Material, Mesh, Object, PbrMaterial, PreparationCache, Projection, ResolvedTexture,
    Scene, TextureRequest, TextureSource, TextureState,
};

fn scene(count: usize, workload: &str) -> Scene {
    let mesh = Mesh::cube();
    let columns = (count as f32).sqrt().ceil() as usize;
    let mut scene = Scene::new().camera(Camera {
        projection: Projection::Orthographic { vertical_size: 20. },
        ..Default::default()
    });
    for index in 0..count {
        let material = match workload {
            "pending_images" => Material::image("surface.png"),
            "blended" => {
                Material::color(gpui::rgba(0x80a0c080)).alpha_mode(gpui_3d::AlphaMode::Blend)
            }
            "mixed_materials" => Material::color(gpui::rgb(0x80a0c0)).pbr(PbrMaterial {
                metallic: (index % 4) as f32 / 3.,
                roughness: 0.1 + (index % 8) as f32 * 0.1,
                ..Default::default()
            }),
            _ => Material::color(gpui::rgb(0x80a0c0)),
        };
        let mut x = ((index % columns) as f32 / columns as f32 - 0.5) * 16.;
        if workload == "mostly_culled" && !index.is_multiple_of(4) {
            x += 100.;
        }
        let y = ((index / columns) as f32 / columns as f32 - 0.5) * 16.;
        scene = scene.object(
            Object::new(mesh.clone(), material)
                .position([x, y, 0.])
                .scale([0.08; 3]),
        );
    }
    scene
}

fn preparation(c: &mut Criterion) {
    for retained in [false, true] {
        let mut group = c.benchmark_group(if retained {
            "retained_preparation"
        } else {
            "scene_preparation"
        });
        group.sample_size(10);
        group.warm_up_time(Duration::from_millis(200));
        group.measurement_time(Duration::from_millis(500));
        for count in [1024, 16384] {
            group.throughput(Throughput::Elements(count as u64));
            for workload in [
                "shared_geometry",
                "mixed_materials",
                "mostly_culled",
                "pending_images",
            ] {
                let scene = scene(count, workload);
                group.bench_with_input(BenchmarkId::new(workload, count), &scene, |b, scene| {
                    let resolve = |request: TextureRequest<'_>| {
                        Ok(match request.source {
                            TextureSource::Image(_) => TextureState::Pending,
                            TextureSource::Solid => TextureState::Ready(ResolvedTexture::None),
                            TextureSource::Ui => unreachable!(),
                        })
                    };
                    let mut cache = PreparationCache::new();
                    if retained {
                        let first = cache.prepare(scene, 1., None, resolve).unwrap();
                        let next = cache.prepare(scene, 1., None, resolve).unwrap();
                        assert!(std::sync::Arc::ptr_eq(&first, &next));
                    }
                    b.iter(|| {
                        if retained {
                            black_box(
                                cache
                                    .prepare(black_box(scene), black_box(1.), None, resolve)
                                    .unwrap(),
                            );
                        } else {
                            black_box(scene.prepare(black_box(1.), None, resolve).unwrap());
                        }
                    });
                });
            }
        }
        group.finish();
    }
}

fn alternating_cameras(c: &mut Criterion) {
    let mut group = c.benchmark_group("alternating_cameras");
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(200));
    group.measurement_time(Duration::from_millis(500));
    for count in [1024, 16384] {
        let front = scene(count, "shared_geometry");
        let side = front.clone().camera(Camera::orbit(0.4, 0.2, 20.));
        group.throughput(Throughput::Elements(count as u64 * 2));
        for capacity in [1, 2] {
            group.bench_function(
                BenchmarkId::new(format!("capacity_{capacity}"), count),
                |b| {
                    let mut cache = PreparationCache::with_capacity(capacity);
                    let resolve =
                        |_: TextureRequest<'_>| Ok(TextureState::Ready(ResolvedTexture::None));
                    let first = cache.prepare(&front, 1., None, resolve).unwrap();
                    cache.prepare(&side, 1., None, resolve).unwrap();
                    let next = cache.prepare(&front, 1., None, resolve).unwrap();
                    assert_eq!(std::sync::Arc::ptr_eq(&first, &next), capacity == 2);
                    b.iter(|| {
                        for scene in [&front, &side] {
                            black_box(cache.prepare(black_box(scene), 1., None, resolve).unwrap());
                        }
                    });
                },
            );
        }
    }
    group.finish();
}

fn resource_rebinding(c: &mut Criterion) {
    let tile = gpui::AtlasTile {
        texture_id: gpui::AtlasTextureId {
            index: 0,
            kind: gpui::AtlasTextureKind::Polychrome,
        },
        tile_id: gpui::TileId(0),
        padding: 0,
        bounds: gpui::Bounds::new(
            gpui::point(gpui::DevicePixels(0), gpui::DevicePixels(0)),
            gpui::size(gpui::DevicePixels(16), gpui::DevicePixels(16)),
        ),
    };
    let ready = TextureState::Ready(ResolvedTexture::Image(tile));
    let relocated = TextureState::Ready(ResolvedTexture::Image(gpui::AtlasTile {
        tile_id: gpui::TileId(1),
        ..tile
    }));
    let mut group = c.benchmark_group("resource_rebinding");
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(200));
    group.measurement_time(Duration::from_millis(500));
    for count in [1024, 16384] {
        let scene = scene(count, "pending_images");
        group.throughput(Throughput::Elements(count as u64 * 2));
        for (workload, states) in [
            ("readiness", [TextureState::Pending, ready]),
            ("atlas_relocation", [ready, relocated]),
        ] {
            for capacity in [0, 1] {
                group.bench_function(
                    BenchmarkId::new(format!("{workload}/capacity_{capacity}"), count),
                    |b| {
                        let mut cache = PreparationCache::with_capacity(capacity);
                        for state in states {
                            let output = cache.prepare(&scene, 1., None, |_| Ok(state)).unwrap();
                            let pending = matches!(state, TextureState::Pending);
                            assert_eq!(output.is_ready(), !pending);
                            assert_eq!(
                                output.frame().objects.len(),
                                if pending { 0 } else { count }
                            );
                        }
                        b.iter(|| {
                            for state in states {
                                black_box(
                                    cache
                                        .prepare(black_box(&scene), 1., None, |_| {
                                            Ok(black_box(state))
                                        })
                                        .unwrap(),
                                );
                            }
                        });
                    },
                );
            }
        }
    }
    group.finish();
}

fn spatial_index(c: &mut Criterion) {
    use gpui_3d::{AffineTransform, Node, SceneGraph};
    let mut group = c.benchmark_group("spatial_index");
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(200));
    group.measurement_time(Duration::from_millis(500));
    for count in [1024, 16384] {
        let mut graph = SceneGraph::new();
        let mesh = Mesh::cube();
        let columns = (count as f32).sqrt().ceil() as usize;
        let nodes: Vec<_> = (0..count)
            .map(|i| {
                let position = [(i % columns) as f32 * 2., (i / columns) as f32 * 2., 0.];
                let handle = graph
                    .insert(
                        None,
                        Node::new()
                            .mesh(mesh.clone(), Material::color(gpui::rgb(0x80a0c0)))
                            .transform(AffineTransform::from_translation(position).unwrap()),
                    )
                    .unwrap();
                (handle, position)
            })
            .collect();
        group.throughput(Throughput::Elements(count as u64));
        for (workload, stride) in [
            ("unchanged", None),
            ("sparse_motion", Some(100)),
            ("all_motion", Some(1)),
        ] {
            let previous = graph.evaluate().unwrap();
            previous.prepare_spatial_index();
            for (i, &(node, mut position)) in nodes.iter().enumerate() {
                if stride.is_some_and(|stride| i.is_multiple_of(stride)) {
                    position[2] = 0.25;
                    graph
                        .set_transform(node, AffineTransform::from_translation(position).unwrap())
                        .unwrap();
                }
            }
            for refit in [false, true] {
                let mode = if refit { "refit" } else { "rebuild" };
                group.bench_function(BenchmarkId::new(format!("{workload}/{mode}"), count), |b| {
                    b.iter_batched(
                        || graph.evaluate().unwrap(),
                        |current| {
                            if refit {
                                current.prepare_spatial_index_from(black_box(&previous));
                            } else {
                                current.prepare_spatial_index();
                            }
                            black_box(current);
                        },
                        BatchSize::PerIteration,
                    );
                });
            }
            for &(node, position) in &nodes {
                graph
                    .set_transform(node, AffineTransform::from_translation(position).unwrap())
                    .unwrap();
            }
        }
    }
    group.finish();
}

criterion_group!(
    benches,
    preparation,
    alternating_cameras,
    resource_rebinding,
    spatial_index
);

#[cfg(feature = "wgpu")]
fn draw_planning(c: &mut Criterion) {
    use gpui_3d::{Scene3dChannels, Scene3dDrawStatistics};
    let mut group = c.benchmark_group("draw_planning");
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(200));
    group.measurement_time(Duration::from_millis(500));
    for count in [1024, 16384] {
        for workload in [
            "shared_geometry",
            "mixed_materials",
            "mostly_culled",
            "blended",
        ] {
            let input = if workload == "mostly_culled" {
                "shared_geometry"
            } else {
                workload
            };
            let mut frame = scene(count, input)
                .prepare(1., None, |_| Ok(TextureState::Ready(ResolvedTexture::None)))
                .unwrap()
                .into_frame();
            if workload == "mostly_culled" {
                for (index, object) in std::sync::Arc::make_mut(&mut frame.objects)
                    .iter_mut()
                    .enumerate()
                {
                    if !index.is_multiple_of(4) {
                        object.model[3][0] += 100.;
                    }
                }
            }
            let visible = if workload == "mostly_culled" {
                count / 4
            } else {
                count
            };
            let expected_draws = if matches!(workload, "mixed_materials" | "blended") {
                visible
            } else {
                1
            };
            let statistics =
                Scene3dDrawStatistics::plan(&frame, Scene3dChannels::COLOR, 65536).unwrap();
            assert_eq!(statistics.camera_instances, visible as u64);
            assert_eq!(statistics.camera_draws, expected_draws as u64);
            group.throughput(Throughput::Elements(count as u64));
            group.bench_with_input(BenchmarkId::new(workload, count), &frame, |b, frame| {
                b.iter(|| {
                    black_box(
                        Scene3dDrawStatistics::plan(
                            black_box(frame),
                            Scene3dChannels::COLOR,
                            65536,
                        )
                        .unwrap(),
                    )
                });
            });
        }
    }
    group.finish();
}

#[cfg(feature = "wgpu")]
fn draw_encoding(c: &mut Criterion) {
    use gpui_wgpu::{
        Scene3dChannels, Scene3dDrawStatistics, Scene3dOutputConfig, WgpuContext,
        WgpuScene3dRenderer, wgpu,
    };
    use std::{sync::Arc, time::Instant};

    if std::env::var("GPUI_3D_GPU_BENCH").as_deref() != Ok("1") {
        return;
    }
    let context = WgpuContext::new_headless().expect("create benchmark GPU context");
    eprintln!("3D benchmark adapter: {:?}", context.adapter.get_info());
    let wait = || {
        context
            .device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(Duration::from_secs(30)),
            })
            .expect("finish benchmark GPU submission");
    };
    let mut group = c.benchmark_group("draw_encoding");
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(200));
    group.measurement_time(Duration::from_secs(1));
    for count in [1024, 16384] {
        for workload in [
            "shared_geometry",
            "mixed_materials",
            "mostly_culled",
            "vertex_updates",
        ] {
            let input = scene(count, workload)
                .prepare(1., None, |_| Ok(TextureState::Ready(ResolvedTexture::None)))
                .unwrap()
                .into_frame();
            let mesh = input.objects[0].mesh.clone();
            for (mode, channels) in [
                ("color", Scene3dChannels::COLOR),
                (
                    "multi_output",
                    Scene3dChannels::COLOR
                        | Scene3dChannels::OBJECT_ID
                        | Scene3dChannels::LINEAR_DEPTH
                        | Scene3dChannels::WORLD_NORMAL,
                ),
            ] {
                let config = Scene3dOutputConfig {
                    size: [256, 256],
                    channels,
                    color_samples: 1,
                };
                let visible = if workload == "mostly_culled" {
                    count / 4
                } else {
                    count
                };
                let passes = if mode == "color" { 1 } else { 4 };
                let mut state = None;
                group.throughput(Throughput::Elements((visible * passes) as u64));
                group.bench_function(BenchmarkId::new(format!("{workload}/{mode}"), count), |b| {
                    let (renderer, expected) = state.get_or_insert_with(|| {
                        let mut renderer = WgpuScene3dRenderer::new(context.clone()).unwrap();
                        renderer
                            .capabilities()
                            .validate(config)
                            .expect("benchmark output support");
                        let expected = Scene3dDrawStatistics::plan(
                            &input,
                            channels,
                            renderer.max_instances_per_batch(),
                        )
                        .unwrap();
                        assert_eq!(expected.camera_instances, (visible * passes) as u64);
                        let draws = if workload == "mixed_materials" {
                            visible
                        } else {
                            visible.div_ceil(renderer.max_instances_per_batch())
                        };
                        assert_eq!(expected.camera_draws, (draws * passes) as u64);
                        let first = renderer.render(&input, config).unwrap();
                        wait();
                        assert_eq!(first.draw_statistics(), expected);
                        (renderer, expected)
                    });
                    b.iter_custom(|iterations| {
                        let mut elapsed = Duration::ZERO;
                        for iteration in 0..iterations {
                            let mut frame = input.clone();
                            if workload == "vertex_updates" {
                                let mut vertices = mesh.vertices().to_vec();
                                let offset = if iteration.is_multiple_of(2) {
                                    0.01
                                } else {
                                    -0.01
                                };
                                for vertex in &mut vertices {
                                    vertex.position[2] += offset;
                                }
                                let updated = mesh
                                    .with_vertices(vertices, mesh.tangents().map(<[_]>::to_vec))
                                    .unwrap();
                                for object in Arc::make_mut(&mut frame.objects) {
                                    object.mesh = updated.clone();
                                }
                            }
                            let start = Instant::now();
                            let output = renderer.render(black_box(&frame), config).unwrap();
                            elapsed += start.elapsed();
                            wait();
                            assert_eq!(output.draw_statistics(), *expected);
                            black_box(output);
                        }
                        elapsed
                    });
                });
            }
        }
    }
    group.finish();
}

#[cfg(feature = "wgpu")]
criterion_group!(draw_benches, draw_planning, draw_encoding);
#[cfg(feature = "wgpu")]
criterion_main!(benches, draw_benches);
#[cfg(not(feature = "wgpu"))]
criterion_main!(benches);
