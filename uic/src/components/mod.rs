//! UIC component building blocks.
//!
//! Public renderable components follow GPUI's [`gpui::Styled`] model. Their
//! outer layout and visual style use normal style methods, and text styling is
//! inherited through the component tree. Appearance types describe interaction
//! states and internal geometry that belong to the component itself.

pub mod badge;
pub mod calendar;
pub mod color_picker;
pub mod context_menu;
pub mod dropdown;
pub mod input;
pub mod modal;
pub mod notification;
pub mod popover;
pub mod progress;
mod range;
pub mod scrollbar;
pub mod selection;
pub mod slider;
pub mod toast;
pub mod tree_picker;
pub mod ui;

pub use ui::space;
