#[cfg(feature = "input-latency-histogram")]
use anyhow::{Result, anyhow};
#[cfg(feature = "input-latency-histogram")]
use hdrhistogram::Histogram;
use scheduler::Instant;
use std::time::Duration;

/// Tracks input event timestamps to determine if input is arriving at a high rate.
/// Used for selective VRR (Variable Refresh Rate) optimization.
#[derive(Clone, Debug)]
pub(crate) struct InputRateTracker {
    timestamps: Vec<Instant>,
    window: Duration,
    inputs_per_second: u32,
    sustain_until: Instant,
    sustain_duration: Duration,
}

impl Default for InputRateTracker {
    fn default() -> Self {
        Self {
            timestamps: Vec::new(),
            window: Duration::from_millis(100),
            inputs_per_second: 60,
            sustain_until: Instant::now(),
            sustain_duration: Duration::from_secs(1),
        }
    }
}

impl InputRateTracker {
    pub fn record_input(&mut self) {
        let now = Instant::now();
        self.timestamps.push(now);
        self.prune_old_timestamps(now);

        let min_events = self.inputs_per_second as u128 * self.window.as_millis() / 1000;
        if self.timestamps.len() as u128 >= min_events {
            self.sustain_until = now + self.sustain_duration;
        }
    }

    pub fn is_high_rate(&self) -> bool {
        Instant::now() < self.sustain_until
    }

    fn prune_old_timestamps(&mut self, now: Instant) {
        self.timestamps
            .retain(|&t| now.duration_since(t) <= self.window);
    }
}

/// A point-in-time snapshot of the input-latency histograms for a window,
/// suitable for external formatting.
#[cfg(feature = "input-latency-histogram")]
pub struct InputLatencySnapshot {
    /// Histogram of input-to-frame latency samples, in nanoseconds.
    pub latency_histogram: Histogram<u64>,
    /// Histogram of input events coalesced per rendered frame.
    pub events_per_frame_histogram: Histogram<u64>,
    /// Count of input events that arrived mid-draw and were excluded from
    /// latency recording.
    pub mid_draw_events_dropped: u64,
}

/// Records the time between when the first input event in a frame is dispatched
/// and when the resulting frame is presented, capturing worst-case latency when
/// multiple events are coalesced into a single frame.
#[cfg(feature = "input-latency-histogram")]
pub(super) struct InputLatencyTracker {
    /// Timestamp of the first unrendered input event in the current frame;
    /// cleared when a frame is presented.
    first_input_at: Option<Instant>,
    /// Count of input events received since the last frame was presented.
    pending_input_count: u64,
    /// Histogram of input-to-frame latency samples, in nanoseconds.
    latency_histogram: Histogram<u64>,
    /// Histogram of input events coalesced per rendered frame.
    events_per_frame_histogram: Histogram<u64>,
    /// Count of input events that arrived mid-draw and were excluded from
    /// latency recording because their effects won't appear until the next frame.
    mid_draw_events_dropped: u64,
}

#[cfg(feature = "input-latency-histogram")]
impl InputLatencyTracker {
    pub(super) fn new() -> Result<Self> {
        Ok(Self {
            first_input_at: None,
            pending_input_count: 0,
            latency_histogram: Histogram::new(3)
                .map_err(|e| anyhow!("Failed to create input latency histogram: {e}"))?,
            events_per_frame_histogram: Histogram::new(3)
                .map_err(|e| anyhow!("Failed to create events per frame histogram: {e}"))?,
            mid_draw_events_dropped: 0,
        })
    }

    /// Record that an input event was dispatched at the given time.
    /// Only the first event's timestamp per frame is retained (worst-case latency).
    pub(super) fn record_input(&mut self, dispatch_time: Instant) {
        self.first_input_at.get_or_insert(dispatch_time);
        self.pending_input_count += 1;
    }

    /// Record that an input event arrived during a draw phase and was excluded
    /// from latency tracking.
    pub(super) fn record_mid_draw_input(&mut self) {
        self.mid_draw_events_dropped += 1;
    }

    /// Record that a frame was presented, flushing pending latency and coalescing samples.
    pub(super) fn record_frame_presented(&mut self) {
        if let Some(first_input_at) = self.first_input_at.take() {
            let latency_nanos = first_input_at.elapsed().as_nanos() as u64;
            self.latency_histogram.record(latency_nanos).ok();
        }
        if self.pending_input_count > 0 {
            self.events_per_frame_histogram
                .record(self.pending_input_count)
                .ok();
            self.pending_input_count = 0;
        }
    }

    pub(super) fn snapshot(&self) -> InputLatencySnapshot {
        InputLatencySnapshot {
            latency_histogram: self.latency_histogram.clone(),
            events_per_frame_histogram: self.events_per_frame_histogram.clone(),
            mid_draw_events_dropped: self.mid_draw_events_dropped,
        }
    }
}
