use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    },
    time::Duration,
};

use cpal::{
    FromSample, Sample, SampleFormat, SizedSample, Stream, StreamConfig, StreamError,
    SupportedStreamConfig,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};

use crate::{MediaError, MediaErrorKind, MediaRecovery, MediaResult};

pub(super) struct PcmOutput {
    stream: Stream,
    shared: Arc<AudioQueue>,
    source_channels: u16,
    source_sample_rate: u32,
    device_channels: u16,
    device_sample_rate: u32,
    resampler: InterleavedResampler,
}

#[derive(Clone)]
pub(super) struct PcmOutputControl {
    shared: Arc<AudioQueue>,
    device_channels: u16,
    device_sample_rate: u32,
}

impl PcmOutputControl {
    pub(super) fn set_playing(&self, playing: bool) {
        self.shared.playing.store(playing, Ordering::Release);
    }

    pub(super) fn set_volume(&self, volume: f64) {
        self.shared
            .volume_bits
            .store((volume as f32).to_bits(), Ordering::Release);
    }

    pub(super) fn set_muted(&self, muted: bool) {
        self.shared.muted.store(muted, Ordering::Release);
    }

    pub(super) fn position(&self) -> Duration {
        let base = Duration::from_nanos(self.shared.base_nanos.load(Ordering::Acquire));
        base.saturating_add(duration_for_samples(
            self.shared.consumed_samples.load(Ordering::Acquire),
            self.device_channels,
            self.device_sample_rate,
        ))
    }
}

impl PcmOutput {
    pub(super) fn open(source_channels: u16, source_sample_rate: u32) -> MediaResult<Self> {
        if source_channels == 0 || source_sample_rate == 0 {
            return Err(audio_output_message(
                "decoded audio has an invalid channel count or sample rate",
                MediaRecovery::None,
            ));
        }
        let host = cpal::default_host();
        let device = host.default_output_device().ok_or_else(|| {
            audio_output_message(
                "the default audio host does not expose an output device",
                MediaRecovery::Retry,
            )
        })?;
        let supported = device.default_output_config().map_err(|error| {
            audio_output_error("failed to query the default audio output format", error)
        })?;
        let config: StreamConfig = supported.clone().into();
        let device_channels = config.channels;
        let device_sample_rate = config.sample_rate;
        let shared = Arc::new(AudioQueue::new());
        let stream = build_stream(&device, &supported, &config, shared.clone())?;
        Ok(Self {
            stream,
            shared,
            source_channels,
            source_sample_rate,
            device_channels,
            device_sample_rate,
            resampler: InterleavedResampler::new(
                source_channels,
                source_sample_rate,
                device_channels,
                device_sample_rate,
            ),
        })
    }

    pub(super) fn matches_format(&self, channels: u16, sample_rate: u32) -> bool {
        self.source_channels == channels && self.source_sample_rate == sample_rate
    }

    pub(super) fn source_channels(&self) -> u16 {
        self.source_channels
    }

    pub(super) fn source_sample_rate(&self) -> u32 {
        self.source_sample_rate
    }

    pub(super) fn start(&self) -> MediaResult<()> {
        self.stream
            .play()
            .map_err(|error| audio_output_error("failed to start audio output", error))
    }

    pub(super) fn set_volume(&self, volume: f64) {
        self.shared
            .volume_bits
            .store((volume as f32).to_bits(), Ordering::Release);
    }

    pub(super) fn set_muted(&self, muted: bool) {
        self.shared.muted.store(muted, Ordering::Release);
    }

    pub(super) fn control(&self) -> PcmOutputControl {
        PcmOutputControl {
            shared: self.shared.clone(),
            device_channels: self.device_channels,
            device_sample_rate: self.device_sample_rate,
        }
    }

    pub(super) fn reset(&mut self, position: Duration) {
        self.resampler.reset();
        self.shared.reset(position);
    }

    pub(super) fn enqueue(&mut self, samples: &[f32]) {
        let samples = self.resampler.push(samples);
        self.shared.enqueue(samples);
    }

