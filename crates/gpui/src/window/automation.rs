use crate::{App, Bounds, Pixels, Size, Window, WindowId, point, px};
use accesskit::{NodeId, Role, Toggled, TreeUpdate};
use collections::{FxHashMap, FxHashSet};
use std::sync::Arc;

#[derive(Default)]
pub(super) struct State {
    generation: u64,
    snapshot: Option<Arc<AutomationSnapshot>>,
    pub(super) click_handlers: FxHashMap<NodeId, (Bounds<Pixels>, Vec<crate::ClickListener>)>,
}

/// A semantic node from one completed GPUI draw. IDs are scoped to a window.
#[derive(Clone, Debug)]
pub struct AutomationNode {
    /// Opaque identity, stable while the element's GPUI ID path is unchanged.
    pub id: u64,
    /// Explicit application identifier, if supplied.
    pub automation_id: Option<String>,
    /// Semantic role.
    pub role: Role,
    /// Authored label. This is not an inferred accessible name.
    pub label: Option<String>,
    /// Text or value reported by the element, absent for sensitive content.
    pub value: Option<String>,
    /// Numeric value, absent for sensitive content.
    pub numeric_value: Option<f64>,
    /// Child identities in semantic tree order.
    pub children: Vec<u64>,
    /// Semantic parent, absent on the window root.
    pub parent: Option<u64>,
    /// Window-local logical bounds. Does not establish visibility or hit-testability.
    pub bounds: Option<Bounds<Pixels>>,
    /// Disabled state, including disabled semantic ancestors.
    pub disabled: bool,
    /// Read-only state reported by the element.
    pub read_only: bool,
    /// Hidden state reported by the semantic tree, including hidden ancestors.
    pub hidden: bool,
    /// Selected state if the element reports one.
    pub selected: Option<bool>,
    /// Toggle state if the element reports one.
    pub toggled: Option<Toggled>,
    /// Whether this is the semantic focus target.
    pub focused: bool,
    /// Whether content was redacted because this node is in a password subtree.
    pub redacted: bool,
    /// Supported automation actions advertised by this node.
    pub actions: Vec<accesskit::Action>,
}

/// A semantic operation, not a physical pointer or keyboard event sequence.
#[derive(Clone, Debug)]
pub enum AutomationAction {
    /// Invoke a semantic click handler.
    Click,
    /// Focus the node's registered focus target.
    Focus,
    /// Replace the value through a registered SetValue handler.
    SetValue(String),
}

/// Immutable semantic data from a completed draw, not a presentation or GPU fence.
#[derive(Clone, Debug)]
pub struct AutomationSnapshot {
    /// Window that produced the snapshot.
    pub window: WindowId,
    /// Per-window monotonically increasing snapshot version.
    pub generation: u64,
    /// Window title at capture time.
    pub title: String,
    /// Device pixels per logical pixel at capture time.
    pub scale_factor: f32,
    /// Logical drawable size at capture time.
    pub viewport_size: Size<Pixels>,
    /// Window root identity.
    pub root: u64,
    /// Nodes in depth-first semantic order.
    pub nodes: Vec<AutomationNode>,
}

impl Window {
    pub(crate) fn register_automation_click(
        &mut self,
        node: NodeId,
        bounds: Bounds<Pixels>,
        listeners: &[crate::ClickListener],
    ) {
        self.automation
            .click_handlers
            .insert(node, (bounds, listeners.to_vec()));
    }

    /// Whether this window has opted into automation collection.
    pub fn is_automation_enabled(&self) -> bool {
        self.a11y.automation_enabled
    }

