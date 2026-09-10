use super::AnimationClip;
use crate::SceneInstance;
use anyhow::{Context, Result, ensure};
use gpui_3d::{NodeHandle, Pose};
use std::{sync::Arc, time::Duration};

/// Treatment of clip targets outside the selected scene instance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnimationTargetPolicy {
    RequireAll,
    SkipMissing,
}

/// Shared clip tracks mapped to one instance's graph-scoped handles.
/// No graph, mesh resources, playback clock, or previous samples are retained.
#[derive(Clone, Debug)]
pub struct BoundAnimation {
    clip: AnimationClip,
    nodes: Arc<[(NodeHandle, usize)]>,
    missing: Arc<[usize]>,
}

/// Owned local TRS and Morph weight inputs from one absolute-time sample.
#[derive(Clone, Debug, Default)]
pub struct AnimationSample {
    pose: Pose,
    weights: Vec<(NodeHandle, Vec<f32>)>,
}

impl AnimationSample {
    pub fn into_parts(self) -> (Pose, Vec<(NodeHandle, Vec<f32>)>) {
        (self.pose, self.weights)
    }

    pub fn pose(&self) -> &Pose {
        &self.pose
    }

    /// Destination glTF node groups, not renderable primitive children.
    /// Only animated weight targets are included; omitted weights use asset defaults.
    pub fn weights(&self) -> &[(NodeHandle, Vec<f32>)] {
        &self.weights
    }
}

impl AnimationClip {
    /// Maps tracks once, preserving first-channel order and authored base TRS.
    /// Clip and asset must originate from the same parsed document. Independent
    /// parses have different identities, even for identical bytes.
    /// Graph edits do not update bindings; evaluation validates handle liveness.
    pub fn bind(
        &self,
        instance: &SceneInstance,
        policy: AnimationTargetPolicy,
    ) -> Result<BoundAnimation> {
        ensure!(
            Arc::ptr_eq(&self.source, &instance.asset().source),
            "animation {} and scene instance originate from different documents",
            self.index()
        );
        let mut nodes = Vec::new();
        let mut missing = Vec::new();
        for (index, source) in self.nodes().iter().enumerate() {
            if let Some(handle) = instance.node(source.node_index()) {
                nodes.push((handle, index));
            } else {
                ensure!(
                    policy == AnimationTargetPolicy::SkipMissing,
                    "animation {} target node {} is outside the scene instance",
                    self.index(),
                    source.node_index()
                );
                missing.push(source.node_index());
            }
        }
        Ok(BoundAnimation {
            clip: self.clone(),
            nodes: nodes.into(),
            missing: missing.into(),
        })
    }
}

impl BoundAnimation {
    pub fn clip(&self) -> &AnimationClip {
        &self.clip
    }

    /// Original node indices excluded by `SkipMissing`, in first-channel order.
    pub fn missing_nodes(&self) -> &[usize] {
        &self.missing
    }

    /// Samples authored absolute time, independently of prior calls.
    /// Tracks clamp to their endpoints. Looping, rate, mixing and constraints
    /// remain caller-owned. Failure returns no partial sample.
    pub fn sample(&self, time: Duration) -> Result<AnimationSample> {
        let mut locals = Vec::new();
        let mut weights = Vec::new();
        for &(node, index) in self.nodes.iter() {
            let source = &self.clip.nodes()[index];
            let sample = (|| -> Result<()> {
                if let Some(track) = source.transform() {
                    locals.push((node, track.sample(time)?));
                }
                if let Some(track) = source.weights() {
                    weights.push((node, track.sample(time)?));
                }
                Ok(())
            })();
            sample.with_context(|| {
                format!(
                    "animation {} node {}",
                    self.clip.index(),
                    source.node_index()
                )
            })?;
        }
        Ok(AnimationSample {
            pose: Pose::new(locals)?,
            weights,
        })
    }
}
