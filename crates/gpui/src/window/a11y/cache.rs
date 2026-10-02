use super::{A11y, A11yActionListener};
use crate::{FocusId, PointerMapping};
use accesskit::{Action, Node, NodeId};
use smallvec::SmallVec;
use std::{ops::Range, sync::Arc};

#[derive(Clone, Copy)]
pub(super) struct ActionRegistration {
    id: NodeId,
    index: usize,
    reused_from: Option<usize>,
}

pub(crate) struct PrepaintCheckpoint {
    events: usize,
    actions: usize,
    nodes: usize,
    ids_stack: SmallVec<[NodeId; 16]>,
    nodes_stack: SmallVec<[Node; 16]>,
    event_stack: Vec<usize>,
    focus: Option<NodeId>,
    active_descendant: Option<NodeId>,
}

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
    use accesskit::{Action, Node, NodeId, Rect, Role};
    use std::sync::{Arc, atomic::AtomicBool};
    use std::{cell::Cell, rc::Rc};

    #[test]
    fn rollback_restores_parent_properties_focus_and_cached_tree() {
        let mut a11y = A11y::new(Arc::new(AtomicBool::new(true)), false, None);
        a11y.sync_active_flag();
        a11y.begin_frame();
        let start = a11y.prepaint_index();
        let mut parent = Node::new(Role::ListBox);
        parent.set_label("Options");
        a11y.nodes.push(NodeId(1), parent);
        a11y.set_node_mapping(NodeId(1), PointerMapping::default());
        a11y.set_focusable(NodeId(1), FocusId::default());
        a11y.set_focus(NodeId(1));
        a11y.nodes
            .push_leaf(NodeId(4), Node::new(Role::ListBoxOption));
        let checkpoint = a11y.prepaint_checkpoint().unwrap();
        a11y.nodes
            .current_node_mut()
            .unwrap()
            .set_value("Discarded");
        a11y.set_node_mapping(NodeId(2), PointerMapping::default());
        a11y.nodes.push(NodeId(2), Node::new(Role::ListBoxOption));
        a11y.set_focusable(NodeId(2), FocusId::default());
        a11y.set_active_descendant(NodeId(2));
        let nested = a11y.prepaint_checkpoint().unwrap();
        a11y.nodes.push_leaf(NodeId(3), Node::new(Role::TextRun));
        a11y.rollback_prepaint(nested);
        a11y.nodes.push_leaf(NodeId(3), Node::new(Role::TextRun));
        a11y.nodes.pop();
        a11y.rollback_prepaint(checkpoint);
        assert_eq!(a11y.nodes.current_node_mut().unwrap().value(), None);
        assert_eq!(a11y.focus_ids.len(), 1);
        assert_eq!(a11y.node_mappings.len(), 1);
        assert!(a11y.node_mappings.contains_key(&NodeId(1)));
        a11y.nodes.push(NodeId(2), Node::new(Role::ListBoxOption));
        a11y.set_active_descendant(NodeId(2));
        a11y.nodes.push_leaf(NodeId(3), Node::new(Role::TextRun));
        a11y.nodes.pop();
        a11y.nodes.pop();
        let range = start..a11y.prepaint_index();
        let expected = a11y.end_frame(1.);
        assert_eq!(expected.focus, NodeId(2));
        assert_eq!(expected.nodes.len(), 5);
        assert_eq!(
            expected
                .nodes
                .iter()
                .find(|(id, _)| *id == NodeId(1))
                .unwrap()
                .1
                .children(),
            &[NodeId(4), NodeId(2)]
        );
        a11y.begin_frame();
        a11y.reuse_prepaint(range);
        let cached = a11y.end_frame(1.);
        assert_eq!(cached.nodes, expected.nodes);
        assert_eq!(cached.focus, expected.focus);
    }

    #[test]
    fn rollback_releases_new_actions_and_returns_reused_actions_for_retry() {
        let mut a11y = A11y::new(Arc::new(AtomicBool::new(true)), false, None);
        a11y.sync_active_flag();
        a11y.begin_frame();
        let owner = Rc::new(Cell::new(0));
        let retained = owner.clone();
        a11y.add_action(
            NodeId(1),
            Action::Click,
            Box::new(move |_, _, _| {
                retained.set(retained.get() + 1);
            }),
        );
        let range = 0..a11y.paint_index();
        a11y.end_frame(1.);
        for _ in 0..3 {
            a11y.begin_frame();
            let checkpoint = a11y.prepaint_checkpoint().unwrap();
            a11y.reuse_paint(range.clone());
            let new_owner = owner.clone();
            a11y.add_action(
                NodeId(1),
                Action::Increment,
                Box::new(move |_, _, _| {
                    new_owner.set(new_owner.get() + 1);
                }),
            );
            assert_eq!(Rc::strong_count(&owner), 3);
            a11y.rollback_prepaint(checkpoint);
            assert!(a11y.action_listeners.is_empty());
            assert_eq!(Rc::strong_count(&owner), 2);
            a11y.reuse_paint(range.clone());
            assert_eq!(a11y.action_listeners[&NodeId(1)].len(), 1);
            assert_eq!(
                a11y.action_listeners[&NodeId(1)][0].as_ref().unwrap().0,
                Action::Click
            );
            a11y.end_frame(1.);
            assert_eq!(Rc::strong_count(&owner), 2);
        }
        a11y.begin_frame();
        a11y.end_frame(1.);
        assert_eq!(Rc::strong_count(&owner), 1);
    }

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
    pub(crate) fn prepaint_checkpoint(&self) -> Option<PrepaintCheckpoint> {
        self.is_active().then(|| PrepaintCheckpoint {
            events: self.prepaint_index(),
            actions: self.paint_index(),
            nodes: self.nodes.all_nodes.len(),
            ids_stack: self.nodes.ids_stack.clone(),
            nodes_stack: self.nodes.nodes_stack.clone(),
            event_stack: self.nodes.event_stack.clone(),
            focus: self.nodes.focus,
            active_descendant: self.nodes.active_descendant,
        })
    }

    pub(crate) fn rollback_prepaint(&mut self, checkpoint: PrepaintCheckpoint) {
        for event in self.nodes.events.drain(checkpoint.events..) {
            if let PrepaintEvent::Push(id, _) | PrepaintEvent::Leaf(id, _) = event {
                self.nodes.seen_ids.remove(&id);
            }
        }
        self.nodes.all_nodes.truncate(checkpoint.nodes);
        self.nodes.ids_stack = checkpoint.ids_stack;
        self.nodes.nodes_stack = checkpoint.nodes_stack;
        self.nodes.event_stack = checkpoint.event_stack;
        self.nodes.focus = checkpoint.focus;
        self.nodes.active_descendant = checkpoint.active_descendant;

        self.focus_ids.clear();
        self.node_mappings.clear();
        for event in &self.nodes.events {
            match event {
                PrepaintEvent::Focusable(id, focus) => {
                    self.focus_ids.insert(*id, *focus);
                }
                PrepaintEvent::Mapping(id, mapping) => {
                    self.node_mappings.insert(*id, mapping.clone());
                }
                _ => {}
            }
        }
        for registration in self.action_order.drain(checkpoint.actions..).rev() {
            let listeners = self.action_listeners.get_mut(&registration.id).unwrap();
            debug_assert_eq!(listeners.len(), registration.index + 1);
            let listener = listeners.pop().unwrap();
            if listeners.is_empty() {
                self.action_listeners.remove(&registration.id);
            }
            if let Some(index) = registration.reused_from {
                self.previous_action_listeners
                    .get_mut(&registration.id)
                    .unwrap()[index] = listener;
            }
        }
    }

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
        for index in range {
            let event = self.nodes.previous_events[index].clone();
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
        self.action_order.push(ActionRegistration {
            id,
            index: listeners.len(),
            reused_from: None,
        });
        listeners.push(Some((action, listener)));
    }

    pub(crate) fn paint_index(&self) -> usize {
        self.action_order.len()
    }

    pub(crate) fn reuse_paint(&mut self, range: Range<usize>) {
        if !self.is_active() {
            return;
        }
        for previous_index in range {
            let ActionRegistration { id, index, .. } = self.previous_action_order[previous_index];
            if let Some((action, listener)) = self
                .previous_action_listeners
                .get_mut(&id)
                .and_then(|listeners| listeners[index].take())
            {
                self.add_action(id, action, listener);
                self.action_order.last_mut().unwrap().reused_from = Some(index);
            }
        }
    }
}
