use std::{
    io::{Read, Write},
    net::Shutdown,
    os::unix::net::UnixStream,
    sync::mpsc,
    thread,
    time::Duration,
};

use gpui_wayland_source::insert_wayland_source;
use wayland_client::{Connection, Dispatch, QueueHandle, protocol::wl_callback};

#[derive(Default)]
struct State(usize);

impl Dispatch<wl_callback::WlCallback, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        _: wl_callback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        state.0 += 1;
    }
}

// A private sync-only Wayland peer. No compositor, display, or input objects.
struct Peer {
    socket: UnixStream,
    thread: Option<thread::JoinHandle<()>>,
}

impl Drop for Peer {
    fn drop(&mut self) {
        let _ = self.socket.shutdown(Shutdown::Both);
        self.thread.take().unwrap().join().unwrap();
    }
}

fn connection() -> (Connection, Peer) {
    let (client, mut server) = UnixStream::pair().unwrap();
    let socket = server.try_clone().unwrap();
    let thread = thread::spawn(move || {
        let mut request = [0; 12];
        while server.read_exact(&mut request).is_ok() {
            let words: Vec<_> = request
                .chunks_exact(4)
                .map(|word| u32::from_ne_bytes(word.try_into().unwrap()))
                .collect();
            assert_eq!(&words[..2], &[1, 12 << 16]); // wl_display.sync
            let response: Vec<_> = [words[2], 12 << 16, 0, 1, (12 << 16) | 1, words[2]]
                .into_iter()
                .flat_map(u32::to_ne_bytes)
                .collect();
            // wl_callback.done followed by wl_display.delete_id.
            if server.write_all(&response).is_err() {
                break;
            }
        }
    });
    (
        Connection::from_socket(client).unwrap(),
        Peer {
            socket,
            thread: Some(thread),
        },
    )
}

const TIMEOUT: Duration = Duration::from_millis(250);

#[test]
fn drains_events_buffered_before_registration() {
    let (connection, _peer) = connection();
    let queue = connection.new_event_queue::<State>();
    let mut reader = connection.new_event_queue::<State>();
    connection.display().sync(&queue.handle(), ());
    reader.roundtrip(&mut State::default()).unwrap();

    let mut event_loop = calloop::EventLoop::try_new().unwrap();
    insert_wayland_source(connection, queue, event_loop.handle()).unwrap();
    let mut state = State::default();
    event_loop.dispatch(Some(TIMEOUT), &mut state).unwrap();
    assert_eq!(state.0, 1);
}

#[test]
fn rearms_queue_wakeup_after_every_other_thread_read() {
    let (connection, _peer) = connection();
    let queue = connection.new_event_queue::<State>();
    let handle = queue.handle();
    let mut event_loop = calloop::EventLoop::try_new().unwrap();
    insert_wayland_source(connection.clone(), queue, event_loop.handle()).unwrap();
    let mut state = State::default();
    event_loop.dispatch(Some(TIMEOUT), &mut state).unwrap();

    let (read, requests) = mpsc::channel();
    let (finished, done) = mpsc::channel();
    let reader = thread::spawn(move || {
        let mut reader = connection.new_event_queue::<State>();
        while requests.recv().is_ok() {
            connection.display().sync(&handle, ());
            reader.roundtrip(&mut State::default()).unwrap();
            finished.send(()).unwrap();
        }
    });
    for expected in 1..=32 {
        read.send(()).unwrap();
        done.recv_timeout(Duration::from_secs(2)).unwrap();
        // The socket has already been drained by the other thread.
        event_loop.dispatch(Some(TIMEOUT), &mut state).unwrap();
        assert_eq!(state.0, expected);
    }
    drop(read);
    reader.join().unwrap();
}

