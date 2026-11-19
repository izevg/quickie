//! FSD `widgets` layer of Quickie's UI: the composed blocks that float above the page.
//!
//! Slices are grouped by meaning (`docs/design/inventory.md`, widgets): `palette`, `tabs`,
//! `feedback`. A slice never uses another, not even one in its own group; what two share moves
//! down to `quickie-ui-shared`.

#![expect(
    unsafe_code,
    reason = "AppKit through objc2: field selection and superviews, the palette field's \
              delegate, and the toast timers"
)]

pub mod feedback;
pub mod palette;
pub mod tabs;

pub use feedback::toast_stack::ToastStack;
pub use palette::address_overlay::AddressOverlay;
pub use palette::command_palette::CommandPalette;
pub use tabs::tab_switcher::TabSwitcher;
