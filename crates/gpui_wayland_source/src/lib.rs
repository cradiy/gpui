//! Wayland queue dispatch with cross-thread wakeups for shared connections.

use std::{
    io,
    sync::Arc,
    task::{Context, Poll as TaskPoll, Wake, Waker},
};

use calloop::{
    EventIterator, EventSource, Interest, LoopHandle, Mode, Poll, PostAction, Readiness,
    RegistrationToken, Token, TokenFactory,
    generic::Generic,
    ping::{Ping, PingSource, make_ping},
};
use wayland_client::{
    Connection, DispatchError, EventQueue,
    backend::{ReadEventsGuard, WaylandError},
};

/// Register a queue, including events read into it by another connection user.
/// The queue must belong to `connection`. Removing the returned registration
/// removes both socket and queue-wakeup interests.
pub fn insert_wayland_source<D: 'static>(
    connection: Connection,
    queue: EventQueue<D>,
    handle: LoopHandle<'_, D>,
) -> calloop::Result<RegistrationToken> {
    let (ping, wake_source) = make_ping()?;
    let waker = Waker::from(Arc::new(QueueWake(ping)));
    let source = WaylandSource {
        queue,
        socket: Generic::new(connection, Interest::READ, Mode::Level),
        wake_source,
        waker: waker.clone(),
        read_guard: None,
        dispatch_token: None,
        read_error: None,
    };
    handle
        .insert_source(source, move |(), queue, data| {
            // This atomically drains the Rust queue and arms its next wakeup.
            // Socket readability alone is insufficient when another thread reads
            // the shared connection and buffers this queue's events.
            match queue.poll_dispatch_pending(&mut Context::from_waker(&waker), data) {
                TaskPoll::Pending => Ok(()),
                TaskPoll::Ready(Err(error)) => Err(error),
                TaskPoll::Ready(Ok(never)) => match never {},
            }
        })
        .map_err(|error| error.error)
}

struct QueueWake(Ping);

impl Wake for QueueWake {
    fn wake(self: Arc<Self>) {
        self.0.ping();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.0.ping();
    }
}

struct WaylandSource<D> {
    queue: EventQueue<D>,
    socket: Generic<Connection>,
    wake_source: PingSource,
    waker: Waker,
    read_guard: Option<ReadEventsGuard>,
    dispatch_token: Option<Token>,
    read_error: Option<WaylandError>,
}

fn dispatch_error(error: impl std::error::Error + Send + Sync + 'static) -> calloop::Error {
    calloop::Error::OtherError(Box::new(error))
}

impl<D> WaylandSource<D> {
    fn flush(&self) -> calloop::Result<()> {
        match self.queue.flush() {
            Ok(()) => Ok(()),
            Err(WaylandError::Io(error)) if error.kind() == io::ErrorKind::WouldBlock => Ok(()),
            Err(error) => Err(dispatch_error(error)),
        }
    }
}

impl<D> EventSource for WaylandSource<D> {
    type Event = ();
    type Metadata = EventQueue<D>;
    type Ret = Result<(), DispatchError>;
    type Error = calloop::Error;
    const NEEDS_EXTRA_LIFECYCLE_EVENTS: bool = true;

    fn process_events<F>(
        &mut self,
        readiness: Readiness,
        token: Token,
        mut callback: F,
    ) -> calloop::Result<PostAction>
    where
        F: FnMut((), &mut EventQueue<D>) -> Self::Ret,
    {
        debug_assert!(self.read_guard.is_none());
        if let Some(error) = self.read_error.take() {
            return Err(dispatch_error(error));
        }
        self.wake_source
            .process_events(readiness, token, |(), _| {})
            .map_err(dispatch_error)?;
        callback((), &mut self.queue).map_err(dispatch_error)?;
        self.flush()?;
        Ok(PostAction::Continue)
    }

    fn register(&mut self, poll: &mut Poll, tokens: &mut TokenFactory) -> calloop::Result<()> {
        self.dispatch_token = Some(tokens.token());
        self.socket.register(poll, tokens)?;
        if let Err(error) = self.wake_source.register(poll, tokens) {
            let _ = self.socket.unregister(poll);
            return Err(error);
        }
        // Drain events buffered before insertion and install the first waker.
        self.waker.wake_by_ref();
        Ok(())
    }

    fn reregister(&mut self, poll: &mut Poll, tokens: &mut TokenFactory) -> calloop::Result<()> {
        self.dispatch_token = Some(tokens.token());
        self.socket.reregister(poll, tokens)?;
        self.wake_source.reregister(poll, tokens)?;
        self.waker.wake_by_ref();
        Ok(())
    }

    fn unregister(&mut self, poll: &mut Poll) -> calloop::Result<()> {
        self.read_guard = None;
        let socket = self.socket.unregister(poll);
        let wake = self.wake_source.unregister(poll);
        socket.and(wake)
    }

    fn before_sleep(&mut self) -> calloop::Result<Option<(Readiness, Token)>> {
        debug_assert!(self.read_guard.is_none());
        self.flush()?;
        self.read_guard = self.queue.prepare_read();
        Ok(self
            .read_guard
            .is_none()
            .then(|| (Readiness::EMPTY, self.dispatch_token.unwrap())))
    }

    fn before_handle_events(&mut self, events: EventIterator<'_>) {
        let mut socket_ready = false;
        for (readiness, token) in events {
            // Generic filters its own token. A queue ping must only cancel the
            // read guard: reading on a ping can wait on another sleeping reader.
            let _ = self.socket.process_events(readiness, token, |_, _| {
                socket_ready = true;
                Ok(PostAction::Continue)
            });
        }
        if let Some(guard) = self.read_guard.take() {
            if socket_ready {
                match guard.read() {
                    Ok(_) => {}
                    Err(WaylandError::Io(error)) if error.kind() == io::ErrorKind::WouldBlock => {}
                    Err(error) => self.read_error = Some(error),
                }
            }
            // Otherwise drop the guard before any source callback is invoked.
        }
    }
}
