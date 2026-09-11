use anyhow::{Result, ensure};
use std::fmt::Write as _;

#[cfg(test)]
mod tests;

/// Interpolation between a triangle's custom vertex values. Smooth values use pixel centers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scene3dVertexInterpolation {
    #[default]
    Perspective,
    Linear,
    /// The first vertex's value. Required for integer attributes.
    Flat,
}

/// A named, tightly packed custom vertex stream in mesh vertex order.
/// Supports `Float32`, `Sint32`, `Uint32`, and their x2/x3/x4 vector formats.
/// In particular, a three-component record occupies 12 bytes, without WGSL struct padding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scene3dVertexAttribute {
    pub name: String,
    pub format: wgpu::VertexFormat,
    pub interpolation: Scene3dVertexInterpolation,
}

impl Scene3dVertexAttribute {
    /// Floating-point formats default to perspective interpolation; integers default to flat.
    /// Compilation validates the name, format, interpolation, and declaration count.
    pub fn new(name: impl Into<String>, format: wgpu::VertexFormat) -> Self {
        let integer = matches!(
            format,
            wgpu::VertexFormat::Uint32
                | wgpu::VertexFormat::Uint32x2
                | wgpu::VertexFormat::Uint32x3
                | wgpu::VertexFormat::Uint32x4
                | wgpu::VertexFormat::Sint32
                | wgpu::VertexFormat::Sint32x2
                | wgpu::VertexFormat::Sint32x3
                | wgpu::VertexFormat::Sint32x4
        );
        Self {
            name: name.into(),
            format,
            interpolation: if integer {
                Scene3dVertexInterpolation::Flat
            } else {
                Scene3dVertexInterpolation::Perspective
            },
        }
    }

    pub fn interpolation(mut self, interpolation: Scene3dVertexInterpolation) -> Self {
        self.interpolation = interpolation;
        self
    }

    fn scalar_and_width(&self) -> Result<(&'static str, u32)> {
        use wgpu::VertexFormat::*;
        Ok(match self.format {
            Float32 => ("f32", 1),
            Float32x2 => ("f32", 2),
            Float32x3 => ("f32", 3),
            Float32x4 => ("f32", 4),
            Sint32 => ("i32", 1),
            Sint32x2 => ("i32", 2),
            Sint32x3 => ("i32", 3),
            Sint32x4 => ("i32", 4),
            Uint32 => ("u32", 1),
            Uint32x2 => ("u32", 2),
            Uint32x3 => ("u32", 3),
            Uint32x4 => ("u32", 4),
            _ => anyhow::bail!(
                "custom attribute {:?} requires a packed 32-bit scalar/vector format",
                self.name
            ),
        })
    }
}

pub(super) fn assemble(
    core: &str,
    attributes: &[Scene3dVertexAttribute],
    maximum: usize,
) -> Result<String> {
    ensure!(
        attributes.len() <= maximum,
        "custom vertex attribute count exceeds limit"
    );
    if attributes.is_empty() {
        return Ok(core.to_owned());
    }
    let mut declarations = String::from("struct MaterialAttributes {\n");
    let mut buffers = String::new();
    let mut fields = String::new();
    let mut interpolated = String::new();
    let mut loads = String::new();
    for (index, attribute) in attributes.iter().enumerate() {
        let name = &attribute.name;
        ensure!(
            name.len() <= 64
                && name.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
                && name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_'),
            "custom attribute name must start with an ASCII letter and contain at most 64 ASCII letters, digits, or underscores"
        );
        ensure!(
            !attributes[..index].iter().any(|a| a.name == *name),
            "duplicate custom attribute {name}"
        );
        let (scalar, width) = attribute.scalar_and_width()?;
        ensure!(
            scalar == "f32" || attribute.interpolation == Scene3dVertexInterpolation::Flat,
            "integer attribute {name} requires flat interpolation"
        );
        let ty = if width == 1 {
            scalar.to_string()
        } else {
            format!("vec{width}<{scalar}>")
        };
        let interpolation = match attribute.interpolation {
            Scene3dVertexInterpolation::Perspective => "perspective, center",
            Scene3dVertexInterpolation::Linear => "linear, center",
            Scene3dVertexInterpolation::Flat => "flat, first",
        };
        writeln!(declarations, "{name}: {ty},")?;
        writeln!(
            buffers,
            "@group(2) @binding({index}) var<storage, read> material_stream_{index}: array<u32>;"
        )?;
        writeln!(
            fields,
            "@location({}) @interpolate({interpolation}) material_attribute_{index}: {ty},",
            9 + index
        )?;
        writeln!(
            interpolated,
            "surface.attributes.{name} = input.material_attribute_{index};"
        )?;
        let components: Vec<_> = (0..width)
            .map(|component| {
                let word =
                    format!("material_stream_{index}[vertex_index * {width}u + {component}u]");
                if scalar == "u32" {
                    word
                } else {
                    format!("bitcast<{scalar}>({word})")
                }
            })
            .collect();
        let value = if width == 1 {
            components[0].clone()
        } else {
            format!("{ty}({})", components.join(", "))
        };
        writeln!(loads, "output.material_attribute_{index} = {value};")?;
    }
    declarations.push_str("}\n");
    declarations.push_str(&buffers);
    let mut source = core.to_owned();
    for (marker, replacement, occurrences) in [
        (
            "// MATERIAL_ATTRIBUTE_DECLARATIONS",
            declarations.as_str(),
            1,
        ),
        ("// MATERIAL_ATTRIBUTE_OUTPUT", fields.as_str(), 1),
        (
            "// MATERIAL_ATTRIBUTE_SURFACE",
            "attributes: MaterialAttributes,",
            1,
        ),
        (
            "// MATERIAL_ATTRIBUTE_INTERPOLATED",
            interpolated.as_str(),
            1,
        ),
        ("// MATERIAL_ATTRIBUTE_LOAD", loads.as_str(), 2),
        (
            "/* MATERIAL_ATTRIBUTE_INDEX */",
            "@builtin(vertex_index) vertex_index: u32,",
            2,
        ),
    ] {
        ensure!(
            source.matches(marker).count() == occurrences,
            "invalid renderer attribute template {marker}"
        );
        source = source.replace(marker, replacement);
    }
    Ok(source)
}

pub(super) fn validate_limits(
    attributes: &[Scene3dVertexAttribute],
    limits: &wgpu::Limits,
) -> Result<()> {
    if attributes.is_empty() {
        return Ok(());
    }
    let count = attributes.len() as u64;
    for (name, required, available) in [
        ("bind groups", 3, u64::from(limits.max_bind_groups)),
        (
            "vertex storage buffers",
            count,
            u64::from(limits.max_storage_buffers_per_shader_stage),
        ),
        (
            "vertex buffers and acceleration structures",
            count + 1,
            u64::from(limits.max_buffers_and_acceleration_structures_per_shader_stage),
        ),
        (
            "attribute bindings",
            count,
            u64::from(limits.max_bindings_per_bind_group),
        ),
        (
            "inter-stage variables",
            count + 9,
            u64::from(limits.max_inter_stage_shader_variables),
        ),
    ] {
        ensure!(
            required <= available,
            "material {name} require {required}, device enables {available}"
        );
    }
    Ok(())
}
