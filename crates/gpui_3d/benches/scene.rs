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
criterion_main!(benches);
