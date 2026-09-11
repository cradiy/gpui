use std::{
    hash::{Hash, Hasher},
    sync::Arc,
};

/// Opaque process-local frame identity. Clones compare equal without retaining GPU resources.
/// Default creates a fresh token; tokens carry neither ordering nor completion information.
#[derive(Clone, Debug, Default)]
pub struct Scene3dFrameId(Arc<()>);

impl PartialEq for Scene3dFrameId {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for Scene3dFrameId {}
impl Hash for Scene3dFrameId {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Arc::as_ptr(&self.0).hash(state);
    }
}
