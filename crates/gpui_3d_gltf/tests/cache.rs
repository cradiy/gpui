use std::{
    cell::Cell,
    future::Future,
    sync::Arc,
    task::{Context, Poll, Waker},
};

use futures::{channel::oneshot, executor::block_on};
use gpui_3d_gltf::{Document, Limits, ResourceCache, ResourceCacheLimits};
use serde_json::json;

#[test]
fn lru_limits_release_cache_references_without_revoking_consumers() {
    let cache = ResourceCache::new(ResourceCacheLimits {
        bytes: 6,
        entries: 2,
    });
    let a = block_on(cache.load("a", 3, || async { Ok(vec![1; 3]) })).unwrap();
    let b = block_on(cache.load("b", 3, || async { Ok(vec![2; 3]) })).unwrap();
    let weak = Arc::downgrade(&b);
    assert!(Arc::ptr_eq(&a, &cache.get(&"a").unwrap()));
    let c = block_on(cache.load("c", 3, || async { Ok(vec![3; 3]) })).unwrap();
    assert!(cache.get(&"b").is_none());
    assert_eq!(&*b, [2; 3]);
    drop(b);
    assert!(weak.upgrade().is_none());
    assert_eq!(cache.cached_bytes(), 6);
    assert_eq!(cache.len(), 2);
    let oversized = block_on(cache.load("large", 7, || async { Ok(vec![4; 7]) })).unwrap();
    assert_eq!(oversized.len(), 7);
    assert_eq!(cache.cached_bytes(), 6);
    assert!(cache.get(&"large").is_none());
    cache.set_limits(ResourceCacheLimits {
        bytes: 6,
        entries: 1,
    });
    assert!(cache.get(&"a").is_none());
    assert!(Arc::ptr_eq(&c, &cache.get(&"c").unwrap()));
    assert_eq!(cache.cached_bytes(), 3);
    cache.set_limits(ResourceCacheLimits {
        bytes: 0,
        entries: 8,
    });
    assert!(cache.is_empty());
    cache.set_limits(ResourceCacheLimits {
        bytes: 5,
        entries: 4,
    });
    block_on(cache.load("x", 3, || async { Ok(vec![0; 3]) })).unwrap();
    block_on(cache.load("y", 2, || async { Ok(vec![0; 2]) })).unwrap();
    block_on(cache.load("z", 2, || async { Ok(vec![0; 2]) })).unwrap();
    assert!(cache.get(&"x").is_none());
    assert_eq!(cache.len(), 2);
    assert_eq!(cache.cached_bytes(), 4);
    cache.set_limits(ResourceCacheLimits {
        bytes: 1,
        entries: 4,
    });
    assert!(cache.is_empty());
    assert_eq!(cache.cached_bytes(), 0);
    assert_eq!(&*a, [1; 3]);
    cache.set_limits(ResourceCacheLimits {
        bytes: 0,
        entries: 8,
    });
    let empty = block_on(cache.load("empty", 0, || async { Ok(Vec::new()) })).unwrap();
    assert!(empty.is_empty());
    assert!(cache.is_empty());
    cache.set_limits(ResourceCacheLimits {
        bytes: 8,
        entries: 0,
    });
    block_on(cache.load("a", 3, || async { Ok(vec![1; 3]) })).unwrap();
    assert!(cache.is_empty());
}

#[test]
fn shared_preparation_reuses_storage_but_keeps_document_budgets_and_namespaces() {
    let bytes = serde_json::to_vec(&json!({"asset":{"version":"2.0"},
        "buffers":[{"byteLength":2,"uri":"part.bin"}],"images":[{"uri":"part.bin"}]}))
    .unwrap();
    let document = Document::from_slice(
        &bytes,
        Limits {
            resource_bytes: 4,
            ..Limits::default()
        },
    )
    .unwrap();
    let cache = ResourceCache::new(ResourceCacheLimits {
        bytes: 16,
        entries: 4,
    });
    let calls = Cell::new(0);
    let load = |namespace| {
        block_on(document.prepare_shared_async(|request| {
            cache.load((namespace, request.uri), request.byte_limit, || {
                calls.set(calls.get() + 1);
                async { Ok(vec![1, 2, 3, 4]) }
            })
        }))
        .unwrap()
    };
    let first = load("one");
    let second = load("one");
    assert_eq!(calls.get(), 1);
    assert_eq!(first.resource_bytes(), 4);
    assert_eq!(second.resource_bytes(), 4);
    assert_eq!(
        first.buffer(0).unwrap().as_ptr(),
        second.buffer(0).unwrap().as_ptr()
    );
    assert_eq!(
        first.image(0).unwrap().bytes().as_ptr(),
        second.image(0).unwrap().bytes().as_ptr()
    );
    let other = load("two");
    assert_eq!(calls.get(), 2);
    assert_ne!(
        first.buffer(0).unwrap().as_ptr(),
        other.buffer(0).unwrap().as_ptr()
    );
    let restricted = Document::from_slice(
        &bytes,
        Limits {
            resource_bytes: 3,
            ..Limits::default()
        },
    )
    .unwrap();
    let failed = block_on(restricted.prepare_shared_async(|request| {
        cache.load(("one", request.uri), request.byte_limit, || async {
            panic!("cache hit must not load")
        })
    }));
    assert!(failed.is_err());
    cache.clear();
    assert!(cache.is_empty());
    assert_eq!(cache.cached_bytes(), 0);
    assert_eq!(first.image(0).unwrap().bytes(), [1, 2, 3, 4]);
}

