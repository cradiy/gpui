use anyhow::{Result, ensure};
use std::sync::{Arc, OnceLock};
use wgpu::naga::{self, Expression, Function, Handle, Module, Statement};

const CORE: &str = include_str!("scene3d.wgsl");
const DEFAULT: &str = include_str!("scene3d_material.wgsl");
const MAX_SOURCE_BYTES: usize = 64 * 1024;
const SURFACE_HELPERS: &[&str] = &["builtin_surface", "unit_vector"];
const SHADING_HELPERS: &[&str] = &[
    "builtin_surface",
    "builtin_shading",
    "surface_normal",
    "material_view_direction",
    "material_light_count",
    "material_light",
    "material_ambient",
    "diffuse_environment",
    "unit_vector",
];

/// CPU-validated mesh shader source with renderer-owned entry points and coverage.
pub(crate) struct MaterialProgram {
    source: Arc<str>,
}

impl MaterialProgram {
    pub(crate) fn builtin() -> &'static Self {
        static PROGRAM: OnceLock<MaterialProgram> = OnceLock::new();
        PROGRAM.get_or_init(|| Self::compile(DEFAULT).expect("invalid built-in mesh material"))
    }

    pub(crate) fn source(&self) -> &str {
        &self.source
    }

    fn compile(material: &str) -> Result<Self> {
        ensure!(
            material.len() <= MAX_SOURCE_BYTES,
            "material source exceeds byte limit"
        );
        let source = format!("{CORE}\n{material}");
        let module = naga::front::wgsl::parse_str(&source)
            .map_err(|error| anyhow::anyhow!("{}", error.emit_to_string(&source)))?;
        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .map_err(|error| anyhow::anyhow!("{error:#}"))?;
        ensure!(
            module.overrides.is_empty(),
            "material overrides are not supported"
        );
        for (handle, _) in module.global_variables.iter() {
            ensure!(
                module
                    .global_variables
                    .get_span(handle)
                    .to_range()
                    .is_some_and(|s| s.start < CORE.len()),
                "material globals and resource declarations are not supported"
            );
        }
        ensure!(
            module.entry_points.len() == 7
                && module.entry_points.iter().all(|entry| {
                    matches!(
                        (entry.name.as_str(), entry.stage),
                        ("vertex" | "shadow_vertex", naga::ShaderStage::Vertex)
                            | (
                                "fragment"
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
                    .filter(|s| s.start >= CORE.len())
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
                    !matches!(expression, Expression::GlobalVariable(_)),
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
                        !function
                            .expressions
                            .iter()
                            .any(|(_, e)| matches!(e, Expression::Derivative { .. })),
                        "surface coverage must use supplied gradients"
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
mod tests;
