use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll, Waker},
};

use futures::{
    channel::oneshot,
    executor::block_on,
    task::{ArcWake, waker},
};
use gpui_3d_gltf::{
    SceneLoadQueue, SceneLoadQueueFull, SceneLoadQueueStats, SceneLoadSlot, SceneLoadStatus,
};

fn poll<T>(future: Pin<&mut impl Future<Output = T>>) -> Poll<T> {
    future.poll(&mut Context::from_waker(Waker::noop()))
}

fn job(
    queue: &SceneLoadQueue,
    index: usize,
    started: Arc<Mutex<Vec<usize>>>,
    receiver: oneshot::Receiver<usize>,
) -> impl Future<Output = anyhow::Result<usize>> + '_ {
    queue.run(move || {
        started.lock().unwrap().push(index);
        async move { Ok(receiver.await?) }
    })
}

#[test]
fn bounded_admission_is_lazy_and_fifo_across_clones() {
    let queue = SceneLoadQueue::new(2.try_into().unwrap(), 2);
    let clone = queue.clone();
    let started = Arc::new(Mutex::new(Vec::new()));
    let mut senders = Vec::new();
    let mut jobs = Vec::new();
    for index in 0..4 {
        let (sender, receiver) = oneshot::channel();
        senders.push(sender);
        jobs.push(Box::pin(job(&clone, index, started.clone(), receiver)));
    }
    assert_eq!(
        queue.stats(),
        SceneLoadQueueStats {
            active: 0,
            waiting: 0
        }
    );
    for task in &mut jobs {
        assert!(poll(task.as_mut()).is_pending());
    }
    assert_eq!(*started.lock().unwrap(), [0, 1]);
    assert_eq!(
        queue.stats(),
        SceneLoadQueueStats {
            active: 2,
            waiting: 2
        }
    );
    let error =
        block_on(queue.run(|| -> std::future::Ready<anyhow::Result<()>> {
            panic!("overflow must not start")
        }))
        .unwrap_err();
    assert!(error.is::<SceneLoadQueueFull>());

    senders.remove(0).send(10).unwrap();
    assert_eq!(block_on(jobs.remove(0)).unwrap(), 10);
    assert!(poll(jobs[2].as_mut()).is_pending());
    assert_eq!(*started.lock().unwrap(), [0, 1]);
    assert!(poll(jobs[1].as_mut()).is_pending());
    assert_eq!(*started.lock().unwrap(), [0, 1, 2]);
    drop(jobs.remove(0));
    assert!(senders[0].is_canceled());
    assert!(poll(jobs[1].as_mut()).is_pending());
    assert_eq!(*started.lock().unwrap(), [0, 1, 2, 3]);
    drop(jobs);
    assert_eq!(
        queue.stats(),
        SceneLoadQueueStats {
            active: 0,
            waiting: 0
        }
    );
}

#[test]
fn dropping_waiters_and_unobserved_grants_preserves_capacity() {
    let queue = SceneLoadQueue::new(1.try_into().unwrap(), 2);
    let started = Arc::new(Mutex::new(Vec::new()));
    let (active_sender, active_receiver) = oneshot::channel();
    let mut active = Box::pin(job(&queue, 0, started.clone(), active_receiver));
    assert!(poll(active.as_mut()).is_pending());
    let (removed_sender, removed_receiver) = oneshot::channel();
    let mut removed = Box::pin(job(&queue, 1, started.clone(), removed_receiver));
    assert!(poll(removed.as_mut()).is_pending());
    drop(removed);
    assert!(removed_sender.is_canceled());
    assert_eq!(queue.stats().waiting, 0);

    let (granted_sender, granted_receiver) = oneshot::channel();
    let mut granted = Box::pin(job(&queue, 2, started.clone(), granted_receiver));
    let (last_sender, last_receiver) = oneshot::channel();
    let mut last = Box::pin(job(&queue, 3, started.clone(), last_receiver));
    assert!(poll(granted.as_mut()).is_pending());
    assert!(poll(last.as_mut()).is_pending());
    drop(active);
    assert!(active_sender.is_canceled());
    assert_eq!(
        queue.stats(),
        SceneLoadQueueStats {
            active: 1,
            waiting: 1
        }
    );
    drop(granted);
    assert!(granted_sender.is_canceled());
    assert!(poll(last.as_mut()).is_pending());
    assert_eq!(*started.lock().unwrap(), [0, 3]);
    last_sender.send(7).unwrap();
    assert_eq!(block_on(last).unwrap(), 7);
    assert_eq!(
        queue.stats(),
        SceneLoadQueueStats {
            active: 0,
            waiting: 0
        }
    );
}

