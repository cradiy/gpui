use std::{collections::HashMap, sync::Arc, time::Duration};

use anyhow::{Context, Result, ensure};
use gltf::{
    Accessor,
    accessor::{DataType, Dimensions},
    animation::{Property, util::ReadOutputs},
};
use gpui_3d::{
    Interpolation, Keyframe, RotationTrack, TransformPose, TransformTrack, VectorTrack, WeightTrack,
};

use crate::PreparedDocument;

/// Aggregate admission for one clip. Shared samplers are charged per channel.
#[derive(Clone, Copy, Debug)]
pub struct AnimationOptions {
    pub channel_limit: usize,
    pub keyframe_limit: usize,
    /// Retained f32 values and derivatives, including zero derivatives in noncubic tracks.
    pub scalar_limit: usize,
}

impl Default for AnimationOptions {
    fn default() -> Self {
        Self {
            channel_limit: 100_000,
            keyframe_limit: 1_048_576,
            scalar_limit: 16_777_216,
        }
    }
}

/// Source-node tracks. Missing transform channels retain the node's authored TRS.
/// Weights remain separate inputs to deformation and are not clamped.
#[derive(Clone, Debug)]
pub struct NodeAnimation {
    node_index: usize,
    transform: Option<TransformTrack>,
    weights: Option<WeightTrack>,
}

impl NodeAnimation {
    pub fn node_index(&self) -> usize {
        self.node_index
    }
    pub fn transform(&self) -> Option<&TransformTrack> {
        self.transform.as_ref()
    }
    pub fn weights(&self) -> Option<&WeightTrack> {
        self.weights.as_ref()
    }
}

/// Immutable CPU animation data with document-local identities and absolute times.
/// Clones share tracks; no graph, clock, playback policy, or GPU resources are owned.
#[derive(Clone, Debug)]
pub struct AnimationClip {
    index: usize,
    name: Option<Arc<str>>,
    nodes: Arc<[NodeAnimation]>,
    start: Duration,
    end: Duration,
}

impl AnimationClip {
    pub fn index(&self) -> usize {
        self.index
    }
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }
    /// Unique targets in first-channel order, independent of scene selection.
    pub fn nodes(&self) -> &[NodeAnimation] {
        &self.nodes
    }
    /// First authored key across channels; times are not shifted to zero.
    pub fn start(&self) -> Duration {
        self.start
    }
    pub fn end(&self) -> Duration {
        self.end
    }
    pub fn duration(&self) -> Duration {
        self.end - self.start
    }
}

impl PreparedDocument {
    /// Converts all channels in a document animation without evaluating a scene.
    /// Errors include the clip and channel index. Conversion is retryable.
    pub fn animation(&self, index: usize, options: AnimationOptions) -> Result<AnimationClip> {
        self.animation_clip(index, options)
            .with_context(|| format!("animation {index}"))
    }

