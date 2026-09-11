use super::*;
use gpui::{point, px, size};

#[test]
fn capture_errors_follow_source_frames() {
    let scene = crate::Scene::new();
    let prepared = scene
        .prepare(1., None, |_| {
            Ok(crate::TextureState::Ready(gpui::MeshTexture3d::None))
        })
        .unwrap();
    let first = Arc::new(prepared.frame().clone());
    let second = Arc::new(prepared.frame().clone());
    let capture = ViewportPickCapture::new(1 << 20);
    let owner = Rc::new(());
    let layout = PickLayout {
        bounds: Bounds::new(point(px(0.), px(0.)), size(px(100.), px(100.))),
        scale: 1.,
        surface: size(px(100.), px(100.)),
    };
    capture.bind(first.clone(), &prepared, scene.camera, layout, &owner);
    capture
        .backend
        .publish::<u32>(&first, Err("frame preparation failed".into()));
    assert!(capture.frame().is_err());
    capture.bind(second.clone(), &prepared, scene.camera, layout, &owner);
    assert!(capture.frame().unwrap().is_none());
    capture
        .backend
        .publish::<u32>(&second, Err("current frame failed".into()));
    assert_eq!(
        capture.frame().err().unwrap().to_string(),
        "current frame failed"
    );
    capture.bind(first, &prepared, scene.camera, layout, &owner);
    assert!(capture.frame().unwrap().is_none());
}

#[test]
fn logical_pointer_mapping_respects_origin_and_half_open_edges() {
    let bounds = Bounds::new(point(px(20.5), px(40.25)), size(px(80.), px(50.)));
    assert_eq!(
        normalized(bounds, point(px(60.5), px(65.25))),
        Some([0.5, 0.5])
    );
    assert_eq!(normalized(bounds, bounds.origin), Some([0., 0.]));
    for position in [
        point(px(100.5), px(50.)),
        point(px(30.), px(90.25)),
        point(px(20.), px(50.)),
        point(px(f32::NAN), px(50.)),
    ] {
        assert_eq!(normalized(bounds, position), None);
    }
    assert_eq!(normalized(Bounds::default(), point(px(0.), px(0.))), None);
}

#[test]
fn capture_bindings_preserve_source_metadata_and_expire_with_mounts() {
    let scene = crate::Scene::new();
    let prepared = scene
        .prepare(1.6, None, |_| {
            Ok(crate::TextureState::Ready(gpui::MeshTexture3d::None))
        })
        .unwrap();
    let frame = Arc::new(prepared.frame().clone());
    let capture = ViewportPickCapture::new(1 << 20);
    let owner = Rc::new(());
    let layout = PickLayout {
        bounds: Bounds::new(point(px(-20.), px(30.)), size(px(80.), px(50.))),
        scale: 1.25,
        surface: size(px(500.), px(300.)),
    };
    capture.bind(frame.clone(), &prepared, scene.camera, layout, &owner);
    let original = capture.snapshot.borrow().clone().unwrap();
    assert_eq!(original.camera.aspect_ratio, Some(1.6));
    assert!(capture.frame().unwrap().is_none());
    capture.bind(frame.clone(), &prepared, scene.camera, layout, &owner);
    assert!(Rc::ptr_eq(
        &original,
        capture.snapshot.borrow().as_ref().unwrap()
    ));
    let resized = PickLayout {
        scale: 2.,
        ..layout
    };
    capture.bind(
        Arc::new(prepared.frame().clone()),
        &prepared,
        scene.camera,
        resized,
        &owner,
    );
    assert!(!Rc::ptr_eq(
        &original,
        capture.snapshot.borrow().as_ref().unwrap()
    ));
    assert!(Arc::ptr_eq(&original.frame, &frame));
    assert_eq!(original.layout.scale, 1.25);
    capture.clear();
    assert!(capture.snapshot.borrow().is_none());
    assert!(original.owner.upgrade().is_some());
    capture.bind(frame, &prepared, scene.camera, layout, &owner);
    drop(owner);
    assert!(original.owner.upgrade().is_none());
    assert!(capture.frame().unwrap().is_none());
}
