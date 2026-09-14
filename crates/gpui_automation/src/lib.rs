//! Semantic inspection, actions and state waiting for GPUI applications.
//! Collection is enabled explicitly per window; no external transport is started.

mod locator;
mod session;

pub use gpui::{AutomationNode, AutomationSnapshot, Role};
pub use locator::{Locator, LookupError, Selector, Snapshot};
pub use session::{Session, WindowInfo};
