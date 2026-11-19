//! FSD `shared` layer of Quickie's UI: design tokens, glass surfaces by elevation, and motion.
//! No domain knowledge lives here; see `docs/design/system.md` for every value.

#![expect(
    unsafe_code,
    reason = "AppKit and Core Animation calls that objc2 marks unsafe: constants, paths, masks"
)]

pub mod glass;
pub mod motion;
pub mod tokens;

pub use glass::{Elevation, GlassSurface, Scrim};
pub use motion::{Motion, Trigger};
