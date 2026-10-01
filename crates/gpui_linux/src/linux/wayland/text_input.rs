use gpui::{Bounds, FocusId, Pixels, PreeditSelection, SurroundingText};

pub(super) const MAX_SURROUNDING_BYTES: usize = 4000;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct InputContext {
    pub focus: Option<FocusId>,
    pub surrounding: Option<SurroundingText>,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Preedit {
    pub text: String,
    pub selection: PreeditSelection,
}

impl Preedit {
    pub fn new(text: Option<String>, cursor_begin: i32, cursor_end: i32) -> Self {
        let text = text.unwrap_or_default();
        let offset = |byte: i32| {
            let mut byte = (byte.max(0) as usize).min(text.len());
            while !text.is_char_boundary(byte) {
                byte -= 1;
            }
            text[..byte].encode_utf16().count()
        };
        let selection = if cursor_begin == -1 && cursor_end == -1 {
            PreeditSelection::Hidden
        } else {
            PreeditSelection::Range {
                anchor: offset(cursor_end),
                head: offset(cursor_begin),
            }
        };
        Self { text, selection }
    }
}

#[derive(Default, Debug, PartialEq, Eq)]
pub(super) struct ImeBatch {
    pub commit: Option<String>,
    pub preedit: Option<Preedit>,
    pub delete: Option<(u32, u32)>,
}

#[derive(Default)]
pub(super) struct TextInputState {
    pub pending: ImeBatch,
    serial: u32,
    pub cursor_rectangle: Option<[i32; 4]>,
    pub context: Option<InputContext>,
    surrounding_serial: u32,
    pub defer_publish: bool,
}

impl TextInputState {
    pub fn committed(&mut self) {
        self.serial = self.serial.wrapping_add(1);
    }

    pub fn can_publish(&self, serial: u32) -> bool {
        self.serial == serial
    }

    pub fn record_context(&mut self, context: InputContext) {
        self.context = Some(context);
        self.surrounding_serial = self.serial.wrapping_add(1);
    }

    pub fn deletion_context(&self, serial: u32) -> Option<&InputContext> {
        (serial.wrapping_sub(self.surrounding_serial) < (1 << 31))
            .then_some(self.context.as_ref())
            .flatten()
    }

    pub fn reset_context(&mut self) {
        self.context = None;
        self.cursor_rectangle = None;
        self.defer_publish = false;
        self.take_pending();
    }

    pub fn take_pending(&mut self) -> ImeBatch {
        std::mem::take(&mut self.pending)
    }

    pub fn update_cursor_rectangle(&mut self, bounds: Bounds<Pixels>) -> Option<[i32; 4]> {
        let rectangle = [
            bounds.origin.x.as_f32() as i32,
            bounds.origin.y.as_f32() as i32,
            bounds.size.width.as_f32() as i32,
            bounds.size.height.as_f32() as i32,
        ];
        if self.cursor_rectangle == Some(rectangle) {
            return None;
        }
        self.cursor_rectangle = Some(rectangle);
        Some(rectangle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ime_preedit_preserves_utf16_cursor_direction_and_visibility() {
        let preedit = |begin, end| Preedit::new(Some("你😀好".into()), begin, end).selection;
        assert_eq!(
            preedit(3, 7),
            PreeditSelection::Range { anchor: 3, head: 1 }
        );
        assert_eq!(
            preedit(7, 3),
            PreeditSelection::Range { anchor: 1, head: 3 }
        );
        assert_eq!(
            preedit(7, 7),
            PreeditSelection::Range { anchor: 3, head: 3 }
        );
        assert_eq!(preedit(-1, -1), PreeditSelection::Hidden);
        assert_eq!(
            preedit(5, 99),
            PreeditSelection::Range { anchor: 4, head: 1 }
        );
        assert_eq!(Preedit::new(None, 0, 0).text, "");
    }

    #[test]
    fn ime_done_drains_one_batch_and_checks_client_commit_serial() {
        let mut state = TextInputState::default();
        state.committed();
        state.committed();
        state.pending.commit = Some("你".into());
        state.pending.preedit = Some(Preedit::new(Some("hao".into()), 1, 1));
        state.pending.delete = Some((4, 3));
        assert!(!state.can_publish(1));
        let batch = state.take_pending();
        assert_eq!(batch.commit.as_deref(), Some("你"));
        assert_eq!(batch.preedit.unwrap().text, "hao");
        assert_eq!(batch.delete, Some((4, 3)));
        assert_eq!(state.take_pending(), ImeBatch::default());
        assert!(state.can_publish(2));
        state.serial = u32::MAX;
        state.committed();
        assert!(state.can_publish(0));
    }

    #[test]
    fn surrounding_deletion_requires_a_current_snapshot_and_resets_with_focus() {
        let mut state = TextInputState::default();
        let context = InputContext {
            focus: None,
            surrounding: SurroundingText::from_utf8("前😀", 7, 7, None, 4000),
        };
        state.record_context(context.clone());
        state.committed();
        assert!(state.deletion_context(0).is_none());
        assert_eq!(state.deletion_context(1), Some(&context));
        state.committed(); // A cursor rectangle update retains the same snapshot.
        assert_eq!(state.deletion_context(2), Some(&context));
        state.pending.delete = Some((4, 0));
        state.reset_context();
        assert!(state.deletion_context(2).is_none());
        assert_eq!(state.take_pending(), ImeBatch::default());

        state.serial = u32::MAX;
        state.record_context(context.clone());
        state.committed();
        assert!(state.deletion_context(u32::MAX).is_none());
        assert_eq!(state.deletion_context(0), Some(&context));
    }

    #[test]
    fn ime_cursor_updates_only_publish_changed_protocol_coordinates() {
        let mut state = TextInputState::default();
        let bounds = |x| {
            Bounds::new(
                gpui::point(gpui::px(x), gpui::px(20.)),
                gpui::size(gpui::px(1.), gpui::px(18.)),
            )
        };
        assert_eq!(
            state.update_cursor_rectangle(bounds(10.)),
            Some([10, 20, 1, 18])
        );
        assert_eq!(state.update_cursor_rectangle(bounds(10.5)), None);
        assert_eq!(
            state.update_cursor_rectangle(bounds(12.)),
            Some([12, 20, 1, 18])
        );
        state.cursor_rectangle = None;
        assert_eq!(
            state.update_cursor_rectangle(bounds(12.)),
            Some([12, 20, 1, 18])
        );
    }
}
