use std::sync::Arc;

use gpui::{Context, SharedString};

/// Controls whether neighboring [`super::Collapsible`] items may be open at
/// the same time.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CollapsibleMode {
    /// Every item is toggled independently.
    #[default]
    Multiple,
    /// Opening one item closes the previously open item.
    Single,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CollapsibleEvent {
    Changed(Arc<[SharedString]>),
}

/// Shared expansion state for one or more collapsible items.
pub struct CollapsibleState {
    mode: CollapsibleMode,
    expanded: Vec<SharedString>,
}

impl gpui::EventEmitter<CollapsibleEvent> for CollapsibleState {}

impl CollapsibleState {
    pub fn new(mode: CollapsibleMode) -> Self {
        Self {
            mode,
            expanded: Vec::new(),
        }
    }

    pub fn with_expanded<S>(mut self, items: impl IntoIterator<Item = S>) -> Self
    where
        S: Into<SharedString>,
    {
        self.expanded = unique_items(items);
        if self.mode == CollapsibleMode::Single {
            self.expanded.truncate(1);
        }
        self
    }

    pub fn mode(&self) -> CollapsibleMode {
        self.mode
    }

    pub fn expanded(&self) -> &[SharedString] {
        &self.expanded
    }

    pub fn is_expanded(&self, item: &str) -> bool {
        self.expanded
            .iter()
            .any(|candidate| candidate.as_ref() == item)
    }

    pub fn set_mode(&mut self, mode: CollapsibleMode, cx: &mut Context<Self>) {
        if self.mode == mode {
            return;
        }
        self.mode = mode;
        if mode == CollapsibleMode::Single && self.expanded.len() > 1 {
            self.expanded.truncate(1);
            self.emit_changed(cx);
        } else {
            cx.notify();
        }
    }

    pub fn set_expanded<S>(&mut self, items: impl IntoIterator<Item = S>, cx: &mut Context<Self>)
    where
        S: Into<SharedString>,
    {
        let mut expanded = unique_items(items);
        if self.mode == CollapsibleMode::Single {
            expanded.truncate(1);
        }
        if self.expanded == expanded {
            return;
        }
        self.expanded = expanded;
        self.emit_changed(cx);
    }

    pub fn set_item_expanded(
        &mut self,
        item: impl Into<SharedString>,
        expanded: bool,
        cx: &mut Context<Self>,
    ) {
        let item = item.into();
        let index = self
            .expanded
            .iter()
            .position(|candidate| candidate == &item);
        if expanded {
            if self.mode == CollapsibleMode::Single {
                if self.expanded.len() == 1 && index == Some(0) {
                    return;
                }
                self.expanded.clear();
                self.expanded.push(item);
            } else if index.is_none() {
                self.expanded.push(item);
            } else {
                return;
            }
        } else if let Some(index) = index {
            self.expanded.remove(index);
        } else {
            return;
        }
        self.emit_changed(cx);
    }

    pub fn toggle(&mut self, item: impl Into<SharedString>, cx: &mut Context<Self>) {
        let item = item.into();
        let expanded = !self.is_expanded(item.as_ref());
        self.set_item_expanded(item, expanded, cx);
    }

    pub fn collapse_all(&mut self, cx: &mut Context<Self>) {
        if self.expanded.is_empty() {
            return;
        }
        self.expanded.clear();
        self.emit_changed(cx);
    }

    fn emit_changed(&self, cx: &mut Context<Self>) {
        cx.emit(CollapsibleEvent::Changed(self.expanded.clone().into()));
        cx.notify();
    }
}

fn unique_items<S>(items: impl IntoIterator<Item = S>) -> Vec<SharedString>
where
    S: Into<SharedString>,
{
    let mut unique = Vec::new();
    for item in items {
        let item = item.into();
        if !unique.contains(&item) {
            unique.push(item);
        }
    }
    unique
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_single_mode_keeps_one_item() {
        let state =
            CollapsibleState::new(CollapsibleMode::Single).with_expanded(["first", "second"]);
        assert_eq!(state.expanded(), &[SharedString::from("first")]);
    }

    #[test]
    fn multiple_mode_preserves_independent_items() {
        let state = CollapsibleState::new(CollapsibleMode::Multiple)
            .with_expanded(["first", "second", "first"]);
        assert_eq!(
            state.expanded(),
            &[SharedString::from("first"), SharedString::from("second")]
        );
    }
}