    pub(super) fn flush(&mut self) {
        let samples = self.resampler.flush();
        self.shared.enqueue(samples);
    }

    pub(super) fn buffered_duration(&self) -> Duration {
        duration_for_samples(
            self.shared.buffered_samples.load(Ordering::Acquire),
            self.device_channels,
            self.device_sample_rate,
        )
    }

    pub(super) fn position(&self) -> Duration {
        self.control().position()
    }

    pub(super) fn is_drained(&self) -> bool {
        self.shared.buffered_samples.load(Ordering::Acquire) == 0
    }

    pub(super) fn take_status(&self) -> MediaResult<OutputStatus> {
        let error = self
            .shared
            .stream_error
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        match error {
            None => Ok(OutputStatus::Ok),
            Some(OutputFailure::Interrupted) => Ok(OutputStatus::Interrupted),
            Some(OutputFailure::Fatal(error)) => Err(audio_output_message(
                format!("the audio output stream failed: {error}"),
                MediaRecovery::Retry,
            )),
        }
    }
}

pub(super) enum OutputStatus {
    Ok,
    Interrupted,
}

struct AudioQueue {
    samples: Mutex<VecDeque<f32>>,
    base_nanos: AtomicU64,
    buffered_samples: AtomicU64,
    consumed_samples: AtomicU64,
    volume_bits: AtomicU32,
    muted: AtomicBool,
    playing: AtomicBool,
    stream_error: Mutex<Option<OutputFailure>>,
}

impl AudioQueue {
    fn new() -> Self {
        Self {
            samples: Mutex::new(VecDeque::new()),
            base_nanos: AtomicU64::new(0),
            buffered_samples: AtomicU64::new(0),
            consumed_samples: AtomicU64::new(0),
            volume_bits: AtomicU32::new(1.0f32.to_bits()),
            muted: AtomicBool::new(false),
            playing: AtomicBool::new(false),
            stream_error: Mutex::new(None),
        }
    }

    fn reset(&self, position: Duration) {
        let mut samples = self
            .samples
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        samples.clear();
        self.base_nanos.store(
            position.as_nanos().min(u128::from(u64::MAX)) as u64,
            Ordering::Release,
        );
        self.buffered_samples.store(0, Ordering::Release);
        self.consumed_samples.store(0, Ordering::Release);
    }

    fn enqueue(&self, samples: Vec<f32>) {
        if samples.is_empty() {
            return;
        }
        let mut queue = self
            .samples
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        queue.extend(samples);
        self.buffered_samples.store(
            u64::try_from(queue.len()).unwrap_or(u64::MAX),
            Ordering::Release,
        );
    }

    fn write<T>(&self, output: &mut [T])
    where
        T: Sample + FromSample<f32>,
    {
        if !self.playing.load(Ordering::Acquire) {
            output.fill_with(|| T::from_sample(0.0));
            return;
        }
        let gain = if self.muted.load(Ordering::Acquire) {
            0.0
        } else {
            f32::from_bits(self.volume_bits.load(Ordering::Acquire))
        };
        let mut queue = self
            .samples
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let consumed = output.len().min(queue.len());
        for sample in output.iter_mut().take(consumed) {
            *sample = T::from_sample(queue.pop_front().unwrap_or(0.0) * gain);
        }
        for sample in output.iter_mut().skip(consumed) {
            *sample = T::from_sample(0.0);
        }
        self.buffered_samples.store(
            u64::try_from(queue.len()).unwrap_or(u64::MAX),
            Ordering::Release,
        );
        self.consumed_samples.fetch_add(
            u64::try_from(consumed).unwrap_or(u64::MAX),
            Ordering::AcqRel,
        );
    }
}

struct InterleavedResampler {
    source_channels: usize,
    source_sample_rate: u32,
    device_channels: usize,
    device_sample_rate: u32,
    source: VecDeque<f32>,
    phase: u128,
}

impl InterleavedResampler {
    fn new(
        source_channels: u16,
        source_sample_rate: u32,
        device_channels: u16,
        device_sample_rate: u32,
    ) -> Self {
        Self {
            source_channels: usize::from(source_channels),
            source_sample_rate,
            device_channels: usize::from(device_channels),
            device_sample_rate,
            source: VecDeque::new(),
            phase: 0,
        }
    }

