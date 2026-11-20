//! Form controls: the rows popovers and settings set things with, the group a standard window
//! puts them in, and the plate under groups, keycaps, and cards.

pub mod group;
pub mod plate;
pub mod toggle_row;

pub use group::FormGroup;
pub use plate::{Paint, Plate};
pub use toggle_row::ToggleRow;
