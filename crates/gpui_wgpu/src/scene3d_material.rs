use anyhow::{Result, ensure};
use std::sync::{Arc, OnceLock};
use wgpu::naga::{self, Expression, Function, Handle, Module, Statement};
mod attributes;
#[cfg(not(target_family = "wasm"))]
mod bindings;
mod resources;
pub use attributes::{Scene3dVertexAttribute, Scene3dVertexInterpolation};
#[cfg(not(target_family = "wasm"))]
pub use bindings::{
    Scene3dMaterialBindingLimits, Scene3dMaterialSnapshot, Scene3dMaterialSource,
    Scene3dMaterialValue, Scene3dVertexStreamValue, Scene3dVertexStreams,
};
pub use resources::{Scene3dMaterialLimits, Scene3dMaterialResource, Scene3dMaterialResourceKind};

const CORE: &str = include_str!("scene3d.wgsl");
const DEFAULT: &str = include_str!("scene3d_material.wgsl");
const SURFACE_HELPERS: &[&str] = &["builtin_surface", "unit_vector"];
const SHADING_HELPERS: &[&str] = &[
    "builtin_surface",
    "builtin_shading",
    "surface_normal",
    "material_factors",
    "material_view_direction",
    "material_view_position",
    "material_view_vector",
    "material_light_count",
    "material_light",
    "material_ambient",
    "material_environment_radiance",
    "material_environment_brdf",
    "diffuse_environment",
    "unit_vector",
];

/// Compiled WGSL and reflected material bindings. Compilation performs no GPU work
/// and does not attach the program to a scene material or create rendering pipelines.
#[derive(Clone, Debug)]
pub struct MaterialProgram {
    source: Arc<str>,
    resources: Arc<[Scene3dMaterialResource]>,
    vertex_attributes: Arc<[Scene3dVertexAttribute]>,
}

