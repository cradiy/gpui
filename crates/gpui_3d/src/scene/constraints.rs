use super::{EvaluatedScene, NodeHandle, SceneError, SceneGraph};
use crate::AffineTransform;
use std::collections::HashMap;

/// Stateless world-transform constraints evaluated after local pose overrides.
/// Constraints do not change hierarchy, visibility inheritance, or authored transforms.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TransformConstraint {
    /// Replaces the node's world transform with `target.world * offset`.
    /// The offset is in target-local space and supports scale, shear, and reflection.
    /// The node's own local transform is not applied. Descendants inherit the result.
    Follow {
        target: NodeHandle,
        offset: AffineTransform,
    },
}

impl SceneGraph {
    /// Evaluates local pose overrides and world constraints without mutating the graph.
    /// Each node accepts at most one constraint, which takes precedence over its local
    /// transform. Targets use their final constrained transforms regardless of input
    /// order. Hierarchy-parent and target dependencies must form an acyclic graph.
    /// Hidden nodes participate in evaluation; only hierarchy controls visibility.
    /// Omitting a constraint restores the node's authored or supplied local transform.
    pub fn evaluate_with_constraints(
        &self,
        transforms: impl IntoIterator<Item = (NodeHandle, AffineTransform)>,
        constraints: impl IntoIterator<Item = (NodeHandle, TransformConstraint)>,
    ) -> Result<EvaluatedScene, SceneError> {
        let mut locals = HashMap::new();
        for (node, local) in transforms {
            self.node(node)?;
            if locals.insert(node, local).is_some() {
                return Err(SceneError::DuplicateTransform(node));
            }
        }
        let mut bindings = HashMap::new();
        for (node, constraint) in constraints {
            self.node(node)?;
            let TransformConstraint::Follow { target, .. } = constraint;
            self.node(target)
                .map_err(|_| SceneError::InvalidConstraintTarget { node, target })?;
            if bindings.insert(node, constraint).is_some() {
                return Err(SceneError::DuplicateConstraint(node));
            }
        }
        if bindings.is_empty() {
            return self.evaluate_with_transforms(locals);
        }

        let mut worlds: HashMap<NodeHandle, AffineTransform> = HashMap::with_capacity(self.len());
        let mut visiting = HashMap::new();
        let mut path = Vec::new();
        let mut pending = Vec::new();
        let mut hierarchy = self.roots().collect::<Vec<_>>();
        while let Some(root) = hierarchy.pop() {
            hierarchy.extend(self.children(root)?);
            pending.push((root, false));
            while let Some((node, finish)) = pending.pop() {
                if worlds.contains_key(&node) {
                    continue;
                }
                let parent = self.parent(node)?;
                if finish {
                    let world = match bindings.get(&node) {
                        Some(TransformConstraint::Follow { target, offset }) => {
                            worlds[target].compose(*offset)
                        }
                        None => parent
                            .map_or(AffineTransform::IDENTITY, |parent| worlds[&parent])
                            .compose(
                                locals
                                    .get(&node)
                                    .copied()
                                    .unwrap_or(self.node(node)?.local_transform()),
                            ),
                    }
                    .map_err(|source| SceneError::InvalidTransform { node, source })?;
                    worlds.insert(node, world);
                    visiting.remove(&node);
                    path.pop();
                    continue;
                }
                if let Some(&start) = visiting.get(&node) {
                    let mut cycle = path[start..].to_vec();
                    cycle.push(node);
                    return Err(SceneError::ConstraintCycle(cycle));
                }
                visiting.insert(node, path.len());
                path.push(node);
                pending.push((node, true));
                if let Some(TransformConstraint::Follow { target, .. }) = bindings.get(&node) {
                    pending.push((*target, false));
                }
                if let Some(parent) = parent {
                    pending.push((parent, false));
                }
            }
        }
        self.evaluate_using(|node, _| Ok(worlds[&node]))
    }
}
