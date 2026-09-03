//! Independently composable disclosure panels and shared accordion state.

mod appearance;
mod collapsible;
mod state;

pub use appearance::CollapsibleAppearance;
pub use collapsible::{Collapsible, CollapsibleIndicatorPosition};
pub use state::{CollapsibleEvent, CollapsibleMode, CollapsibleState};
