//! Translation of dynamic WGSL effects to native renderer resource layouts.

use anyhow::{Result, anyhow, ensure};

#[cfg(test)]
mod tests;

/// Resource contract of a dynamic shader module.
#[derive(Clone, Copy, Debug)]
pub enum ShaderKind {
    /// An effect using zero, one, two or four image textures.
    Effect { image_count: u8 },
    /// A backdrop effect with raw and blurred scene textures.
    Backdrop,
    /// A blur pass with one scene texture.
    BackdropBlur,
}

impl ShaderKind {
    fn textures(self) -> Result<&'static [u32]> {
        match self {
            Self::Effect { image_count: 0 } => Ok(&[]),
            Self::Effect { image_count: 1 } | Self::BackdropBlur => Ok(&[1]),
            Self::Effect { image_count: 2 } | Self::Backdrop => Ok(&[1, 3]),
            Self::Effect { image_count: 4 } => Ok(&[1, 3, 4, 5]),
            Self::Effect { image_count } => {
                Err(anyhow!("unsupported effect image count: {image_count}"))
            }
        }
    }

    fn entries(self) -> [&'static str; 2] {
        match self {
            Self::Effect { .. } => ["vs_effect", "fs_effect"],
            Self::Backdrop => ["vs_backdrop", "fs_backdrop"],
            Self::BackdropBlur => ["vs_backdrop", "fs_blur"],
        }
    }
}

fn parse(source: &str, kind: ShaderKind) -> Result<(naga::Module, naga::valid::ModuleInfo)> {
    let module = naga::front::wgsl::parse_str(source)
        .map_err(|error| anyhow!("WGSL parse error: {}", error.emit_to_string(source)))?;
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .map_err(|error| anyhow!("WGSL validation error: {error}"))?;
    for (entry, stage) in kind
        .entries()
        .into_iter()
        .zip([naga::ShaderStage::Vertex, naga::ShaderStage::Fragment])
    {
        ensure!(
            module
                .entry_points
                .iter()
                .any(|e| e.name == entry && e.stage == stage),
            "missing {stage:?} entry point: {entry}"
        );
    }
    Ok((module, info))
}

/// Translates a module to MSL 2.0. Backdrop effects use hardware sampling.
pub fn to_msl(source: &str, kind: ShaderKind) -> Result<String> {
    let textures = kind.textures()?;
    let (module, info) = parse(source, kind)?;
    let mut resources = naga::back::msl::EntryPointResources::default();
    for (group, buffer) in [(0, 0), (1, 1)] {
        resources.resources.insert(
            naga::ResourceBinding { group, binding: 0 },
            naga::back::msl::BindTarget {
                buffer: Some(buffer),
                ..Default::default()
            },
        );
    }
    for (index, &binding) in textures.iter().enumerate() {
        resources.resources.insert(
            naga::ResourceBinding { group: 1, binding },
            naga::back::msl::BindTarget {
                texture: Some(index as u8),
                ..Default::default()
            },
        );
    }
    if matches!(kind, ShaderKind::Backdrop) {
        resources.resources.insert(
            naga::ResourceBinding {
                group: 1,
                binding: 2,
            },
            naga::back::msl::BindTarget {
                sampler: Some(naga::back::msl::BindSamplerTarget::Resource(0)),
                ..Default::default()
            },
        );
    }
    resources.sizes_buffer = Some(2);
    let mut options = naga::back::msl::Options {
        lang_version: (2, 0),
        fake_missing_bindings: false,
        ..Default::default()
    };
    for entry in &module.entry_points {
        options
            .per_entry_point_map
            .insert(entry.name.clone(), resources.clone());
    }
    let (output, reflection) = naga::back::msl::write_string(
        &module,
        &info,
        &options,
        &naga::back::msl::PipelineOptions::default(),
    )
    .map_err(|error| anyhow!("MSL generation error: {error}"))?;
    for entry in reflection.entry_point_names {
        entry.map_err(|error| anyhow!("MSL entry-point generation error: {error:?}"))?;
    }
    Ok(output)
}

/// Translates a module to HLSL Shader Model 5.0. Backdrops require manual sampling.
pub fn to_hlsl(source: &str, kind: ShaderKind) -> Result<String> {
    let textures = kind.textures()?;
    let (module, info) = parse(source, kind)?;
    let mut options = naga::back::hlsl::Options {
        shader_model: naga::back::hlsl::ShaderModel::V5_0,
        fake_missing_bindings: false,
        ..Default::default()
    };
    for (group, register) in [(0, 0), (1, 1)] {
        options.binding_map.insert(
            naga::ResourceBinding { group, binding: 0 },
            naga::back::hlsl::BindTarget {
                space: 0,
                register,
                ..Default::default()
            },
        );
    }
    for (index, &binding) in textures.iter().enumerate() {
        options.binding_map.insert(
            naga::ResourceBinding { group: 1, binding },
            naga::back::hlsl::BindTarget {
                space: 0,
                register: if index == 0 { 0 } else { index as u32 + 1 },
                ..Default::default()
            },
        );
    }
    let pipeline_options = naga::back::hlsl::PipelineOptions::default();
    let mut output = String::new();
    let reflection = naga::back::hlsl::Writer::new(&mut output, &options, &pipeline_options)
        .write(&module, &info, None)
        .map_err(|error| anyhow!("HLSL generation error: {error}"))?;
    for entry in reflection.entry_point_names {
        entry.map_err(|error| anyhow!("HLSL entry-point generation error: {error}"))?;
    }
    Ok(output)
}