#[test]
fn invalidation_prevents_pending_loads_from_repopulating_cache() {
    let cache = ResourceCache::new(ResourceCacheLimits {
        bytes: 8,
        entries: 4,
    });
    for operation in 0..3 {
        let (sender, receiver) = oneshot::channel();
        let mut old = Box::pin(cache.load("key", 4, || async { Ok(receiver.await?) }));
        let mut cx = Context::from_waker(Waker::noop());
        assert!(old.as_mut().poll(&mut cx).is_pending());
        match operation {
            0 => {
                assert!(!cache.invalidate(&"key"));
            }
            1 => cache.clear(),
            _ => cache.set_limits(ResourceCacheLimits {
                bytes: 9,
                entries: 4,
            }),
        }
        sender.send(vec![1; 4]).unwrap();
        let Poll::Ready(Ok(bytes)) = old.as_mut().poll(&mut cx) else {
            panic!("load must complete")
        };
        assert_eq!(&*bytes, [1; 4]);
        assert!(cache.is_empty());
    }
    let (sender, receiver) = oneshot::channel();
    let mut old = Box::pin(cache.load("key", 4, || async { Ok(receiver.await?) }));
    let mut cx = Context::from_waker(Waker::noop());
    assert!(old.as_mut().poll(&mut cx).is_pending());
    cache.invalidate(&"key");
    let fresh = block_on(cache.load("key", 4, || async { Ok(vec![2; 4]) })).unwrap();
    sender.send(vec![1; 4]).unwrap();
    assert!(old.as_mut().poll(&mut cx).is_ready());
    assert!(Arc::ptr_eq(&fresh, &cache.get(&"key").unwrap()));
    assert!(cache.invalidate(&"key"));
    assert!(cache.get(&"key").is_none());
}

#[test]
fn failed_oversized_and_dropped_loads_are_retryable() {
    let cache = ResourceCache::new(ResourceCacheLimits {
        bytes: 8,
        entries: 4,
    });
    assert!(block_on(cache.load("key", 4, || async { anyhow::bail!("unavailable") })).is_err());
    assert!(block_on(cache.load("key", 4, || async { Ok(vec![0; 5]) })).is_err());
    assert!(cache.is_empty());
    let (sender, receiver) = oneshot::channel::<Vec<u8>>();
    let mut loading = Box::pin(cache.load("key", 4, || async { Ok(receiver.await?) }));
    assert!(
        loading
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    drop(loading);
    assert!(sender.is_canceled());
    assert!(cache.is_empty());
    let shared = cache.clone();
    let loaded = std::thread::spawn(move || {
        block_on(shared.load("key", 4, || async { Ok(vec![3; 4]) })).unwrap()
    })
    .join()
    .unwrap();
    let hit = block_on(cache.load("key", 4, || async { panic!("cache hit") })).unwrap();
    assert!(Arc::ptr_eq(&loaded, &hit));
    assert!(block_on(cache.load("key", 3, || async { panic!("oversized cache hit") })).is_err());
    assert_eq!(cache.cached_bytes(), 4);
}

#[test]
fn concurrent_misses_do_not_overwrite_a_completed_insertion() {
    let cache = ResourceCache::new(ResourceCacheLimits {
        bytes: 8,
        entries: 4,
    });
    let (first_sender, first_receiver) = oneshot::channel();
    let (second_sender, second_receiver) = oneshot::channel();
    let mut first = Box::pin(cache.load("key", 4, || async { Ok(first_receiver.await?) }));
    let mut second = Box::pin(cache.load("key", 4, || async { Ok(second_receiver.await?) }));
    let mut cx = Context::from_waker(Waker::noop());
    assert!(first.as_mut().poll(&mut cx).is_pending());
    assert!(second.as_mut().poll(&mut cx).is_pending());
    second_sender.send(vec![2; 4]).unwrap();
    let Poll::Ready(Ok(retained)) = second.as_mut().poll(&mut cx) else {
        panic!("second request must complete")
    };
    first_sender.send(vec![1; 4]).unwrap();
    let Poll::Ready(Ok(late)) = first.as_mut().poll(&mut cx) else {
        panic!("first request must complete")
    };
    assert_eq!(&*late, [1; 4]);
    assert!(Arc::ptr_eq(&retained, &cache.get(&"key").unwrap()));
    assert_eq!(cache.len(), 1);
    assert_eq!(cache.cached_bytes(), 4);
}
