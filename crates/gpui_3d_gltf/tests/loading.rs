use std::{
    future::Future,
    task::{Context, Poll, Waker},
};

use futures::{channel::oneshot, executor::block_on};
use gpui_3d_gltf::{Document, Limits, SceneAsset, SceneLoadSlot, SceneLoadStatus, SceneOptions};
use serde_json::json;

fn definition(index: usize) -> gpui_3d_gltf::SceneDefinition {
    let document = Document::from_slice(
        &serde_json::to_vec(&json!({
            "asset":{"version":"2.0"}, "scenes":[{"nodes":[0]},{"nodes":[1]}],
            "nodes":[{"name":"first"},{"name":"second"}]
        }))
        .unwrap(),
        Limits::default(),
    )
    .unwrap();
    document
        .prepare(|_| unreachable!())
        .unwrap()
        .scene(Some(index), SceneOptions::default())
        .unwrap()
}

fn asset(index: usize) -> SceneAsset {
    definition(index)
        .resolve_images(|_, _| unreachable!())
        .unwrap()
}

#[test]
fn completed_results_cannot_replace_newer_or_foreign_requests() {
    let mut slot = SceneLoadSlot::new();
    let old = block_on(slot.begin().run(std::future::ready(Ok(asset(0)))));
    let current = slot.begin();
    assert!(!slot.accept(old));
    assert_eq!(slot.status(), SceneLoadStatus::Loading);
    assert!(slot.asset().is_none());
    let ready = block_on(current.run(std::future::ready(Ok(asset(1)))));
    assert!(slot.accept(ready));
    assert_eq!(slot.asset().unwrap().index(), 1);

    let late_failure = block_on(slot.begin().run(async { anyhow::bail!("old failure") }));
    let newer = slot.begin();
    assert!(!slot.accept(late_failure));
    assert!(slot.error().is_none());
    assert_eq!(slot.asset().unwrap().index(), 1);
    let ready = block_on(newer.run(std::future::ready(Ok(asset(0)))));
    assert!(slot.accept(ready));
    assert_eq!(slot.status(), SceneLoadStatus::Ready);

    let mut other = SceneLoadSlot::new();
    let local = slot.begin();
    let foreign = block_on(other.begin().run(std::future::ready(Ok(asset(1)))));
    assert!(!slot.accept(foreign));
    assert_eq!(slot.asset().unwrap().index(), 0);
    assert_eq!(slot.status(), SceneLoadStatus::Loading);
    assert_eq!(other.status(), SceneLoadStatus::Cancelled);
    let ready = block_on(local.run(std::future::ready(Ok(asset(1)))));
    assert!(slot.accept(ready));
    assert_eq!(slot.asset().unwrap().index(), 1);
}

#[test]
fn worker_payloads_are_checked_before_local_resolution() {
    let mut slot = SceneLoadSlot::new();
    let source = definition(1);
    let work = slot
        .begin()
        .run(async move { source.decode_resources(gpui_3d_gltf::ImageDecodeLimits::default()) });
    let old = std::thread::spawn(move || block_on(work)).join().unwrap();
    let current = slot.begin();
    assert!(!slot.accept_with(old, |_| panic!("stale data must not be resolved")));
    let source = definition(0);
    let work = current
        .run(async move { source.decode_resources(gpui_3d_gltf::ImageDecodeLimits::default()) });
    let ready = std::thread::spawn(move || block_on(work)).join().unwrap();
    assert!(slot.accept_with(ready, |decoded| decoded.resolve()));
    assert_eq!(slot.asset().unwrap().index(), 0);

    let ready = block_on(slot.begin().run(async { Ok(()) }));
    assert!(slot.accept_with(ready, |_| anyhow::bail!("local resolution failed")));
    assert_eq!(slot.status(), SceneLoadStatus::Failed);
    assert_eq!(slot.asset().unwrap().index(), 0);
    assert_eq!(slot.error().unwrap().to_string(), "local resolution failed");
    let failed = block_on(
        slot.begin()
            .run(async { Err::<(), _>(anyhow::anyhow!("worker failed")) }),
    );
    assert!(slot.accept_with(failed, |_| panic!("worker failure must skip resolution")));
    assert_eq!(slot.error().unwrap().to_string(), "worker failed");
    let ready = block_on(slot.begin().run(async { Ok(()) }));
    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        slot.accept_with(ready, |_| panic!("resolution interrupted"));
    }));
    assert!(unwound.is_err());
    assert_eq!(slot.status(), SceneLoadStatus::Cancelled);
    assert_eq!(slot.asset().unwrap().index(), 0);
}

