use std::sync::Arc;

/// Restricts which video decoder implementations an extraction session may use.
///
/// Explicit policies exclude decoders outside the requested acceleration class.
/// A backend that cannot enforce a policy must return an error.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VideoDecoderPolicy {
    #[default]
    Auto,
    SoftwareOnly,
    HardwareOnly,
}

/// The acceleration type reported by the decoder implementation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecoderAcceleration {
    Software,
    Hardware,
    Unknown,
}

/// A backend-specific device property exposed by an active decoder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecoderDeviceProperty {
    pub name: Arc<str>,
    pub value: Arc<str>,
}

/// Decoder metadata supplied by a backend after video output begins.
///
/// Acceleration describes the decoder, independently of frame storage. Device
/// properties are optional and use backend-specific names and values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VideoDecoderInfo {
    pub name: Arc<str>,
    pub acceleration: DecoderAcceleration,
    pub device_properties: Vec<DecoderDeviceProperty>,
}
