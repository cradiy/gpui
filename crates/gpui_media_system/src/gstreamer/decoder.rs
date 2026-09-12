use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use gpui_media_core::{DecoderAcceleration, DecoderDeviceProperty, VideoDecoderInfo, VideoFrame};
use gst::prelude::*;

pub(super) mod selection;

struct Observation {
    pad: gst::glib::WeakRef<gst::Pad>,
    stream_id: gst::glib::GString,
    info: Arc<VideoDecoderInfo>,
}

#[derive(Clone, Default)]
pub(super) struct DecoderTracker(Arc<Mutex<Vec<Observation>>>);

impl DecoderTracker {
    pub(super) fn attach(&self, playbin: &gst::Element) {
        let Some(bin) = playbin.downcast_ref::<gst::Bin>() else {
            return;
        };
        let tracker = self.clone();
        bin.connect_deep_element_added(move |_, _, element| {
            let Some(factory) = element.factory() else {
                return;
            };
            let classes = factory.klass().split('/').collect::<Vec<_>>();
            if !classes.contains(&"Decoder")
                || !classes.contains(&"Video")
                || element.is::<gst::Bin>()
            {
                return;
            }
            for pad in element.src_pads() {
                tracker.observe_pad(element, &pad);
            }
            let tracker = tracker.clone();
            element.connect_pad_added(move |element, pad| {
                if pad.direction() == gst::PadDirection::Src {
                    tracker.observe_pad(element, pad);
                }
            });
        });
        let tracker = self.clone();
        bin.connect_deep_element_removed(move |_, _, element| {
            tracker.0.lock().unwrap().retain(|entry| {
                entry
                    .pad
                    .upgrade()
                    .and_then(|pad| pad.parent_element())
                    .is_some_and(|parent| parent != *element)
            });
        });
    }

    fn observe_pad(&self, element: &gst::Element, pad: &gst::Pad) {
        let element = element.downgrade();
        let tracker = self.clone();
        let pending = AtomicBool::new(true);
        pad.add_probe(
            gst::PadProbeType::BUFFER
                | gst::PadProbeType::BUFFER_LIST
                | gst::PadProbeType::EVENT_DOWNSTREAM,
            move |pad, probe| {
                if let Some(event) = probe.event() {
                    if matches!(
                        event.view(),
                        gst::EventView::StreamStart(_) | gst::EventView::Caps(_)
                    ) {
                        pending.store(true, Ordering::Release);
                        tracker.remove_pad(pad);
                    }
                } else if pending.swap(false, Ordering::AcqRel) {
                    let caps = pad.current_caps();
                    if caps
                        .as_ref()
                        .and_then(|caps| caps.structure(0))
                        .is_some_and(|structure| structure.name() == "video/x-raw")
                        && let Some(stream_id) = pad.stream_id()
                        && let Some(element) = element.upgrade()
                        && let Some(info) = decoder_info(&element)
                    {
                        let mut entries = tracker.0.lock().unwrap();
                        entries
                            .retain(|entry| entry.pad.upgrade().is_some_and(|other| other != *pad));
                        entries.push(Observation {
                            pad: pad.downgrade(),
                            stream_id,
                            info: Arc::new(info),
                        });
                    }
                }
                gst::PadProbeReturn::Ok
            },
        );
    }

    fn remove_pad(&self, pad: &gst::Pad) {
        self.0
            .lock()
            .unwrap()
            .retain(|entry| entry.pad.upgrade().is_some_and(|other| other != *pad));
    }

    pub(super) fn info(&self, sink: &gst_app::AppSink) -> Option<Arc<VideoDecoderInfo>> {
        let stream_id = sink.static_pad("sink")?.stream_id()?;
        let entries = self.0.lock().unwrap();
        let mut matching = entries
            .iter()
            .filter(|entry| entry.stream_id == stream_id && entry.pad.upgrade().is_some());
        let info = matching.next()?.info.clone();
        matching.next().is_none().then_some(info)
    }

    pub(super) fn annotate(&self, frame: VideoFrame, sink: &gst_app::AppSink) -> Arc<VideoFrame> {
        Arc::new(match self.info(sink) {
            Some(info) => frame.with_decoder_info(info),
            None => frame,
        })
    }
}

#[cfg(test)]
mod tests;

fn decoder_info(element: &gst::Element) -> Option<VideoDecoderInfo> {
    let factory = element.factory()?;
    let acceleration = if factory.klass().split('/').any(|class| class == "Hardware") {
        DecoderAcceleration::Hardware
    } else {
        DecoderAcceleration::Software
    };
    let device_properties = ["device-path", "device-id", "cuda-device-id", "adapter-luid"]
        .into_iter()
        .filter_map(|name| {
            let property = element.find_property(name)?;
            if !property.flags().contains(gst::glib::ParamFlags::READABLE) {
                return None;
            }
            let value = element.property_value(name);
            let value = if let Ok(value) = value.get::<String>() {
                value
            } else if let Ok(value) = value.get::<u32>() {
                value.to_string()
            } else if let Ok(value) = value.get::<i32>() {
                value.to_string()
            } else if let Ok(value) = value.get::<i64>() {
                value.to_string()
            } else if let Ok(value) = value.get::<u64>() {
                value.to_string()
            } else {
                return None;
            };
            (!value.is_empty()).then(|| DecoderDeviceProperty {
                name: name.into(),
                value: value.into(),
            })
        })
        .collect();
    Some(VideoDecoderInfo {
        name: factory.name().as_str().into(),
        acceleration,
        device_properties,
    })
}