impl MaterialProgram {
    pub(crate) fn builtin() -> &'static Self {
        static PROGRAM: OnceLock<MaterialProgram> = OnceLock::new();
        PROGRAM.get_or_init(|| Self::compile(DEFAULT).expect("invalid built-in mesh material"))
    }

    /// Complete renderer and material WGSL, retaining renderer-owned output entry points.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Validates source and reflects group 1 resources under default CPU admission limits.
    /// Accepts material_surface and material_shading functions, with optional helpers.
    pub fn compile(material: &str) -> Result<Self> {
        Self::compile_with_limits(material, Scene3dMaterialLimits::default())
    }

    /// Resources in ascending binding order, including declarations unused by either evaluator.
    pub fn resources(&self) -> &[Scene3dMaterialResource] {
        &self.resources
    }

    /// Checks enabled device limits including the renderer's standard material bindings.
    /// This does not validate actual texture formats, handles, or GPU shader compilation.
    pub fn validate_limits(&self, limits: &wgpu::Limits) -> Result<()> {
        resources::validate_limits(&self.resources, limits)?;
        attributes::validate_limits(&self.vertex_attributes, limits)
    }

    /// Compiles without creating an adapter or allocating GPU resources.
    pub fn compile_with_limits(material: &str, limits: Scene3dMaterialLimits) -> Result<Self> {
        Self::compile_with_attributes_and_limits(material, &[], limits)
    }

    /// Custom streams in declaration order. Each uses one vertex-visible group 2 binding.
    pub fn vertex_attributes(&self) -> &[Scene3dVertexAttribute] {
        &self.vertex_attributes
    }

    /// Compiles typed custom vertex inputs exposed as `SurfaceInput.attributes`.
    /// Streams use packed 32-bit scalar/vector records and hardware interpolation.
    pub fn compile_with_attributes(
        material: &str,
        attributes: &[Scene3dVertexAttribute],
    ) -> Result<Self> {
        Self::compile_with_attributes_and_limits(
            material,
            attributes,
            Scene3dMaterialLimits::default(),
        )
    }

    /// Compiles custom vertex inputs under explicit source and declaration budgets.
    pub fn compile_with_attributes_and_limits(
        material: &str,
        attributes: &[Scene3dVertexAttribute],
        limits: Scene3dMaterialLimits,
    ) -> Result<Self> {
        ensure!(
            material.len() <= limits.max_source_bytes,
            "material source exceeds byte limit"
        );
        let core = attributes::assemble(CORE, attributes, limits.max_vertex_attributes)?;
        let source = format!("{core}\n{material}");
        let module = naga::front::wgsl::parse_str(&source)
            .map_err(|error| anyhow::anyhow!("{}", error.emit_to_string(&source)))?;
        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .map_err(|error| anyhow::anyhow!("{error:#}"))?;
        ensure!(
            module.overrides.iter().all(|(handle, _)| {
                module
                    .overrides
                    .get_span(handle)
                    .to_range()
                    .is_some_and(|span| span.end <= core.len())
            }),
            "material overrides are not supported"
        );
        let resources = resources::reflect(&module, &info, limits, core.len())?;
        ensure!(
            module.entry_points.len() == 8
                && module.entry_points.iter().all(|entry| {
                    matches!(
                        (entry.name.as_str(), entry.stage),
                        ("vertex" | "shadow_vertex", naga::ShaderStage::Vertex)
                            | (
                                "fragment"
                                    | "mesh_pass_fragment"
                                    | "shadow_fragment"
                                    | "object_id"
                                    | "linear_depth"
                                    | "world_normal",
                                naga::ShaderStage::Fragment
                            )
                    )
                }),
            "material entry points are owned by the renderer"
        );
        let custom: Vec<_> = module
            .functions
            .iter()
            .filter_map(|(handle, _)| {
                module
                    .functions
                    .get_span(handle)
                    .to_range()
                    .filter(|s| s.start >= core.len())
                    .map(|_| handle)
            })
            .collect();
        for &handle in &custom {
            let function = &module.functions[handle];
            ensure!(
                !info[handle].may_kill
                    && !statements(&function.body)
                        .iter()
                        .any(|s| matches!(s, Statement::Kill)),
                "material function {:?} may discard",
                function.name
            );
            for (_, expression) in function.expressions.iter() {
                ensure!(
                    !matches!(expression, Expression::Override(_)),
                    "material function {:?} accesses a private renderer override",
                    function.name
                );
                ensure!(
                    !matches!(expression, Expression::GlobalVariable(global) if module.global_variables[*global].binding.as_ref().is_none_or(|b| b.group != 1)),
                    "material function {:?} accesses a private renderer global",
                    function.name
                );
            }
            for called in calls(&function.body) {
                let name = module.functions[called].name.as_deref().unwrap_or("");
                ensure!(
                    custom.contains(&called)
                        || SURFACE_HELPERS.contains(&name)
                        || SHADING_HELPERS.contains(&name),
                    "material function {:?} calls private renderer function {name}",
                    function.name
                );
            }
        }
        for (entry, reference, allowed) in [
            ("material_surface", "builtin_surface", SURFACE_HELPERS),
            ("material_shading", "builtin_shading", SHADING_HELPERS),
        ] {
            let root = named(&module, entry)?;
            ensure!(custom.contains(&root), "missing material function {entry}");
            let signature = &module.functions[named(&module, reference)?];
            let function = &module.functions[root];
            ensure!(
                function
                    .arguments
                    .iter()
                    .map(|a| a.ty)
                    .eq(signature.arguments.iter().map(|a| a.ty))
                    && function.result.as_ref().map(|r| r.ty)
                        == signature.result.as_ref().map(|r| r.ty),
                "invalid {entry} signature"
            );
            let mut pending = vec![root];
            let mut visited = Vec::new();
            while let Some(handle) = pending.pop() {
                if visited.contains(&handle) {
                    continue;
                }
                visited.push(handle);
                let function = &module.functions[handle];
                if entry == "material_surface" {
                    ensure!(
                        !function.expressions.iter().any(|(_, e)| matches!(
                            e,
                            Expression::Derivative { .. }
                                | Expression::ImageSample {
                                    level: naga::SampleLevel::Auto | naga::SampleLevel::Bias(_),
                                    ..
                                }
                        )),
                        "surface coverage must use supplied gradients or explicit texture levels"
                    );
                }
                for called in calls(&function.body) {
                    if custom.contains(&called) {
                        pending.push(called);
                    } else {
                        let name = module.functions[called].name.as_deref().unwrap_or("");
                        ensure!(allowed.contains(&name), "{entry} cannot call {name}");
                    }
                }
            }
        }
        Ok(Self {
            source: source.into(),
            resources: resources.into(),
            vertex_attributes: attributes.into(),
        })
    }
}

fn named(module: &Module, name: &str) -> Result<Handle<Function>> {
    module
        .functions
        .iter()
        .find(|(_, f)| f.name.as_deref() == Some(name))
        .map(|(h, _)| h)
        .ok_or_else(|| anyhow::anyhow!("missing material function {name}"))
}

fn calls(body: &naga::Block) -> Vec<Handle<Function>> {
    statements(body)
        .into_iter()
        .filter_map(|statement| {
            if let Statement::Call { function, .. } = statement {
                Some(*function)
            } else {
                None
            }
        })
        .collect()
}

fn statements(body: &naga::Block) -> Vec<&Statement> {
    let mut result = Vec::new();
    let mut pending = vec![body];
    while let Some(block) = pending.pop() {
        for statement in block.iter() {
            result.push(statement);
            match statement {
                Statement::Block(block) => pending.push(block),
                Statement::If { accept, reject, .. } => pending.extend([accept, reject]),
                Statement::Switch { cases, .. } => {
                    pending.extend(cases.iter().map(|case| &case.body))
                }
                Statement::Loop {
                    body, continuing, ..
                } => pending.extend([body, continuing]),
                _ => {}
            }
        }
    }
    result
}

#[cfg(test)]
mod resources_tests;
#[cfg(test)]
mod tests;
