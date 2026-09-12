use crate::Scene3dVertexAttribute;
use anyhow::{Result, ensure};
#[cfg(all(test, not(target_family = "wasm")))]
mod tests;

#[derive(Clone, Copy)]
pub(in super::super::super) struct Expansion {
    mode: u32,
    amount: f32,
    weight: u32,
    limit: f32,
}

impl Expansion {
    pub fn new(
        attributes: &[Scene3dVertexAttribute],
        expansion: Option<&gpui::MeshPassExpansion3d>,
    ) -> Result<Self> {
        let Some(expansion) = expansion else {
            return Ok(Self {
                mode: 0,
                amount: 0.,
                weight: u32::MAX,
                limit: 1.,
            });
        };
        ensure!(expansion.is_valid(), "invalid mesh pass expansion");
        let weight = if let Some(name) = &expansion.weight_attribute {
            let index = attributes
                .iter()
                .position(|attribute| attribute.name == name.as_ref())
                .ok_or_else(|| anyhow::anyhow!("unknown mesh pass width attribute {name}"))?;
            ensure!(
                attributes[index].format == wgpu::VertexFormat::Float32,
                "mesh pass width attribute must be Float32"
            );
            index as u32
        } else {
            u32::MAX
        };
        Ok(Self {
            mode: expansion.space as u32,
            amount: expansion.amount,
            weight,
            limit: expansion.weight_limit,
        })
    }

    pub fn key(self) -> [u32; 4] {
        [
            self.mode,
            self.amount.to_bits(),
            self.weight,
            self.limit.to_bits(),
        ]
    }

    pub fn constants(self) -> [(&'static str, f64); 4] {
        [
            ("mesh_pass_expansion_mode", self.mode as f64),
            ("mesh_pass_expansion_amount", f64::from(self.amount)),
            ("mesh_pass_weight_index", self.weight as f64),
            ("mesh_pass_weight_limit", f64::from(self.limit)),
        ]
    }
}
