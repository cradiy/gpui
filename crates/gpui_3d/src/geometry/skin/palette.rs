use super::{AffineTransform, Arc, Skin, SkinError};

/// Immutable mesh-local joint matrices in inverse-bind order. Clones share the
/// matrices and joint-binding identity; no vertices or GPU resources are retained.
#[derive(Clone, Debug)]
pub struct SkinPalette {
    inverse_bind: Arc<[AffineTransform]>,
    matrices: Arc<[[[f32; 4]; 4]]>,
}

impl SkinPalette {
    /// Column-major matrices, including mesh-world cancellation and inverse binds.
    pub fn matrices(&self) -> &[[[f32; 4]; 4]] {
        &self.matrices
    }

    pub(in crate::geometry) fn validate_binding(&self, skin: &Skin) -> Result<(), SkinError> {
        if !Arc::ptr_eq(&self.inverse_bind, &skin.inverse_bind) {
            return Err(SkinError::PaletteBindingMismatch);
        }
        Ok(())
    }
}

impl Skin {
    /// Composes `inverse(mesh_world) * joint_world * inverse_bind` on the CPU,
    /// validating every joint before returning an immutable palette. It is reusable
    /// by this binding, its clones and vertex remaps, independently of vertex input.
    /// No vertex blending, upload or GPU work occurs. Blend singularities and
    /// unrepresentable vertex results are checked during evaluation.
    pub fn palette(
        &self,
        mesh_world: AffineTransform,
        joint_world: &[AffineTransform],
    ) -> Result<SkinPalette, SkinError> {
        if joint_world.len() != self.joint_count() {
            return Err(SkinError::JointCount {
                expected: self.joint_count(),
                actual: joint_world.len(),
            });
        }
        let world_to_mesh = mesh_world.inverse();
        let matrices = joint_world
            .iter()
            .zip(self.inverse_bind.iter())
            .enumerate()
            .map(|(joint, (&world, &bind))| {
                world_to_mesh
                    .compose(world)
                    .and_then(|local| local.compose(bind))
                    .map(AffineTransform::matrix)
                    .map_err(|_| SkinError::InvalidJointTransform { joint })
            })
            .collect::<Result<Arc<[_]>, _>>()?;
        Ok(SkinPalette {
            inverse_bind: self.inverse_bind.clone(),
            matrices,
        })
    }
}