    /// Dispatches against the current completed draw. Rejects stale generations,
    /// pending redraws, unavailable nodes and unsupported actions. Success means
    /// a handler was dispatched, not that the requested state was achieved.
    pub fn perform_automation_action(
        &mut self,
        generation: u64,
        node_id: u64,
        action: AutomationAction,
        cx: &mut App,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(self.invalidator.not_drawing(), "cannot act during a draw");
        let snapshot = self
            .automation
            .snapshot
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("automation snapshot is not ready"))?;
        anyhow::ensure!(
            snapshot.generation == generation,
            "stale automation generation"
        );
        anyhow::ensure!(!self.invalidator.is_dirty(), "window has a pending redraw");
        let node = snapshot
            .nodes
            .iter()
            .find(|node| node.id == node_id)
            .ok_or_else(|| anyhow::anyhow!("automation node is no longer present"))?;
        anyhow::ensure!(
            !node.disabled && !node.hidden,
            "automation node is disabled or hidden"
        );
        let (action, data) = match action {
            AutomationAction::Click => (accesskit::Action::Click, None),
            AutomationAction::Focus => (accesskit::Action::Focus, None),
            AutomationAction::SetValue(value) => (
                accesskit::Action::SetValue,
                Some(accesskit::ActionData::Value(value.into())),
            ),
        };
        anyhow::ensure!(
            action != accesskit::Action::SetValue || !node.read_only,
            "automation node is read-only"
        );
        anyhow::ensure!(
            node.actions.contains(&action),
            "node does not support {action:?}"
        );
        let target = NodeId(node_id);
        let registered = self
            .a11y
            .action_listeners
            .get(&target)
            .is_some_and(|listeners| listeners.iter().any(|(kind, _)| *kind == action));
        if registered {
            let mut listeners = self.a11y.action_listeners.remove(&target).unwrap();
            for (kind, listener) in &mut listeners {
                if *kind == action {
                    listener(data.as_ref(), self, cx);
                }
            }
            self.a11y.action_listeners.insert(target, listeners);
        } else if action == accesskit::Action::Click {
            let (bounds, listeners) = self
                .automation
                .click_handlers
                .get(&target)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("node has no semantic click handler"))?;
            let event = crate::ClickEvent::Keyboard(crate::KeyboardClickEvent {
                bounds,
                ..Default::default()
            });
            for listener in listeners {
                listener(&event, self, cx);
            }
        } else if action == accesskit::Action::Focus {
            let handle = self
                .a11y
                .focus_ids
                .get(&target)
                .and_then(|id| crate::FocusHandle::for_id(*id, &cx.focus_handles))
                .ok_or_else(|| anyhow::anyhow!("node has no focus target"))?;
            self.focus(&handle, cx);
        } else {
            anyhow::bail!("node has no semantic handler for {action:?}");
        }
        self.refresh();
        Ok(())
    }

    /// Enables semantic collection for this window independently of platform
    /// accessibility activation. Requires the `automation` feature. No server is
    /// started. Disabling drops the retained snapshot; existing clones remain valid.
    pub fn set_automation_enabled(&mut self, enabled: bool) -> anyhow::Result<()> {
        anyhow::ensure!(
            !enabled || self.a11y.allow_automation(),
            "accessibility is disabled for this window"
        );
        if self.a11y.automation_enabled != enabled {
            self.a11y.automation_enabled = enabled;
            self.automation.snapshot = None;
            self.automation.click_handlers.clear();
            self.refresh();
        }
        Ok(())
    }

    /// Returns the most recently completed semantic snapshot. `None` means
    /// collection is disabled or no draw has completed since it was enabled.
    pub fn automation_snapshot(&self) -> Option<Arc<AutomationSnapshot>> {
        self.automation.snapshot.clone()
    }

    pub(super) fn publish_automation_snapshot(&mut self, update: &TreeUpdate) {
        if !self.a11y.automation_enabled {
            return;
        }
        self.automation.generation += 1;
        let scale = self.scale_factor();
        let raw: FxHashMap<_, _> = update.nodes.iter().map(|(id, node)| (*id, node)).collect();
        let mut nodes = Vec::with_capacity(raw.len());
        let root = update.tree.as_ref().map_or(NodeId(0), |tree| tree.root);
        let mut stack = vec![(root, None, false, false, false)];
        let mut seen = FxHashSet::default();
        while let Some((id, parent, secret_parent, disabled_parent, hidden_parent)) = stack.pop() {
            let Some(node) = raw.get(&id) else {
                continue;
            };
            if !seen.insert(id) {
                continue;
            }
            let secret = secret_parent || node.role() == Role::PasswordInput;
            let disabled = disabled_parent || node.is_disabled();
            let hidden = hidden_parent || node.is_hidden();
            stack.extend(
                node.children()
                    .iter()
                    .rev()
                    .map(|child| (*child, Some(id.0), secret, disabled, hidden)),
            );
            nodes.push(AutomationNode {
                id: id.0,
                parent,
                automation_id: node.author_id().map(str::to_owned),
                role: node.role(),
                label: if secret_parent {
                    None
                } else {
                    node.label().map(str::to_owned)
                },
                value: if secret {
                    None
                } else {
                    node.value().map(str::to_owned)
                },
                numeric_value: if secret { None } else { node.numeric_value() },
                children: node.children().iter().map(|child| child.0).collect(),
                bounds: node.bounds().map(|rect| {
                    Bounds::from_corners(
                        point(px(rect.x0 as f32 / scale), px(rect.y0 as f32 / scale)),
                        point(px(rect.x1 as f32 / scale), px(rect.y1 as f32 / scale)),
                    )
                }),
                disabled,
                read_only: node.is_read_only(),
                hidden,
                selected: node.is_selected(),
                toggled: node.toggled(),
                focused: update.focus == id,
                redacted: secret,
                actions: [
                    accesskit::Action::Click,
                    accesskit::Action::Focus,
                    accesskit::Action::SetValue,
                ]
                .into_iter()
                .filter(|action| node.supports_action(*action))
                .collect(),
            });
        }
        self.automation.snapshot = Some(Arc::new(AutomationSnapshot {
            window: self.window_handle().window_id(),
            generation: self.automation.generation,
            title: self.window_title(),
            scale_factor: scale,
            viewport_size: self.viewport_size(),
            root: root.0,
            nodes,
        }));
    }
}