#[test]
fn both_event_loops_dispatch_a_shared_connection_concurrently() {
    let (connection, _peer) = connection();
    let queue_a = connection.new_event_queue::<State>();
    let queue_b = connection.new_event_queue::<State>();
    for _ in 0..128 {
        connection.display().sync(&queue_a.handle(), ());
        connection.display().sync(&queue_b.handle(), ());
    }
    let other_connection = connection.clone();
    let other = thread::spawn(move || {
        let mut event_loop = calloop::EventLoop::try_new().unwrap();
        insert_wayland_source(other_connection, queue_b, event_loop.handle()).unwrap();
        let mut state = State::default();
        for _ in 0..20 {
            event_loop.dispatch(Some(TIMEOUT), &mut state).unwrap();
            if state.0 == 128 {
                break;
            }
        }
        state.0
    });
    let mut event_loop = calloop::EventLoop::try_new().unwrap();
    insert_wayland_source(connection, queue_a, event_loop.handle()).unwrap();
    let mut state = State::default();
    for _ in 0..20 {
        event_loop.dispatch(Some(TIMEOUT), &mut state).unwrap();
        if state.0 == 128 {
            break;
        }
    }
    assert_eq!(state.0, 128);
    assert_eq!(other.join().unwrap(), 128);
}

#[test]
fn dispatches_socket_events_without_another_reader() {
    let (connection, _peer) = connection();
    let queue = connection.new_event_queue::<State>();
    let handle = queue.handle();
    let mut event_loop = calloop::EventLoop::try_new().unwrap();
    insert_wayland_source(connection.clone(), queue, event_loop.handle()).unwrap();
    let mut state = State::default();
    event_loop.dispatch(Some(TIMEOUT), &mut state).unwrap();
    for expected in 1..=8 {
        connection.display().sync(&handle, ());
        // A leftover coalesced ping may be handled before socket readiness.
        for _ in 0..3 {
            event_loop.dispatch(Some(TIMEOUT), &mut state).unwrap();
            if state.0 == expected {
                break;
            }
        }
        assert_eq!(state.0, expected);
    }
}

#[test]
fn unfreezing_wakes_a_buffered_queue() {
    let (connection, _peer) = connection();
    let queue = connection.new_event_queue::<State>();
    let handle = queue.handle();
    let mut reader = connection.new_event_queue::<State>();
    let mut event_loop = calloop::EventLoop::try_new().unwrap();
    insert_wayland_source(connection.clone(), queue, event_loop.handle()).unwrap();
    let mut state = State::default();
    event_loop.dispatch(Some(TIMEOUT), &mut state).unwrap();
    let guard = handle.freeze();
    connection.display().sync(&handle, ());
    reader.roundtrip(&mut State::default()).unwrap();
    event_loop.dispatch(Some(TIMEOUT), &mut state).unwrap();
    assert_eq!(state.0, 0);
    drop(guard);
    event_loop.dispatch(Some(TIMEOUT), &mut state).unwrap();
    assert_eq!(state.0, 1);
}

#[test]
fn disabled_source_rearms_and_removed_source_stops_dispatching() {
    let (connection, _peer) = connection();
    let queue = connection.new_event_queue::<State>();
    let handle = queue.handle();
    let mut reader = connection.new_event_queue::<State>();
    let mut event_loop = calloop::EventLoop::try_new().unwrap();
    let token = insert_wayland_source(connection.clone(), queue, event_loop.handle()).unwrap();
    let mut state = State::default();
    event_loop.dispatch(Some(TIMEOUT), &mut state).unwrap();
    event_loop.handle().disable(&token).unwrap();
    connection.display().sync(&handle, ());
    reader.roundtrip(&mut State::default()).unwrap();
    event_loop
        .dispatch(Some(Duration::ZERO), &mut state)
        .unwrap();
    assert_eq!(state.0, 0);
    event_loop.handle().enable(&token).unwrap();
    event_loop.dispatch(Some(TIMEOUT), &mut state).unwrap();
    assert_eq!(state.0, 1);
    event_loop.handle().remove(token);
    connection.display().sync(&handle, ());
    reader.roundtrip(&mut State::default()).unwrap();
    event_loop
        .dispatch(Some(Duration::ZERO), &mut state)
        .unwrap();
    assert_eq!(state.0, 1);
}

#[test]
fn disconnect_is_reported_to_the_event_loop() {
    let (connection, peer) = connection();
    let queue = connection.new_event_queue::<State>();
    let mut event_loop = calloop::EventLoop::try_new().unwrap();
    insert_wayland_source(connection, queue, event_loop.handle()).unwrap();
    let mut state = State::default();
    event_loop.dispatch(Some(TIMEOUT), &mut state).unwrap();
    drop(peer);
    assert!(event_loop.dispatch(Some(TIMEOUT), &mut state).is_err());
}
