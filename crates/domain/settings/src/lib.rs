//! Settings context: what the user prefers, kept per profile.
//!
//! - [`SiteZooms`]: the page zoom of each site the user zoomed, in Safari's steps
//!   ([`ZoomLevel`]), 50 % to 300 %. A site never zoomed, or zoomed back to 100 %, is not
//!   listed.
//!
//! Zoom is a preference about how a site is shown, not a protection, so it lives here and not
//! with the shields' per-site policies; both key on the kernel's [`Site`].
//!
//! Each preference is an aggregate the store saves in one transaction, through a plain snapshot
//! that comes back through a validated `restore`. Pure: no I/O, no async.

mod zoom;

pub use quickie_domain_kernel::{Site, SiteError};
pub use zoom::{CorruptZooms, SiteZooms, SiteZoomsSnapshot, ZoomLevel, ZoomStep};
