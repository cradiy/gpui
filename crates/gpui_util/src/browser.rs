//! Thread-bound browser resources with transferable ownership handles.

use std::{
    any::Any,
    cell::RefCell,
    collections::HashMap,
    marker::PhantomData,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

thread_local! {
    static RESOURCES: RefCell<HashMap<u64, Rc<dyn Any>>> = RefCell::new(HashMap::new());
    static RELEASE: async_channel::Sender<u64> = {
        let (sender, receiver) = async_channel::unbounded();
        wasm_bindgen_futures::spawn_local(async move {
            while let Ok(id) = receiver.recv().await {
                // Drop outside the registry borrow: resources may own other handles.
                let resource = RESOURCES.with(|resources| resources.borrow_mut().remove(&id));
                drop(resource);
            }
        });
        sender
    };
}

struct Lease {
    id: u64,
    release: async_channel::Sender<u64>,
}

impl Drop for Lease {
    fn drop(&mut self) {
        let _ = self.release.try_send(self.id);
    }
}

/// Shared ownership of a resource that stays on its creating browser thread.
///
/// Access from another thread returns `None`. Releasing the final handle on any
/// thread schedules destruction on the owner thread; no JS value crosses threads.
pub struct BrowserResource<T: 'static> {
    lease: Arc<Lease>,
    marker: PhantomData<fn() -> T>,
}

impl<T> Clone for BrowserResource<T> {
    fn clone(&self) -> Self {
        Self {
            lease: self.lease.clone(),
            marker: PhantomData,
        }
    }
}

impl<T> std::fmt::Debug for BrowserResource<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("BrowserResource")
            .field(&self.lease.id)
            .finish()
    }
}

impl<T> BrowserResource<T> {
    pub fn new(value: T) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        RESOURCES.with(|resources| resources.borrow_mut().insert(id, Rc::new(value)));
        Self {
            lease: Arc::new(Lease {
                id,
                release: RELEASE.with(Clone::clone),
            }),
            marker: PhantomData,
        }
    }

    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> Option<R> {
        let resource =
            RESOURCES.with(|resources| resources.borrow().get(&self.lease.id).cloned())?;
        Some(f(resource.downcast_ref::<T>()?))
    }
}

/// An immutable browser video frame, closed when its last owner releases it.
#[derive(Clone, Debug)]
pub struct BrowserVideoFrame {
    resource: BrowserResource<OwnedVideoFrame>,
    width: u32,
    height: u32,
}

struct OwnedVideoFrame(web_sys::VideoFrame);
impl Drop for OwnedVideoFrame {
    fn drop(&mut self) {
        self.0.close();
    }
}

impl BrowserVideoFrame {
    /// Takes an independent reference to the supplied frame.
    pub fn new(frame: &web_sys::VideoFrame) -> Result<Self, wasm_bindgen::JsValue> {
        let owned = web_sys::VideoFrame::clone(frame)?;
        Ok(Self {
            width: owned.display_width(),
            height: owned.display_height(),
            resource: BrowserResource::new(OwnedVideoFrame(owned)),
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Runs on the frame's owner thread, or returns `None` on another thread.
    pub fn with<R>(&self, f: impl FnOnce(&web_sys::VideoFrame) -> R) -> Option<R> {
        self.resource.with(|frame| f(&frame.0))
    }
}
