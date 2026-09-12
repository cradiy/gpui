use super::*;
use crate::Scene3dMaterialResource;

#[derive(Clone, Copy)]
pub(super) enum Input {
    Uniform(u64),
    Texture,
    Sampler,
}

pub(super) fn resolve(
    resources: &[Scene3dMaterialResource],
    inputs: impl IntoIterator<Item = (u32, Input)>,
    partial: bool,
    limits: Scene3dMaterialBindingLimits,
) -> Result<(Vec<Option<usize>>, u64)> {
    let mut mapping = vec![None; resources.len()];
    let mut uniform_bytes = 0u64;
    for resource in resources {
        if let Scene3dMaterialResourceKind::Uniform { min_size } = resource.kind {
            uniform_bytes = uniform_bytes
                .checked_add(min_size)
                .ok_or_else(|| anyhow::anyhow!("material uniform byte overflow"))?;
        }
    }
    ensure!(
        uniform_bytes <= limits.max_uniform_bytes,
        "material uniform snapshot exceeds byte budget"
    );
    for (input_index, (binding, input)) in inputs.into_iter().enumerate() {
        let index = resources
            .binary_search_by_key(&binding, |r| r.binding)
            .map_err(|_| anyhow::anyhow!("unknown material binding {binding}"))?;
        ensure!(
            mapping[index].is_none(),
            "duplicate material binding {binding}"
        );
        match (resources[index].kind, input) {
            (Scene3dMaterialResourceKind::Uniform { min_size }, Input::Uniform(size)) => {
                ensure!(
                    size == min_size,
                    "material uniform binding {binding} requires exactly {min_size} bytes, received {size}"
                );
            }
            (Scene3dMaterialResourceKind::Texture { .. }, Input::Texture)
            | (Scene3dMaterialResourceKind::Sampler, Input::Sampler) => {}
            _ => anyhow::bail!("material binding {binding} has an incompatible resource kind"),
        }
        mapping[index] = Some(input_index);
    }
    if !partial {
        for (resource, input) in resources.iter().zip(&mapping) {
            ensure!(
                input.is_some(),
                "missing material binding {}",
                resource.binding
            );
        }
    }
    Ok((mapping, uniform_bytes))
}
