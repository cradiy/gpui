use super::{A11y, A11yActionListener};
use crate::{FocusId, PointerMapping};
use accesskit::{Action, Node, NodeId};
use std::{ops::Range, sync::Arc};

#[derive(Clone)]
pub(super) enum PrepaintEvent {
    Push(NodeId, Option<Arc<Node>>),
    Pop,
    Leaf(NodeId, Arc<Node>),
    Mapping(NodeId, PointerMapping),
    Focusable(NodeId, FocusId),
    Focus(NodeId),
    ActiveDescendant(NodeId),
}

#[cfg(test)]
mod tests {
    use super::super::A11y;
    use crate::{FocusId, PointerMapping};
    use accesskit::{Node, NodeId, Rect, Role};
    use std::sync::{Arc, atomic::AtomicBool};

    #[test]
    fn cached_a11y_replays_synthetic_properties_hierarchy_and_active_descendant() {
        let mut a11y = A11y::new(Arc::new(AtomicBool::new(true)), false, None);
        a11y.sync_active_flag();
        a11y.begin_frame();
        let start = a11y.prepaint_index();
        let mut container = Node::new(Role::ListBox);
        container.set_label("Options");
        a11y.set_node_mapping(NodeId(1), PointerMapping::default());
        a11y.nodes.push(NodeId(1), container);
        a11y.set_focusable(NodeId(1), FocusId::default());
        a11y.set_focus(NodeId(1));
        a11y.nodes.push(NodeId(2), Node::new(Role::ListBoxOption));
        a11y.set_active_descendant(NodeId(2));
        let mut text = Node::new(Role::TextRun);
        text.set_value("Hello");
        text.set_bounds(Rect::new(1., 2., 30., 20.));
        text.set_character_positions(vec![0., 5., 10., 15., 20.]);
        a11y.nodes.push_leaf(NodeId(3), text);
        // Synthetic builders can edit the owner after its children have been produced.
        a11y.nodes.current_node_mut().unwrap().set_value("Selected");
        a11y.nodes.pop();
        a11y.nodes.pop();
        let range = start..a11y.prepaint_index();
        let expected = a11y.end_frame(1.);
        assert_eq!(expected.focus, NodeId(2));
        for _ in 0..3 {
            a11y.begin_frame();
            a11y.reuse_prepaint(range.clone());
            let update = a11y.end_frame(1.);
            assert_eq!(update.focus, expected.focus);
            assert_eq!(update.nodes, expected.nodes);
            assert_eq!(a11y.focus_ids[&NodeId(1)], FocusId::default());
        }
        a11y.begin_frame();
        let empty = a11y.end_frame(1.);
        assert_eq!(empty.nodes.len(), 1);
        assert!(a11y.focus_ids.is_empty());
        assert!(a11y.node_mappings.is_empty());
    }

    #[test]
    fn cached_a11y_active_descendant_uses_current_parent_focus() {
        let mut a11y = A11y::new(Arc::new(AtomicBool::new(true)), false, None);
        a11y.sync_active_flag();
        a11y.begin_frame();
        a11y.nodes.push(NodeId(1), Node::new(Role::ListBox));
        a11y.set_focusable(NodeId(1), FocusId::default());
        a11y.set_focus(NodeId(1));
        let start = a11y.prepaint_index();
        a11y.nodes.push(NodeId(2), Node::new(Role::ListBoxOption));
        a11y.set_active_descendant(NodeId(2));
        a11y.nodes.pop();
        let range = start..a11y.prepaint_index();
        a11y.nodes.pop();
        assert_eq!(a11y.end_frame(1.).focus, NodeId(2));

        a11y.begin_frame();
        a11y.nodes.push(NodeId(1), Node::new(Role::ListBox));
        a11y.reuse_prepaint(range);
        a11y.nodes.pop();
        let update = a11y.end_frame(1.);
        assert_eq!(update.focus, super::super::ROOT_NODE_ID);
        assert_eq!(
            update
                .nodes
                .iter()
                .find(|(id, _)| *id == NodeId(1))
                .unwrap()
                .1
                .children(),
            &[NodeId(2)]
        );
    }
}

