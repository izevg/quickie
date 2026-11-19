//! FSD `shared` layer of Quickie's UI: design tokens, glass surfaces by elevation, motion, and
//! the views every overlay is made of: keycaps, pills, hint bars, the list, and toggle rows;
//! and how sizes and ages read. No domain knowledge lives here; see `docs/design/system.md` for
//! every value.

#![expect(
    unsafe_code,
    reason = "AppKit and Core Animation calls that objc2 marks unsafe: constants, paths, masks, \
              and the list's table data source"
)]

pub mod controls;
pub mod form;
pub mod format;
pub mod glass;
pub mod list;
pub mod motion;
pub mod tokens;

pub use controls::{Pill, StyledText, hint_bar, keycap_group, symbol};
pub use form::ToggleRow;
pub use glass::{Elevation, GlassSurface, Scrim};
pub use list::{ListRow, ListView};
pub use motion::{Motion, Trigger};
