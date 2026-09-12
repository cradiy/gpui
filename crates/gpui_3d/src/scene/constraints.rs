use super::{EvaluatedScene, NodeHandle, SceneError, SceneGraph};
use crate::{AffineTransform, AimError, AimSettings, AimStatus, Mesh};
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
    /// Aims the node's parent-composed local pose at a point in the target's local space.
    /// Target and parent transforms use their final constrained poses.
    Aim {
        target: NodeHandle,
        target_offset: [f32; 3],
        settings: AimSettings,
    },
}

impl TransformConstraint {
    fn target(self) -> NodeHandle {
        match self {
            Self::Follow { target, .. } | Self::Aim { target, .. } => target,
        }
    }
}

/// Per-node outcome retained with an evaluated snapshot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ConstraintStatus {
    Follow,
    Aim(AimStatus),
}

impl SceneGraph {
    /// Evaluates local pose overrides and world constraints without mutating the graph.
    /// Each node accepts at most one constraint. Follow replaces its world transform;
    /// Aim rotates its parent-composed local pose. Targets use final transforms
    /// regardless of input order. Parent and target dependencies must be acyclic.
    /// Hidden nodes participate in evaluation; only hierarchy controls visibility.
    /// Omitting a constraint restores the node's authored or supplied local transform.
    pub fn evaluate_with_constraints(
        &self,
        transforms: impl IntoIterator<Item = (NodeHandle, AffineTransform)>,
        constraints: impl IntoIterator<Item = (NodeHandle, TransformConstraint)>,
    ) -> Result<EvaluatedScene, SceneError> {
        self.evaluate_with_constraints_and_meshes(transforms, constraints, [])
    }

    /// Evaluates local transforms, world constraints, and replacement mesh resources.
    /// Meshes use their node's final constrained world transform for rendering,
    /// bounds, and queries. Constraint outcomes are retained in the snapshot.
    /// Mesh targets must contain geometry; duplicate, foreign, expired, and
    /// non-mesh targets are rejected as in `evaluate_with_overrides`.
    /// The graph, its revision, and previous snapshots remain unchanged.
    pub fn evaluate_with_constraints_and_meshes(
        &self,
        transforms: impl IntoIterator<Item = (NodeHandle, AffineTransform)>,
        constraints: impl IntoIterator<Item = (NodeHandle, TransformConstraint)>,
        meshes: impl IntoIterator<Item = (NodeHandle, Mesh)>,
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
            let target = constraint.target();
            self.node(target)
                .map_err(|_| SceneError::InvalidConstraintTarget { node, target })?;
            if bindings.insert(node, constraint).is_some() {
                return Err(SceneError::DuplicateConstraint(node));
            }
        }
        if bindings.is_empty() {
            return self.evaluate_with_overrides(locals, meshes);
        }

        let mut worlds: HashMap<NodeHandle, AffineTransform> = HashMap::with_capacity(self.len());
        let mut statuses = HashMap::with_capacity(bindings.len());
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
                    let local_world = || {
                        parent
                            .map_or(AffineTransform::IDENTITY, |parent| worlds[&parent])
                            .compose(
                                locals
                                    .get(&node)
                                    .copied()
                                    .unwrap_or(self.node(node)?.local_transform()),
                            )
                            .map_err(|source| SceneError::InvalidTransform { node, source })
                    };
                    let world = match bindings.get(&node) {
                        Some(TransformConstraint::Follow { target, offset }) => {
                            statuses.insert(node, ConstraintStatus::Follow);
                            worlds[target]
                                .compose(*offset)
                                .map_err(|source| SceneError::InvalidTransform { node, source })?
                        }
                        Some(TransformConstraint::Aim {
                            target,
                            target_offset,
                            settings,
                        }) => {
                            if !target_offset.iter().all(|value| value.is_finite()) {
                                return Err(SceneError::InvalidAim {
                                    node,
                                    source: AimError::InvalidTarget,
                                });
                            }
                            let matrix = worlds[target].matrix();
                            let target = std::array::from_fn(|r| {
                                ((0..3)
                                    .map(|c| f64::from(matrix[c][r]) * f64::from(target_offset[c]))
                                    .sum::<f64>()
                                    + f64::from(matrix[3][r]))
                                    as f32
                            });
                            if !target.iter().all(|value| value.is_finite()) {
                                return Err(SceneError::InvalidAim {
                                    node,
                                    source: AimError::Unrepresentable,
                                });
                            }
                            let aimed = settings
                                .solve(local_world()?, target)
                                .map_err(|source| SceneError::InvalidAim { node, source })?;
                            statuses.insert(node, ConstraintStatus::Aim(aimed.status));
                            aimed.transform
                        }
                        None => local_world()?,
                    };
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
                if let Some(constraint) = bindings.get(&node) {
                    pending.push((constraint.target(), false));
                }
                if let Some(parent) = parent {
                    pending.push((parent, false));
                }
            }
        }
        let mut evaluated = self.evaluate_using(meshes, |node, _| Ok(worlds[&node]))?;
        evaluated.constraint_status = statuses;
        Ok(evaluated)
    }
}