    fn reset(&mut self) {
        self.source.clear();
        self.phase = 0;
    }

    fn push(&mut self, samples: &[f32]) -> Vec<f32> {
        self.source.extend(samples.iter().copied());
        self.produce(false)
    }

    fn flush(&mut self) -> Vec<f32> {
        let output = self.produce(true);
        self.reset();
        output
    }

    fn produce(&mut self, flush: bool) -> Vec<f32> {
        let source_frames = self.source.len() / self.source_channels;
        let mut output = Vec::new();
        let device_rate = u128::from(self.device_sample_rate);
        let source_end = (source_frames as u128).saturating_mul(device_rate);
        while self.phase.saturating_add(device_rate) < source_end
            || (flush && self.phase < source_end)
        {
            let lower = usize::try_from(self.phase / device_rate).unwrap_or(usize::MAX);
            let upper = lower.saturating_add(1).min(source_frames.saturating_sub(1));
            let fraction = (self.phase % device_rate) as f32 / self.device_sample_rate as f32;
            for channel in 0..self.device_channels {
                let lower_sample = self.source_sample(lower, channel);
                let upper_sample = self.source_sample(upper, channel);
                output.push(lower_sample + (upper_sample - lower_sample) * fraction);
            }
            self.phase = self
                .phase
                .saturating_add(u128::from(self.source_sample_rate));
        }
        let consumed_frames = usize::try_from(self.phase / device_rate).unwrap_or(usize::MAX);
        let consumed_samples = consumed_frames
            .min(source_frames)
            .saturating_mul(self.source_channels);
        self.source.drain(..consumed_samples);
        self.phase %= device_rate;
        output
    }

    fn source_sample(&self, frame: usize, device_channel: usize) -> f32 {
        if self.device_channels == 1 && self.source_channels > 1 {
            let frame_offset = frame.saturating_mul(self.source_channels);
            let sum = (0..self.source_channels)
                .filter_map(|channel| self.source.get(frame_offset + channel))
                .copied()
                .sum::<f32>();
            return sum / self.source_channels as f32;
        }
        let source_channel = if self.source_channels == 1 {
            0
        } else if device_channel < self.source_channels {
            device_channel
        } else {
            return 0.0;
        };
        self.source
            .get(frame * self.source_channels + source_channel)
            .copied()
            .unwrap_or(0.0)
    }
}

fn build_stream(
    device: &cpal::Device,
    supported: &SupportedStreamConfig,
    config: &StreamConfig,
    shared: Arc<AudioQueue>,
) -> MediaResult<Stream> {
    match supported.sample_format() {
        SampleFormat::I8 => build_typed_stream::<i8>(device, config, shared),
        SampleFormat::I16 => build_typed_stream::<i16>(device, config, shared),
        SampleFormat::I24 => build_typed_stream::<cpal::I24>(device, config, shared),
        SampleFormat::I32 => build_typed_stream::<i32>(device, config, shared),
        SampleFormat::I64 => build_typed_stream::<i64>(device, config, shared),
        SampleFormat::U8 => build_typed_stream::<u8>(device, config, shared),
        SampleFormat::U16 => build_typed_stream::<u16>(device, config, shared),
        SampleFormat::U24 => build_typed_stream::<cpal::U24>(device, config, shared),
        SampleFormat::U32 => build_typed_stream::<u32>(device, config, shared),
        SampleFormat::U64 => build_typed_stream::<u64>(device, config, shared),
        SampleFormat::F32 => build_typed_stream::<f32>(device, config, shared),
        SampleFormat::F64 => build_typed_stream::<f64>(device, config, shared),
        format => Err(MediaError::new(
            MediaErrorKind::AudioOutput,
            format!("the default audio device uses unsupported sample format {format}"),
            MediaRecovery::None,
        )),
    }
}

