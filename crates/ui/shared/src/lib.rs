//! FSD `shared` layer of Quickie's UI: design tokens, glass surfaces by elevation, motion, and
//! the views every overlay is made of: keycaps, pills, hint bars, the query field, the list,
//! toggle rows, the E3 sheet with its answer buttons, and the tab card of the switcher and the
//! overview; and how sizes and ages read. No domain knowledge lives here; see
//! `docs/design/system.md` for every value.

#![expect(
    unsafe_code,
    reason = "AppKit and Core Animation calls that objc2 marks unsafe: constants, paths, masks, \
              the list's table data source, the sheet's click target, and font weights"
)]

pub mod controls;
pub mod field;
pub mod form;
pub mod format;
pub mod glass;
pub mod list;
pub mod motion;
pub mod sheet;
#[cfg(feature = "stories")]
pub mod stories;
pub mod tab_card;
pub mod tokens;

pub use controls::{
    HintStrip, KeycapStyle, Pill, PillPart, PillTone, StyledText, hint_bar, hint_bar_active,
    keycap, keycap_group, keycap_group_styled, symbol,
};
pub use field::QueryField;
pub use form::{FormGroup, Plate, ToggleRow};
pub use glass::{Elevation, GlassSurface, Scrim, cast_shadow};
pub use list::{IconTint, ListRow, ListSection, ListView, RowIcon};
pub use motion::{Motion, Trigger};
pub use sheet::{
    Emphasis, Sheet, SheetAnswer, SheetCard, SheetContent, SheetIcon, SheetLine, header,
    keyed_button, named_header,
};
