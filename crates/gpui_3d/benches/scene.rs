use std::{hint::black_box, time::Duration};

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use gpui_3d::{
    Camera, Material, Mesh, Object, PbrMaterial, Projection, ResolvedTexture, Scene, TextureSource,
    TextureState,
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
    let mut group = c.benchmark_group("scene_preparation");
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
                b.iter(|| {
                    black_box(
                        scene
                            .prepare(black_box(1.), None, |request| {
                                Ok(match request.source {
                                    TextureSource::Image(_) => TextureState::Pending,
                                    TextureSource::Solid => {
                                        TextureState::Ready(ResolvedTexture::None)
                                    }
                                    TextureSource::Ui => unreachable!(),
                                })
                            })
                            .unwrap(),
                    )
                });
            });
        }
    }
    group.finish();
}

criterion_group!(benches, preparation);

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
criterion_group!(draw_benches, draw_planning);
#[cfg(feature = "wgpu")]
criterion_main!(benches, draw_benches);
#[cfg(not(feature = "wgpu"))]
criterion_main!(benches);