fn build_typed_stream<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    shared: Arc<AudioQueue>,
) -> MediaResult<Stream>
where
    T: SizedSample + FromSample<f32>,
{
    let error_state = shared.clone();
    device
        .build_output_stream(
            config,
            move |output: &mut [T], _| shared.write(output),
            move |error| {
                let message = error.to_string();
                let failure = match classify_stream_error(error) {
                    // CPAL's ALSA backend prepares or recovers the PCM stream after reporting an
                    // xrun. Rebuilding it here loses buffered PCM and forces a decoder seek,
                    // which is both unnecessary and harmful for codecs such as MP3 that need
                    // preceding bit-reservoir frames.
                    None => {
                        log::warn!("audio output xrun recovered by the backend: {message}");
                        return;
                    }
                    Some(OutputFailure::Interrupted) => {
                        log::warn!("audio output interrupted: {message}");
                        OutputFailure::Interrupted
                    }
                    Some(OutputFailure::Fatal(error)) => {
                        log::error!("gpui_media audio output stream failed: {message}");
                        OutputFailure::Fatal(error)
                    }
                };
                *error_state
                    .stream_error
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(failure);
            },
            None,
        )
        .map_err(|error| audio_output_error("failed to build the audio output stream", error))
}

fn duration_for_samples(samples: u64, channels: u16, sample_rate: u32) -> Duration {
    let denominator = u128::from(channels) * u128::from(sample_rate);
    if denominator == 0 {
        return Duration::ZERO;
    }
    let nanos = u128::from(samples).saturating_mul(1_000_000_000) / denominator;
    Duration::from_nanos(nanos.min(u128::from(u64::MAX)) as u64)
}

fn audio_output_error(
    context: impl std::fmt::Display,
    error: impl std::fmt::Display,
) -> MediaError {
    MediaError::new(
        MediaErrorKind::AudioOutput,
        format!("{context}: {error}"),
        MediaRecovery::Retry,
    )
}

enum OutputFailure {
    Interrupted,
    Fatal(String),
}

fn classify_stream_error(error: StreamError) -> Option<OutputFailure> {
    match error {
        StreamError::BufferUnderrun => None,
        StreamError::DeviceNotAvailable | StreamError::StreamInvalidated => {
            Some(OutputFailure::Interrupted)
        }
        StreamError::BackendSpecific { .. } => Some(OutputFailure::Fatal(error.to_string())),
    }
}

fn audio_output_message(
    message: impl Into<std::sync::Arc<str>>,
    recovery: MediaRecovery,
) -> MediaError {
    MediaError::new(MediaErrorKind::AudioOutput, message, recovery)
}

#[cfg(test)]
mod tests {
    use std::{sync::atomic::Ordering, time::Duration};

    use cpal::StreamError;

    use super::{AudioQueue, InterleavedResampler, OutputFailure, classify_stream_error};

    #[test]
    fn paused_callback_does_not_consume_pcm() {
        let queue = AudioQueue::new();
        queue.enqueue(vec![1.0, 2.0]);
        let mut output = [1.0; 4];
        queue.write(&mut output);
        assert_eq!(output, [0.0; 4]);
        assert_eq!(queue.consumed_samples.load(Ordering::Acquire), 0);
        assert_eq!(queue.buffered_samples.load(Ordering::Acquire), 2);

        queue.reset(Duration::from_secs(2));
        assert_eq!(queue.buffered_samples.load(Ordering::Acquire), 0);
    }

    #[test]
    fn resampler_preserves_duration_and_maps_channels() {
        let mut resampler = InterleavedResampler::new(1, 44_100, 2, 48_000);
        let mut output = resampler.push(&vec![0.0; 44_100]);
        output.extend(resampler.flush());
        assert_eq!(output.len(), 48_000 * 2);
    }

    #[test]
    fn recovered_xrun_does_not_invalidate_the_output_stream() {
        assert!(classify_stream_error(StreamError::BufferUnderrun).is_none());
        assert!(matches!(
            classify_stream_error(StreamError::DeviceNotAvailable),
            Some(OutputFailure::Interrupted)
        ));
        assert!(matches!(
            classify_stream_error(StreamError::StreamInvalidated),
            Some(OutputFailure::Interrupted)
        ));
    }
}
