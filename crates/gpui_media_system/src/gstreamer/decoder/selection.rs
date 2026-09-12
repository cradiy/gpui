use gpui_media_core::{
    DecoderAcceleration, MediaError, MediaResult, VideoDecoderPolicy, VideoFrame,
};
use gst::prelude::*;

pub(crate) fn configure(playbin: &gst::Element, policy: VideoDecoderPolicy) {
    playbin.connect("element-setup", false, move |values| {
        let element = values[1]
            .get::<gst::Element>()
            .expect("element-setup argument");
        if element
            .factory()
            .is_some_and(|factory| factory.name() == "uridecodebin")
        {
            configure_decoder(&element, policy);
        }
        None
    });
}

fn configure_decoder(decoder: &gst::Element, policy: VideoDecoderPolicy) {
    let signal = gst::glib::subclass::SignalId::lookup("autoplug-select", decoder.type_())
        .expect("uridecodebin autoplug-select signal");
    let results = gst::glib::EnumClass::with_type(signal.query().return_type().type_())
        .expect("autoplug-select result enum");
    decoder.connect("autoplug-select", false, move |values| {
        let factory = values[3]
            .get::<gst::ElementFactory>()
            .expect("autoplug-select factory");
        let classes = factory.klass().split('/').collect::<Vec<_>>();
        let allowed = !classes.contains(&"Decoder")
            || !classes.contains(&"Video")
            || match policy {
                VideoDecoderPolicy::Auto => true,
                VideoDecoderPolicy::SoftwareOnly => !classes.contains(&"Hardware"),
                VideoDecoderPolicy::HardwareOnly => classes.contains(&"Hardware"),
            };
        results.to_value_by_nick(if allowed { "try" } else { "skip" })
    });
}

pub(crate) fn validate(frame: &VideoFrame, policy: VideoDecoderPolicy) -> MediaResult<()> {
    let required = match policy {
        VideoDecoderPolicy::Auto => return Ok(()),
        VideoDecoderPolicy::SoftwareOnly => DecoderAcceleration::Software,
        VideoDecoderPolicy::HardwareOnly => DecoderAcceleration::Hardware,
    };
    if frame
        .decoder_info()
        .is_some_and(|info| info.acceleration == required)
    {
        Ok(())
    } else {
        Err(MediaError::unsupported(format!(
            "video output does not identify a decoder satisfying {policy:?}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn autoplug_policies_are_per_decoder_and_leave_registry_ranks_unchanged() {
        gst::init().unwrap();
        let factories = gst::ElementFactory::factories_with_type(
            gst::ElementFactoryType::DECODER,
            gst::Rank::NONE,
        );
        assert!(!factories.is_empty());
        let ranks = factories
            .iter()
            .map(|factory| factory.rank())
            .collect::<Vec<_>>();
        let software = gst::ElementFactory::make("uridecodebin").build().unwrap();
        let hardware = gst::ElementFactory::make("uridecodebin").build().unwrap();
        let automatic = gst::ElementFactory::make("uridecodebin").build().unwrap();
        configure_decoder(&software, VideoDecoderPolicy::SoftwareOnly);
        configure_decoder(&hardware, VideoDecoderPolicy::HardwareOnly);
        let pad = gst::Pad::builder(gst::PadDirection::Src).build();
        let caps = gst::Caps::new_any();
        for (factory, rank) in factories.iter().zip(ranks) {
            let classes = factory.klass().split('/').collect::<Vec<_>>();
            for (decoder, skip) in [
                (
                    &software,
                    classes.contains(&"Video") && classes.contains(&"Hardware"),
                ),
                (
                    &hardware,
                    classes.contains(&"Video") && !classes.contains(&"Hardware"),
                ),
                (&automatic, false),
            ] {
                let result = decoder
                    .emit_by_name_with_values(
                        "autoplug-select",
                        &[pad.to_value(), caps.to_value(), factory.to_value()],
                    )
                    .unwrap();
                let (_, result) = gst::glib::EnumValue::from_value(&result).unwrap();
                assert_eq!(
                    result.nick(),
                    if skip { "skip" } else { "try" },
                    "{}",
                    factory.name()
                );
            }
            assert_eq!(factory.rank(), rank);
        }
    }
}
