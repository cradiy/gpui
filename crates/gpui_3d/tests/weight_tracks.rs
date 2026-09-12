use gpui_3d::{AnimationError, Interpolation, Keyframe, WeightTrack};
use std::time::Duration;

fn seconds(value: f64) -> Duration {
    Duration::from_secs_f64(value)
}

fn close(actual: &[f32], expected: &[f32]) {
    assert_eq!(actual.len(), expected.len());
    for (&a, &b) in actual.iter().zip(expected) {
        assert!((a - b).abs() < 2e-4, "{actual:?} != {expected:?}");
    }
}

#[test]
fn weight_tracks_seek_clamp_and_hold_signed_unnormalized_components() {
    let keys = [
        Keyframe::new(seconds(2.), vec![0., -1., 2., 0.5, 4.]),
        Keyframe::new(seconds(4.), vec![2., 1., -2., 1.5, 8.]),
    ];
    let linear = WeightTrack::new(keys.clone(), Interpolation::Linear).unwrap();
    let copy = linear.clone();
    assert!(std::ptr::eq(linear.keyframes(), copy.keyframes()));
    for (time, expected) in [
        (seconds(8.), &keys[1].value[..]),
        (seconds(3.), &[1., 0., 0., 1., 6.][..]),
        (Duration::ZERO, &keys[0].value[..]),
        (seconds(4.), &keys[1].value[..]),
        (seconds(3.), &[1., 0., 0., 1., 6.][..]),
    ] {
        close(&linear.sample(time).unwrap(), expected);
        let mut output = vec![f32::NAN; linear.weight_count()];
        copy.sample_into(time, &mut output).unwrap();
        close(&output, expected);
    }
    let step = WeightTrack::new(keys.clone(), Interpolation::Step).unwrap();
    close(&step.sample(seconds(3.999)).unwrap(), &keys[0].value);
    close(&step.sample(seconds(4.)).unwrap(), &keys[1].value);
    for mode in [
        Interpolation::Step,
        Interpolation::Linear,
        Interpolation::CubicSpline,
    ] {
        let constant = WeightTrack::new([keys[1].clone()], mode).unwrap();
        for time in [Duration::ZERO, Duration::MAX] {
            close(&constant.sample(time).unwrap(), &keys[1].value);
        }
    }
}

#[test]
fn cubic_weight_derivatives_reproduce_polynomials_across_unequal_segments() {
    let values = |t: f64| {
        vec![
            t.powi(3) as f32,
            (2. * t) as f32,
            -t.powi(2) as f32,
            -0.5,
            (1. - t) as f32,
        ]
    };
    let derivatives = |t: f64| vec![(3. * t * t) as f32, 2., (-2. * t) as f32, 0., -1.];
    let track = WeightTrack::new(
        [2., 5., 9.]
            .map(|t| Keyframe::new(seconds(t), values(t)).tangents(derivatives(t), derivatives(t))),
        Interpolation::CubicSpline,
    )
    .unwrap();
    for t in [8.5_f64, 3., 2., 7.25, 5., 4., 10.] {
        close(&track.sample(seconds(t)).unwrap(), &values(t.min(9.)));
    }
    let zero_derivatives = WeightTrack::new(
        [
            Keyframe::new(Duration::ZERO, vec![0., 2.]),
            Keyframe::new(seconds(4.), vec![4., -2.]),
        ],
        Interpolation::CubicSpline,
    )
    .unwrap();
    close(
        &zero_derivatives.sample(seconds(1.)).unwrap(),
        &[0.625, 1.375],
    );
}

#[test]
fn weight_sampling_preserves_small_offsets_at_large_absolute_times() {
    let start = Duration::from_secs(u64::MAX - 1);
    let track = WeightTrack::new(
        [
            Keyframe::new(start, vec![-f32::MAX, 2., 4., -3., 0., 1.]),
            Keyframe::new(
                start + Duration::from_nanos(4),
                vec![f32::MAX, 6., -4., 1., 2., 3.],
            ),
        ],
        Interpolation::Linear,
    )
    .unwrap();
    close(
        &track.sample(start + Duration::from_nanos(2)).unwrap(),
        &[0., 4., 0., -1., 1., 2.],
    );
}

#[test]
fn malformed_weight_keys_report_component_and_time_errors() {
    assert_eq!(
        WeightTrack::new([], Interpolation::Linear).unwrap_err(),
        AnimationError::EmptyTrack
    );
    assert_eq!(
        WeightTrack::new([Keyframe::new(Duration::ZERO, vec![])], Interpolation::Step).unwrap_err(),
        AnimationError::EmptyWeights
    );
    let key = Keyframe::new(Duration::ZERO, vec![0., 1.]);
    assert_eq!(
        WeightTrack::new(
            [key.clone(), Keyframe::new(seconds(1.), vec![1.])],
            Interpolation::Linear
        )
        .unwrap_err(),
        AnimationError::WeightCount {
            key: 1,
            expected: 2,
            actual: 1
        }
    );
    for mode in [
        Interpolation::Step,
        Interpolation::Linear,
        Interpolation::CubicSpline,
    ] {
        assert_eq!(
            WeightTrack::new([key.clone().tangents(vec![0.], vec![])], mode).unwrap_err(),
            AnimationError::WeightTangentCount {
                key: 0,
                expected: 2,
                incoming: 1,
                outgoing: 0
            }
        );
        assert_eq!(
            WeightTrack::new([key.clone().tangents(vec![], vec![0.; 3])], mode).unwrap_err(),
            AnimationError::WeightTangentCount {
                key: 0,
                expected: 2,
                incoming: 0,
                outgoing: 3
            }
        );
        for key in [
            Keyframe::new(Duration::ZERO, vec![f32::NAN, 1.]),
            key.clone().tangents(vec![f32::INFINITY, 0.], vec![]),
            key.clone().tangents(vec![], vec![0., f32::NEG_INFINITY]),
        ] {
            assert_eq!(
                WeightTrack::new([key], mode).unwrap_err(),
                AnimationError::NonFiniteKey { key: 0 }
            );
        }
    }
    for end in [Duration::ZERO, seconds(1.)] {
        assert_eq!(
            WeightTrack::new(
                [
                    Keyframe::new(seconds(1.), vec![0.]),
                    Keyframe::new(end, vec![1.])
                ],
                Interpolation::Linear
            )
            .unwrap_err(),
            AnimationError::NonIncreasingTime { key: 1 }
        );
    }
}

