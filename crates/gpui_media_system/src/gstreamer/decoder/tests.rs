use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use gpui_media_core::{
    DecoderAcceleration, MediaOutputSink, MediaSource, SeekMode, VideoDecoderInfo,
};
use gst::prelude::*;

use super::DecoderTracker;
use crate::gstreamer::{GstreamerPlayback, frame_extractor::GstreamerFrameExtractionSession};
use gpui_media_core::FrameExtractionSession;

fn source() -> MediaSource {
    MediaSource::parse(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../gpui_media/examples/assets/tracks_and_subtitles.mp4"
    ))
    .unwrap()
}

fn assert_video_decoder(info: &VideoDecoderInfo) {
    let factory = gst::ElementFactory::find(&info.name).unwrap();
    let classes = factory.klass().split('/').collect::<Vec<_>>();
    assert!(classes.contains(&"Decoder"));
    assert!(classes.contains(&"Video"));
    assert_eq!(
        info.acceleration,
        if classes.contains(&"Hardware") {
            DecoderAcceleration::Hardware
        } else {
            DecoderAcceleration::Software
        }
    );
}

#[test]
fn raw_output_does_not_imply_a_software_decoder() {
    gst::init().unwrap();
    let pipeline = gst::Pipeline::new();
    let tracker = DecoderTracker::default();
    tracker.attach(pipeline.upcast_ref());
    let caps = gst::Caps::builder("video/x-raw")
        .field("format", "RGBA")
        .field("width", 1_i32)
        .field("height", 1_i32)
        .build();
    let source = gst_app::AppSrc::builder().caps(&caps).build();
    let sink = gst_app::AppSink::builder().sync(false).build();
    pipeline
        .add_many([source.upcast_ref::<gst::Element>(), sink.upcast_ref()])
        .unwrap();
    source.link(&sink).unwrap();
    pipeline.set_state(gst::State::Paused).unwrap();
    source
        .push_buffer(gst::Buffer::from_slice([0_u8; 4]))
        .unwrap();
    let sample = sink.try_pull_preroll(gst::ClockTime::from_seconds(5));
    let info = tracker.info(&sink);
    pipeline.set_state(gst::State::Null).unwrap();
    assert!(sample.is_some());
    assert!(info.is_none());
    assert!(tracker.0.lock().unwrap().is_empty());
}

#[test]
#[ignore = "requires GStreamer MP4 and H.264 decoding plugins"]
fn extraction_reports_decoder_across_seeks_and_retains_snapshot() {
    let mut session =
        GstreamerFrameExtractionSession::new(&source(), Duration::from_secs(5)).unwrap();
    let initial = session.initial_frame().unwrap();
    let info = initial.decoder_info().unwrap().clone();
    assert_video_decoder(&info);
    let later = session
        .frame_at(Duration::from_secs(1), SeekMode::Accurate)
        .unwrap();
    assert_eq!(later.decoder_info().unwrap().as_ref(), info.as_ref());
    assert!(later.timestamp() > initial.timestamp());
    drop(session);
    assert!(Arc::ptr_eq(initial.decoder_info().unwrap(), &info));
    assert_eq!(later.decoder_info().unwrap().as_ref(), info.as_ref());
}

#[test]
#[ignore = "requires GStreamer MP4 and H.264 decoding plugins"]
fn playback_preroll_and_reload_report_decoder_without_stale_observations() {
    let (sink, output) = MediaOutputSink::channel();
    let session = GstreamerPlayback::new(&source(), None, sink).unwrap();
    let audio_sink = gst::ElementFactory::make("fakesink").build().unwrap();
    session.playbin.set_property("audio-sink", audio_sink);
    let tracker = DecoderTracker::default();
    tracker.attach(&session.playbin);
    assert!(tracker.0.lock().unwrap().is_empty());
    session.pause().unwrap();
    session
        .playbin
        .state(gst::ClockTime::from_seconds(5))
        .0
        .unwrap();
    let frame = output.video_frames.try_recv().unwrap();
    assert_video_decoder(frame.decoder_info().unwrap());
    assert!(!tracker.0.lock().unwrap().is_empty());
    session.play().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let playing_frame = loop {
        if let Ok(candidate) = output.video_frames.try_recv()
            && candidate.timestamp() > frame.timestamp()
        {
            break candidate;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for playback output"
        );
        std::thread::sleep(Duration::from_millis(5));
    };
    assert_video_decoder(playing_frame.decoder_info().unwrap());
    session.playbin.set_state(gst::State::Null).unwrap();
    assert!(tracker.0.lock().unwrap().is_empty());
    while output.video_frames.try_recv().is_ok() {}
    session.reload(false).unwrap();
    session
        .playbin
        .state(gst::ClockTime::from_seconds(5))
        .0
        .unwrap();
    let reloaded = output.video_frames.try_recv().unwrap();
    assert_video_decoder(reloaded.decoder_info().unwrap());
    drop(session);
    assert!(tracker.0.lock().unwrap().is_empty());
    assert_video_decoder(frame.decoder_info().unwrap());
}
