use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use gpui_3d::{
    AffineTransform, Mesh, MorphTarget, MorphTargets, PlaneOptions, Skin, SkinInfluence,
};
use std::{hint::black_box, sync::mpsc, thread, time::Duration};

const JOINTS: usize = 64;

struct Workload {
    mesh: Mesh,
    skin: Skin,
    morph: MorphTargets,
    palettes: [Vec<Vec<AffineTransform>>; 2],
    weights: [Vec<Vec<f32>>; 2],
}

impl Workload {
    fn new(side: u32, influences: usize, targets: usize, instances: usize) -> Self {
        let mesh = Mesh::subdivided_plane(PlaneOptions {
            size: [2.; 2],
            segments: [side - 1; 2],
        })
        .unwrap();
        let skin = Skin::new(
            [AffineTransform::IDENTITY; JOINTS],
            (0..mesh.vertex_count()).map(|vertex| {
                (0..influences)
                    .map(|influence| SkinInfluence {
                        joint: (vertex + influence * 7) % JOINTS,
                        weight: (influence + 1) as f32,
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .unwrap();
        let morph = MorphTargets::new(
            mesh.clone(),
            (0..targets).map(|target| {
                let amplitude = 0.005 / (target + 1) as f32;
                MorphTarget {
                    positions: Some(
                        mesh.vertices()
                            .iter()
                            .map(|v| {
                                [
                                    amplitude * v.position[1],
                                    0.,
                                    amplitude * (v.position[0] + 0.5),
                                ]
                            })
                            .collect(),
                    ),
                    normals: Some(vec![[amplitude, -amplitude, 0.]; mesh.vertex_count()].into()),
                    tangents: Some(vec![[0., amplitude, 0.]; mesh.vertex_count()].into()),
                }
            }),
        )
        .unwrap();
        let palettes = std::array::from_fn(|phase| {
            (0..instances)
                .map(|instance| {
                    (0..JOINTS)
                        .map(|joint| {
                            let angle = (joint + instance + phase) as f32 * 0.003;
                            AffineTransform::from_trs(
                                [0., joint as f32 * 0.001, phase as f32 * 0.01],
                                [0., 0., (angle * 0.5).sin(), (angle * 0.5).cos()],
                                [1.; 3],
                            )
                            .unwrap()
                        })
                        .collect()
                })
                .collect()
        });
        let weights = std::array::from_fn(|phase| {
            (0..instances)
                .map(|instance| {
                    (0..targets)
                        .map(|target| 0.2 + ((target + instance + phase) % 7) as f32 * 0.1)
                        .collect()
                })
                .collect()
        });
        Self {
            mesh,
            skin,
            morph,
            palettes,
            weights,
        }
    }

    fn evaluate(&self, phase: usize, instance: usize) -> Mesh {
        let morphed = self
            .morph
            .evaluate(black_box(&self.weights[phase][instance]))
            .unwrap();
        self.skin
            .evaluate(&morphed, black_box(&self.palettes[phase][instance]))
            .unwrap()
    }

    fn serial_frame(&self, phase: usize) -> Vec<(usize, Mesh)> {
        (0..self.palettes[phase].len())
            .map(|instance| (instance, self.evaluate(phase, instance)))
            .collect()
    }
}

fn skin(c: &mut Criterion) {
    let mut group = c.benchmark_group("skin_cpu");
    for side in [32, 128, 256] {
        for influences in [1, 4, 8] {
            let workload = Workload::new(side, influences, 1, 1);
            group.throughput(Throughput::Elements(workload.mesh.vertex_count() as u64));
            group.bench_function(
                BenchmarkId::new(
                    format!("influences_{influences}"),
                    workload.mesh.vertex_count(),
                ),
                |b| {
                    let mut phase = 0;
                    b.iter(|| {
                        phase ^= 1;
                        black_box(
                            workload
                                .skin
                                .evaluate(
                                    black_box(&workload.mesh),
                                    black_box(&workload.palettes[phase][0]),
                                )
                                .unwrap(),
                        );
                    });
                },
            );
        }
    }
    group.finish();
}

fn morph(c: &mut Criterion) {
    let mut group = c.benchmark_group("morph_cpu");
    for side in [32, 128, 256] {
        for targets in [1, 8, 32] {
            let workload = Workload::new(side, 1, targets, 1);
            group.throughput(Throughput::Elements(workload.mesh.vertex_count() as u64));
            group.bench_function(
                BenchmarkId::new(format!("targets_{targets}"), workload.mesh.vertex_count()),
                |b| {
                    let mut phase = 0;
                    b.iter(|| {
                        phase ^= 1;
                        black_box(
                            workload
                                .morph
                                .evaluate(black_box(&workload.weights[phase][0]))
                                .unwrap(),
                        );
                    });
                },
            );
        }
    }
    group.finish();
    let mut group = c.benchmark_group("morph_active_cpu");
    let workload = Workload::new(128, 1, 32, 1);
    for active in [0, 4, 32] {
        let weights: [Vec<f32>; 2] = std::array::from_fn(|phase| {
            workload.weights[phase][0]
                .iter()
                .enumerate()
                .map(|(index, &weight)| if index < active { weight } else { 0. })
                .collect()
        });
        group.bench_function(BenchmarkId::new("active_of_32", active), |b| {
            let mut phase = 0;
            b.iter(|| {
                phase ^= 1;
                black_box(workload.morph.evaluate(black_box(&weights[phase])).unwrap());
            });
        });
    }
    group.finish();
}

fn parallel_frame(
    commands: &[mpsc::Sender<usize>],
    results: &[mpsc::Receiver<Vec<(usize, Mesh)>>],
    phase: usize,
) -> Vec<(usize, Mesh)> {
    for command in commands {
        command.send(phase).unwrap();
    }
    results
        .iter()
        .flat_map(|result| result.recv().unwrap())
        .collect()
}

fn batches(c: &mut Criterion) {
    let mut group = c.benchmark_group("deformation_batch_cpu");
    for instances in [1, 8, 32] {
        let workload = Workload::new(128, 4, 8, instances);
        group.throughput(Throughput::Elements(
            (workload.mesh.vertex_count() * instances) as u64,
        ));
        group.bench_function(BenchmarkId::new("serial", instances), |b| {
            let mut phase = 0;
            b.iter(|| {
                phase ^= 1;
                black_box(workload.serial_frame(phase));
            });
        });
        if instances == 1 {
            continue;
        }
        group.bench_function(BenchmarkId::new("workers_4", instances), |b| {
            thread::scope(|scope| {
                let workload = &workload;
                let (commands, results): (Vec<_>, Vec<_>) = (0..4)
                    .map(|worker| {
                        let (send, receive) = mpsc::channel::<usize>();
                        let (completed, results) = mpsc::channel();
                        scope.spawn(move || {
                            while let Ok(phase) = receive.recv() {
                                let outputs = (worker..instances)
                                    .step_by(4)
                                    .map(|instance| (instance, workload.evaluate(phase, instance)))
                                    .collect();
                                if completed.send(outputs).is_err() {
                                    break;
                                }
                            }
                        });
                        (send, results)
                    })
                    .unzip();
                for phase in 0..2 {
                    let expected = workload.serial_frame(phase);
                    let actual = parallel_frame(&commands, &results, phase);
                    assert_eq!(actual.len(), instances);
                    let mut seen = vec![false; instances];
                    for (instance, mesh) in actual {
                        assert!(!seen[instance]);
                        seen[instance] = true;
                        let expected = &expected[instance].1;
                        assert_eq!(mesh.vertex_count(), expected.vertex_count());
                        for (a, b) in mesh.vertices().iter().zip(expected.vertices()) {
                            assert_eq!(a.position, b.position);
                            assert_eq!(a.normal, b.normal);
                        }
                        assert_eq!(mesh.tangents(), expected.tangents());
                        assert_eq!(mesh.bounds(), expected.bounds());
                    }
                }
                let mut phase = 0;
                b.iter(|| {
                    phase ^= 1;
                    black_box(parallel_frame(&commands, &results, phase));
                });
                drop(commands);
            });
        });
    }
    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(10)
        .warm_up_time(Duration::from_millis(200)).measurement_time(Duration::from_secs(1));
    targets = skin, morph, batches
}
criterion_main!(benches);