    fn animation_clip(&self, index: usize, options: AnimationOptions) -> Result<AnimationClip> {
        crate::validation::supported_extensions(self.gltf())?;
        let animation = self
            .gltf()
            .animations()
            .nth(index)
            .context("animation index out of range")?;
        let count = self.gltf().as_json().animations[index].channels.len();
        ensure!(
            count > 0 && count <= options.channel_limit,
            "channel count {count} must be nonzero and within channel limit"
        );
        let mut nodes: Vec<NodeAnimation> = Vec::new();
        let mut targets = HashMap::<usize, (usize, u8)>::new();
        let mut keys_left = options.keyframe_limit;
        let mut scalars_left = options.scalar_limit;
        let mut start = Duration::MAX;
        let mut end = Duration::ZERO;
        for channel in animation.channels() {
            let convert = (|| -> Result<()> {
                let target = channel.target();
                let source = target.node();
                let source_index = source.index();
                let property = target.property();
                let bit = match property {
                    Property::Translation => 1,
                    Property::Rotation => 2,
                    Property::Scale => 4,
                    Property::MorphTargetWeights => 8,
                };
                let (node_index, mask) = targets.entry(source_index).or_insert_with(|| {
                    let node_index = nodes.len();
                    nodes.push(NodeAnimation {
                        node_index: source_index,
                        transform: None,
                        weights: None,
                    });
                    (node_index, 0)
                });
                ensure!(
                    *mask & bit == 0,
                    "node {source_index}: duplicate {property:?} target"
                );
                *mask |= bit;
                ensure!(
                    self.gltf().as_json().nodes[source_index].matrix.is_none(),
                    "node {source_index}: animated nodes must use TRS, not matrix"
                );
                let sampler = channel.sampler();
                let interpolation = match sampler.interpolation() {
                    gltf::animation::Interpolation::Step => Interpolation::Step,
                    gltf::animation::Interpolation::Linear => Interpolation::Linear,
                    gltf::animation::Interpolation::CubicSpline => Interpolation::CubicSpline,
                };
                let input = sampler.input();
                let output = sampler.output();
                ensure!(
                    float(&input, Dimensions::Scalar),
                    "input accessor {} must contain float scalars",
                    input.index()
                );
                let count = input.count();
                let cubic = interpolation == Interpolation::CubicSpline;
                ensure!(
                    count >= if cubic { 2 } else { 1 },
                    "insufficient keyframes for {interpolation:?}"
                );
                let components = match property {
                    Property::Translation | Property::Scale => {
                        ensure!(
                            float(&output, Dimensions::Vec3),
                            "output accessor {} must contain float VEC3 values",
                            output.index()
                        );
                        3
                    }
                    Property::Rotation => {
                        ensure!(
                            normalized_or_float(&output, Dimensions::Vec4),
                            "output accessor {} must contain float or normalized VEC4 values",
                            output.index()
                        );
                        4
                    }
                    Property::MorphTargetWeights => {
                        ensure!(
                            normalized_or_float(&output, Dimensions::Scalar),
                            "output accessor {} must contain float or normalized scalars",
                            output.index()
                        );
                        morph_count(&source)?
                    }
                };
                let output_count = count
                    .checked_mul(if cubic { 3 } else { 1 })
                    .and_then(|count| {
                        count.checked_mul(if property == Property::MorphTargetWeights {
                            components
                        } else {
                            1
                        })
                    })
                    .context("output count overflow")?;
                ensure!(
                    output.count() == output_count,
                    "output accessor {} has {} elements; expected {output_count}",
                    output.index(),
                    output.count()
                );
                keys_left = keys_left
                    .checked_sub(count)
                    .context("keyframe limit exceeded")?;
                let scalar_count = count
                    .checked_mul(components)
                    .and_then(|count| count.checked_mul(3))
                    .context("scalar count overflow")?;
                scalars_left = scalars_left
                    .checked_sub(scalar_count)
                    .context("scalar limit exceeded")?;
                let reader = channel.reader(|buffer| self.buffer(buffer.index()));
                let mut times = Vec::with_capacity(count);
                let mut previous = None;
                for (key, seconds) in read(&input, reader.read_inputs())?.enumerate() {
                    ensure!(
                        seconds.is_finite()
                            && seconds >= 0.
                            && previous.is_none_or(|previous| seconds > previous),
                        "keyframe {key}: times must be finite, nonnegative, and strictly increasing"
                    );
                    let time = Duration::try_from_secs_f32(seconds)
                        .with_context(|| format!("keyframe {key}: time exceeds Duration range"))?;
                    ensure!(
                        times.last().is_none_or(|previous| time > *previous),
                        "keyframe {key}: times collapse at Duration precision"
                    );
                    times.push(time);
                    previous = Some(seconds);
                }
                ensure!(times.len() == count, "input accessor length mismatch");
                start = start.min(times[0]);
                end = end.max(times[count - 1]);
                let node = &mut nodes[*node_index];
                if property != Property::MorphTargetWeights && node.transform.is_none() {
                    let (translation, rotation, scale) = source.transform().decomposed();
                    node.transform = Some(TransformTrack::new(TransformPose {
                        translation,
                        rotation,
                        scale,
                    })?);
                }
                let outputs = reader.read_outputs();
                match property {
                    Property::Translation | Property::Scale => {
                        let values = match outputs {
                            Some(
                                ReadOutputs::Translations(values) | ReadOutputs::Scales(values),
                            ) => Some(values),
                            None => None,
                            _ => anyhow::bail!("unexpected output property"),
                        };
                        let keys = keys(&times, cubic, read(&output, values)?.map(Ok))?;
                        if property == Property::Scale {
                            ensure!(
                                keys.iter()
                                    .all(|key| key.value.iter().all(|value| *value != 0.)),
                                "scale keyframes must be invertible"
                            );
                        }
                        let track = VectorTrack::new(keys, interpolation)?;
                        node.transform = node.transform.take().map(|transform| {
                            if property == Property::Translation {
                                transform.translation(track)
                            } else {
                                transform.scale(track)
                            }
                        });
                    }
                    Property::Rotation => {
                        let values = match outputs {
                            Some(ReadOutputs::Rotations(values)) => Some(values.into_f32()),
                            None => None,
                            _ => anyhow::bail!("unexpected output property"),
                        };
                        let track = RotationTrack::new(
                            keys(&times, cubic, read(&output, values)?.map(Ok))?,
                            interpolation,
                        )?;
                        node.transform = node
                            .transform
                            .take()
                            .map(|transform| transform.rotation(track));
                    }
                    Property::MorphTargetWeights => {
                        let values = match outputs {
                            Some(ReadOutputs::MorphTargetWeights(values)) => {
                                Some(values.into_f32())
                            }
                            None => None,
                            _ => anyhow::bail!("unexpected output property"),
                        };
                        let mut values = read(&output, values)?;
                        let groups = std::iter::from_fn(move || {
                            let first = values.next()?;
                            let mut group = Vec::with_capacity(components);
                            group.push(first);
                            for _ in 1..components {
                                match values.next() {
                                    Some(value) => group.push(value),
                                    None => {
                                        return Some(Err(anyhow::anyhow!(
                                            "incomplete weight vector"
                                        )));
                                    }
                                }
                            }
                            Some(Ok(group))
                        });
                        node.weights = Some(WeightTrack::new(
                            keys(&times, cubic, groups)?,
                            interpolation,
                        )?);
                    }
                }
                Ok(())
            })();
            convert.with_context(|| format!("channel {}", channel.index()))?;
        }
        Ok(AnimationClip {
            index,
            name: animation.name().map(Arc::from),
            nodes: nodes.into(),
            start,
            end,
        })
    }
}

