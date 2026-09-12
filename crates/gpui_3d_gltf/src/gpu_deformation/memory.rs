use super::*;

/// Payload for one imported scene evaluation, excluding resident sources, CPU
/// data, render packing, readbacks, driver overhead and other retained samples.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GpuSceneEvaluationMemory {
    /// Conservative sum of newly allocated weights, palettes, intermediate and
    /// output buffers across all stages; not a measurement of peak residency.
    pub evaluation_bytes: u64,
    /// Vertex buffers returned to the caller, including reused bind-space outputs.
    /// Counted per primitive, independently of whether an allocation is new.
    pub output_bytes: u64,
}

impl GpuSceneDeformation {
    /// Inspects the stages selected by authored/default and overridden weights
    /// without GPU allocation or submission. Validates mapped primitive and weight
    /// targets; pose matrices and device health are checked during evaluation.
    pub fn evaluation_memory(
        &self,
        instance: &SubtreeInstance,
        weights: &[(NodeHandle, Vec<f32>)],
    ) -> Result<GpuSceneEvaluationMemory> {
        for source in &self.primitives {
            ensure!(
                instance.node(source.primitive.handle).is_some(),
                "primitive is absent from the instance"
            );
        }
        self.memory_for_weights(&self.resolve_weights(instance, weights)?)
    }

    pub(super) fn memory_for_weights(
        &self,
        weights: &HashMap<NodeHandle, &[f32]>,
    ) -> Result<GpuSceneEvaluationMemory> {
        self.primitives
            .iter()
            .try_fold(GpuSceneEvaluationMemory::default(), |mut total, source| {
                let mut evaluation = 0_u64;
                let mut output = 0;
                let mut add = |bytes: u64| -> Result<()> {
                    evaluation = evaluation
                        .checked_add(bytes)
                        .context("GPU scene evaluation payload overflow")?;
                    Ok(())
                };
                if let Some(morph) = &source.morph {
                    let changed = weights[&source.primitive.handle]
                        .iter()
                        .any(|weight| *weight != 0.);
                    let memory = morph.gpu.memory();
                    output = memory.output_bytes;
                    if changed || morph.tangents.is_none() {
                        add(memory.weight_bytes)?;
                        add(memory.output_bytes)?;
                    }
                    if changed {
                        if let Some(normals) = &morph.normals {
                            add(normals.memory().output_bytes)?;
                        }
                        if let Some((tangents, _)) = &morph.tangents {
                            add(tangents.memory().evaluation_bytes)?;
                        }
                    }
                }
                if let Some((_, skin)) = &source.skin {
                    let memory = skin.memory();
                    add(memory.palette_bytes)?;
                    add(memory.output_bytes)?;
                    output = memory.output_bytes;
                }
                total.evaluation_bytes = total
                    .evaluation_bytes
                    .checked_add(evaluation)
                    .context("GPU scene evaluation payload overflow")?;
                total.output_bytes = total
                    .output_bytes
                    .checked_add(output)
                    .context("GPU scene output payload overflow")?;
                Ok(total)
            })
    }
}
