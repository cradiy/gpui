use collections::HashMap;

#[derive(Debug, Hash, PartialEq, Eq)]
pub(crate) enum SerialKind {
    DataDevice,
    InputMethod,
    MouseEnter,
    MousePress,
    KeyPress,
}

#[derive(Debug)]
struct SerialData {
    serial: u32,
}

impl SerialData {
    fn new(value: u32) -> Self {
        Self { serial: value }
    }
}

#[derive(Debug)]
/// Helper for tracking of different serial kinds.
pub(crate) struct SerialTracker {
    serials: HashMap<SerialKind, SerialData>,
    latest_input: u32,
}

impl SerialTracker {
    pub fn new() -> Self {
        Self {
            serials: HashMap::default(),
            latest_input: 0,
        }
    }

    pub fn update(&mut self, kind: SerialKind, value: u32) {
        if matches!(kind, SerialKind::MousePress | SerialKind::KeyPress) {
            self.latest_input = value;
        }
        self.serials.insert(kind, SerialData::new(value));
    }

    pub(crate) fn get_latest_input(&self) -> u32 {
        self.latest_input
    }

    /// Returns the latest tracked serial of the provided [`SerialKind`]
    ///
    /// Will return 0 if not tracked.
    pub fn get(&self, kind: SerialKind) -> u32 {
        self.serials
            .get(&kind)
            .map(|serial_data| serial_data.serial)
            .unwrap_or(0)
    }

    /// Returns the most recent serial across all tracked kinds.
    ///
    /// Wayland compositor serial numbers are monotonically increasing, so the
    /// highest value is always the most recently received one. This is the
    /// correct serial to use for [`set_selection`] when the triggering event
    /// may have been a mouse press rather than a key press: using 0 (the
    /// default when a kind has never been seen) causes compositors to silently
    /// reject the request.
    ///
    /// Returns 0 only if no serial of any kind has been received yet.
    pub fn get_latest(&self) -> u32 {
        self.serials
            .values()
            .map(|serial_data| serial_data.serial)
            .max()
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activation_serial_tracks_input_order_including_wraparound() {
        let mut tracker = SerialTracker::new();
        assert_eq!(tracker.get_latest_input(), 0);
        tracker.update(SerialKind::MousePress, u32::MAX - 1);
        tracker.update(SerialKind::MouseEnter, u32::MAX);
        assert_eq!(tracker.get_latest_input(), u32::MAX - 1);
        tracker.update(SerialKind::KeyPress, 1);
        tracker.update(SerialKind::DataDevice, 2);
        assert_eq!(tracker.get_latest_input(), 1);
        tracker.update(SerialKind::MousePress, 3);
        assert_eq!(tracker.get_latest_input(), 3);
    }
}