fn float(accessor: &Accessor<'_>, dimensions: Dimensions) -> bool {
    accessor.dimensions() == dimensions
        && accessor.data_type() == DataType::F32
        && !accessor.normalized()
}

fn read<'a, T: Default + 'a>(
    accessor: &Accessor<'_>,
    values: Option<impl Iterator<Item = T> + 'a>,
) -> Result<Box<dyn Iterator<Item = T> + 'a>> {
    match values {
        Some(values) => Ok(Box::new(values)),
        None if accessor.view().is_none() && accessor.sparse().is_none() => Ok(Box::new(
            std::iter::repeat_with(T::default).take(accessor.count()),
        )),
        None => anyhow::bail!("accessor {} is unavailable", accessor.index()),
    }
}

fn normalized_or_float(accessor: &Accessor<'_>, dimensions: Dimensions) -> bool {
    accessor.dimensions() == dimensions
        && match accessor.data_type() {
            DataType::F32 => !accessor.normalized(),
            DataType::I8 | DataType::U8 | DataType::I16 | DataType::U16 => accessor.normalized(),
            _ => false,
        }
}

fn morph_count(node: &gltf::Node<'_>) -> Result<usize> {
    let mesh = node.mesh().context("weight target has no mesh")?;
    let mut counts = mesh
        .primitives()
        .map(|primitive| primitive.morph_targets().count());
    let count = counts.next().context("weight target has no primitives")?;
    ensure!(
        count > 0 && counts.all(|other| other == count),
        "weight target primitives must have the same nonzero morph count"
    );
    for weights in [mesh.weights(), node.weights()].into_iter().flatten() {
        ensure!(
            weights.len() == count && weights.iter().all(|value| value.is_finite()),
            "invalid default morph weights"
        );
    }
    Ok(count)
}

fn keys<T: Default>(
    times: &[Duration],
    cubic: bool,
    mut values: impl Iterator<Item = Result<T>>,
) -> Result<Vec<Keyframe<T>>> {
    let mut result = Vec::with_capacity(times.len());
    for &time in times {
        let first = values.next().context("missing keyframe value")??;
        let key = if cubic {
            let value = values.next().context("missing cubic value")??;
            let outgoing = values.next().context("missing cubic derivative")??;
            Keyframe::new(time, value).tangents(first, outgoing)
        } else {
            Keyframe::new(time, first)
        };
        result.push(key);
    }
    ensure!(
        values.next().is_none(),
        "unexpected trailing keyframe values"
    );
    Ok(result)
}