#[test]
fn slot_cancellation_releases_waiting_and_running_loads() {
    let queue = SceneLoadQueue::new(1.try_into().unwrap(), 1);
    let mut blocker = Box::pin(queue.run(|| std::future::pending::<anyhow::Result<()>>()));
    assert!(poll(blocker.as_mut()).is_pending());
    let mut slot = SceneLoadSlot::new();
    let mut queued = Box::pin(slot.begin().run(queue.run(
        || -> std::future::Ready<anyhow::Result<()>> { panic!("cancelled waiter must not start") },
    )));
    assert!(poll(queued.as_mut()).is_pending());
    slot.cancel();
    assert!(!slot.accept_with(block_on(queued), |_| unreachable!()));
    assert_eq!(queue.stats().waiting, 0);
    assert_eq!(slot.status(), SceneLoadStatus::Cancelled);
    drop(blocker);

    let (sender, receiver) = oneshot::channel::<()>();
    let mut running = Box::pin(
        slot.begin()
            .run(queue.run(|| async move { Ok(receiver.await?) })),
    );
    assert!(poll(running.as_mut()).is_pending());
    let newer = slot.begin();
    assert!(!slot.accept_with(block_on(running), |_| unreachable!()));
    assert!(sender.is_canceled());
    assert_eq!(queue.stats().active, 0);
    let ready = block_on(newer.run(queue.run(|| async { Ok(()) })));
    assert!(slot.accept_with(ready, |_| anyhow::bail!("resolution failed")));
    assert_eq!(slot.status(), SceneLoadStatus::Failed);
}

#[test]
fn immediate_only_admission_recovers_after_errors_and_unwinding() {
    let queue = SceneLoadQueue::new(1.try_into().unwrap(), 0);
    let mut first = Box::pin(queue.run(|| std::future::pending::<anyhow::Result<()>>()));
    assert!(poll(first.as_mut()).is_pending());
    let mut slot = SceneLoadSlot::new();
    let full = block_on(slot.begin().run(queue.run(|| async { Ok(()) })));
    assert!(slot.accept_with(full, |_| unreachable!()));
    assert!(slot.error().unwrap().is::<SceneLoadQueueFull>());
    assert_eq!(slot.status(), SceneLoadStatus::Failed);
    drop(first);
    assert!(
        block_on(queue.run(|| async { Err::<(), _>(anyhow::anyhow!("read failed")) })).is_err()
    );
    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        block_on(
            queue
                .run(|| -> std::future::Ready<anyhow::Result<()>> { panic!("constructor failed") }),
        )
    }));
    assert!(unwound.is_err());
    assert_eq!(block_on(queue.run(|| async { Ok(42) })).unwrap(), 42);
    assert_eq!(
        queue.stats(),
        SceneLoadQueueStats {
            active: 0,
            waiting: 0
        }
    );
}

#[derive(Default)]
struct WakeCount(AtomicUsize);

impl ArcWake for WakeCount {
    fn wake_by_ref(arc_self: &Arc<Self>) {
        arc_self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn worker_release_wakes_the_next_caller_owned_task() {
    let queue = SceneLoadQueue::new(1.try_into().unwrap(), 1);
    let (started_sender, started_receiver) = std::sync::mpsc::channel();
    let (finish_sender, finish_receiver) = oneshot::channel::<()>();
    let worker_queue = queue.clone();
    let worker = std::thread::spawn(move || {
        block_on(worker_queue.run(|| async move {
            started_sender.send(()).unwrap();
            Ok(finish_receiver.await?)
        }))
    });
    started_receiver.recv().unwrap();
    let wakes = Arc::new(WakeCount::default());
    let waker = waker(wakes.clone());
    let mut next = Box::pin(queue.run(|| async { Ok(9) }));
    assert!(
        next.as_mut()
            .poll(&mut Context::from_waker(&waker))
            .is_pending()
    );
    finish_sender.send(()).unwrap();
    worker.join().unwrap().unwrap();
    assert!(wakes.0.load(Ordering::SeqCst) > 0);
    assert_eq!(block_on(next).unwrap(), 9);
    assert_eq!(
        queue.stats(),
        SceneLoadQueueStats {
            active: 0,
            waiting: 0
        }
    );
}