#[test]
fn output_buffers_remain_unchanged_on_size_errors_and_cubic_overflow() {
    let track = WeightTrack::new(
        [
            Keyframe::new(Duration::ZERO, vec![1., 0.]).tangents(vec![], vec![0., f32::MAX]),
            Keyframe::new(seconds(10.), vec![3., 0.]).tangents(vec![0., -f32::MAX], vec![]),
        ],
        Interpolation::CubicSpline,
    )
    .unwrap();
    let mut output = [17., 23.];
    assert_eq!(
        track.sample_into(seconds(5.), &mut output),
        Err(AnimationError::InvalidSample)
    );
    assert_eq!(output, [17., 23.]);
    assert_eq!(
        track.sample(seconds(5.)),
        Err(AnimationError::InvalidSample)
    );
    let mut small = [17.];
    assert_eq!(
        track.sample_into(Duration::ZERO, &mut small),
        Err(AnimationError::OutputCount {
            expected: 2,
            actual: 1
        })
    );
    assert_eq!(small, [17.]);
    track.sample_into(seconds(10.), &mut output).unwrap();
    assert_eq!(output, [3., 0.]);
}

#[test]
fn weight_samples_drive_independent_morph_skin_bounds_and_queries() {
    use gpui::rgb;
    use gpui_3d::{
        AffineTransform, Camera, Material, Mesh, MorphTarget, MorphTargets, Object, Ray,
        ResolvedTexture, Scene, Skin, SkinInfluence, TextureState,
    };

    let base = Mesh::plane();
    let morph = MorphTargets::new(
        base.clone(),
        [
            MorphTarget {
                positions: Some(vec![[0., 0., 2.]; base.vertex_count()].into()),
                ..Default::default()
            },
            MorphTarget {
                positions: Some(vec![[4., 0., 0.]; base.vertex_count()].into()),
                ..Default::default()
            },
        ],
    )
    .unwrap();
    let skin = Skin::new(
        [AffineTransform::IDENTITY],
        (0..base.vertex_count()).map(|_| {
            [SkinInfluence {
                joint: 0,
                weight: 1.,
            }]
        }),
    )
    .unwrap();
    let joints = [AffineTransform::from_translation([0., 2., -0.5]).unwrap()];
    let track = WeightTrack::new(
        [
            Keyframe::new(Duration::ZERO, vec![0., 0.]),
            Keyframe::new(seconds(4.), vec![2., 1.]),
        ],
        Interpolation::Linear,
    )
    .unwrap();
    let bind = skin
        .evaluate(
            &morph
                .evaluate(&track.sample(Duration::ZERO).unwrap())
                .unwrap(),
            &joints,
        )
        .unwrap();
    for time in [1., 4., 0., 3., 1.] {
        let mesh = skin
            .evaluate(
                &morph
                    .evaluate(&track.sample(seconds(time)).unwrap())
                    .unwrap(),
                &joints,
            )
            .unwrap();
        let x = time as f32;
        let z = time as f32 - 0.5;
        assert_eq!(mesh.bounds().min(), [x - 0.5, 1.5, z]);
        assert_eq!(mesh.bounds().max(), [x + 0.5, 2.5, z]);
        let scene = Scene::new()
            .camera(Camera {
                eye: [x, 2., 6.],
                target: [x, 2., z],
                ..Default::default()
            })
            .object(Object::new(mesh.clone(), Material::color(rgb(0xffffff))).id("animated"));
        let hit = scene
            .raycast(Ray::new([x, 2., 6.], [0., 0., -1.]).unwrap())
            .unwrap();
        assert_eq!(hit.position, [x, 2., z]);
        assert_eq!(hit.distance, 6. - z);
        let prepared = scene
            .prepare(1., None, |_| Ok(TextureState::Ready(ResolvedTexture::None)))
            .unwrap();
        for (actual, expected) in prepared.frame().objects[0]
            .mesh
            .vertices()
            .iter()
            .zip(mesh.vertices())
        {
            assert_eq!(actual.position, expected.position);
            assert_eq!(actual.normal, expected.normal);
        }
        assert_eq!(bind.bounds().min(), [-0.5, 1.5, -0.5]);
        assert_eq!(base.bounds().min(), [-0.5, -0.5, 0.]);
    }
}