#[test]
fn failures_and_cancellation_retain_assets_until_explicit_clear() {
    let mut slot = SceneLoadSlot::new();
    let ready = block_on(slot.begin().run(std::future::ready(Ok(asset(0)))));
    assert!(slot.accept(ready));
    let failed = block_on(slot.begin().run(async { anyhow::bail!("unavailable") }));
    assert!(slot.accept(failed));
    assert_eq!(slot.status(), SceneLoadStatus::Failed);
    assert_eq!(slot.error().unwrap().to_string(), "unavailable");
    assert_eq!(slot.asset().unwrap().index(), 0);
    assert!(!slot.cancel());

    let done = block_on(slot.begin().run(std::future::ready(Ok(asset(1)))));
    assert!(slot.error().is_none());
    assert!(slot.cancel());
    assert!(!slot.accept(done));
    assert_eq!(slot.status(), SceneLoadStatus::Cancelled);
    assert_eq!(slot.asset().unwrap().index(), 0);

    let retained = slot.asset().unwrap().clone();
    let ready = block_on(slot.begin().run(std::future::ready(Ok(asset(1)))));
    slot.clear();
    assert!(!slot.accept(ready));
    assert_eq!(slot.status(), SceneLoadStatus::Idle);
    assert!(slot.asset().is_none());
    assert!(slot.error().is_none());
    assert_eq!(retained.nodes()[0].name.as_deref(), Some("first"));

    let ready = block_on(slot.begin().run(std::future::ready(Ok(asset(1)))));
    assert!(slot.accept(ready));
    assert_eq!(slot.asset().unwrap().index(), 1);
    assert_eq!(slot.status(), SceneLoadStatus::Ready);
}

#[test]
fn abandoned_requests_futures_and_completions_leave_loading_state() {
    let mut slot = SceneLoadSlot::new();
    drop(slot.begin());
    assert_eq!(slot.status(), SceneLoadStatus::Cancelled);
    let (sender, receiver) = oneshot::channel::<SceneAsset>();
    let mut loading = Box::pin(slot.begin().run(async move { Ok(receiver.await?) }));
    assert!(
        loading
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    drop(loading);
    assert!(sender.is_canceled());
    assert_eq!(slot.status(), SceneLoadStatus::Cancelled);

    let unpolled = slot
        .begin()
        .run::<SceneAsset>(async { panic!("unpolled pipeline") });
    drop(unpolled);
    assert_eq!(slot.status(), SceneLoadStatus::Cancelled);
    let done = block_on(slot.begin().run(std::future::ready(Ok(asset(0)))));
    assert_eq!(slot.status(), SceneLoadStatus::Loading);
    drop(done);
    assert_eq!(slot.status(), SceneLoadStatus::Cancelled);
    assert!(slot.asset().is_none());
}

#[test]
fn superseding_and_dropping_slots_cancel_pending_resource_pipelines() {
    let document = Document::from_slice(
        &serde_json::to_vec(&json!({
            "asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],"nodes":[{}],
            "buffers":[{"byteLength":4,"uri":"mesh.bin"}]
        }))
        .unwrap(),
        Limits::default(),
    )
    .unwrap();
    for drop_slot in [false, true] {
        let mut slot = SceneLoadSlot::new();
        let (sender, receiver) = oneshot::channel();
        let mut receiver = Some(receiver);
        let pipeline = async {
            let resources = document
                .prepare_async(|_| {
                    let receiver = receiver.take().unwrap();
                    async move { Ok(receiver.await?) }
                })
                .await?;
            resources
                .scene(None, SceneOptions::default())?
                .resolve_images(|_, _| unreachable!())
        };
        let mut loading = Box::pin(slot.begin().run(pipeline));
        let mut cx = Context::from_waker(Waker::noop());
        assert!(loading.as_mut().poll(&mut cx).is_pending());
        if drop_slot {
            drop(slot);
            let Poll::Ready(done) = loading.as_mut().poll(&mut cx) else {
                panic!("cancelled task must finish")
            };
            drop(done);
        } else {
            let next = slot.begin();
            let Poll::Ready(done) = loading.as_mut().poll(&mut cx) else {
                panic!("superseded task must finish")
            };
            assert!(!slot.accept(done));
            assert_eq!(slot.status(), SceneLoadStatus::Loading);
            let done = block_on(next.run(std::future::ready(Ok(asset(1)))));
            assert!(slot.accept(done));
            assert_eq!(slot.asset().unwrap().index(), 1);
        }
        assert!(sender.is_canceled());
    }
}
