/// Registry order determines the selected seat and its replacement. Adding a
/// second seat never displaces the one already serving the application.
#[derive(Default)]
pub(super) struct Seats {
    available: Vec<(u32, u32)>,
}

impl Seats {
    pub fn selected(&self) -> Option<(u32, u32)> {
        self.available.first().copied()
    }

    /// Returns whether the selected seat changed.
    pub fn add(&mut self, name: u32, version: u32) -> bool {
        if self.available.iter().any(|seat| seat.0 == name) {
            return false;
        }
        let changed = self.available.is_empty();
        self.available.push((name, version));
        changed
    }

    /// Returns whether removal requires releasing the selected seat.
    pub fn remove(&mut self, name: u32) -> bool {
        let changed = self.selected().is_some_and(|seat| seat.0 == name);
        self.available.retain(|seat| seat.0 != name);
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seat_hotplug_preserves_selection_and_uses_registry_order_for_fallback() {
        let mut seats = Seats::default();
        assert!(seats.add(20, 7));
        assert!(!seats.add(5, 8));
        assert!(!seats.add(30, 9));
        assert!(!seats.add(20, 7));
        assert_eq!(seats.selected(), Some((20, 7)));
        assert!(!seats.remove(99));
        assert!(!seats.remove(30));
        assert!(seats.remove(20));
        assert_eq!(seats.selected(), Some((5, 8)));
        assert!(seats.remove(5));
        assert_eq!(seats.selected(), None);
        assert!(!seats.remove(5));
        assert!(seats.add(40, 9));
        assert_eq!(seats.selected(), Some((40, 9)));
    }
}