impl A11y {
    pub(crate) fn cache_key(&self, focus: Option<FocusId>) -> (bool, Option<FocusId>) {
        (
            self.is_active(),
            self.is_active().then_some(focus).flatten(),
        )
    }

    pub(crate) fn cache_reuse_allowed(&self) -> bool {
        // Automation also collects semantic click closures outside the AccessKit tree.
        #[cfg(feature = "automation")]
        if self.automation_enabled {
            return false;
        }
        true
    }

    pub(crate) fn set_node_mapping(&mut self, id: NodeId, mapping: PointerMapping) {
        self.nodes
            .events
            .push(PrepaintEvent::Mapping(id, mapping.clone()));
        self.node_mappings.insert(id, mapping);
    }

    pub(crate) fn prepaint_index(&self) -> usize {
        self.nodes.events.len()
    }

    pub(crate) fn reuse_prepaint(&mut self, range: Range<usize>) {
        if !self.is_active() {
            return;
        }
        // Push records contain the final node, including synthetic properties and
        // children. Only roots attach to the live parent; inner edges are already stored.
        let mut depth = 0usize;
        let mut skipped = 0usize;
        for event in self.nodes.previous_events[range].to_vec() {
            if skipped > 0 {
                match event {
                    PrepaintEvent::Push(..) => skipped += 1,
                    PrepaintEvent::Pop => skipped -= 1,
                    _ => {}
                }
                continue;
            }
            match event {
                PrepaintEvent::Push(id, node) => {
                    if !self.nodes.push_recorded(
                        id,
                        (*node.expect("completed subtree")).clone(),
                        depth == 0,
                    ) {
                        skipped = 1;
                        continue;
                    }
                    depth += 1;
                }
                PrepaintEvent::Pop => {
                    self.nodes.pop();
                    depth -= 1;
                }
                PrepaintEvent::Leaf(id, node) => {
                    self.nodes
                        .push_leaf_recorded(id, (*node).clone(), depth == 0);
                }
                PrepaintEvent::Mapping(id, mapping) => self.set_node_mapping(id, mapping),
                PrepaintEvent::Focusable(id, focus) => self.set_focusable(id, focus),
                PrepaintEvent::Focus(id) => self.set_focus(id),
                PrepaintEvent::ActiveDescendant(id) => self.set_active_descendant(id),
            }
        }
        debug_assert_eq!(depth, 0);
    }

    pub(crate) fn can_remap(&self, range: Range<usize>, mapping: &PointerMapping) -> bool {
        !self.is_active()
            || self.nodes.previous_events[range]
                .iter()
                .all(|event| !matches!(event, PrepaintEvent::Mapping(_, old) if old != mapping))
    }

    pub(crate) fn remap_prepaint(&mut self, range: Range<usize>, mapping: &PointerMapping) {
        if !self.is_active() {
            return;
        }
        for event in &mut self.nodes.events[range] {
            if let PrepaintEvent::Mapping(id, old) = event {
                *old = mapping.clone();
                self.node_mappings.insert(*id, mapping.clone());
            }
        }
    }

    pub(crate) fn add_action(&mut self, id: NodeId, action: Action, listener: A11yActionListener) {
        let listeners = self.action_listeners.entry(id).or_default();
        self.action_order.push((id, listeners.len()));
        listeners.push(Some((action, listener)));
    }

    pub(crate) fn paint_index(&self) -> usize {
        self.action_order.len()
    }

    pub(crate) fn reuse_paint(&mut self, range: Range<usize>) {
        if !self.is_active() {
            return;
        }
        for (id, index) in self.previous_action_order[range].to_vec() {
            if let Some((action, listener)) = self
                .previous_action_listeners
                .get_mut(&id)
                .and_then(|listeners| listeners[index].take())
            {
                self.add_action(id, action, listener);
            }
        }
    }
}
