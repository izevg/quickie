//! Settings context: what the user prefers, kept per profile.
//!
//! - [`SiteZooms`]: the page zoom of each site the user zoomed, in Safari's steps
//!   ([`ZoomLevel`]), 50 % to 300 %. A site never zoomed, or zoomed back to 100 %, is not
//!   listed.
//! - [`InstalledExtensions`]: the web extensions the user installed in the main profile, what
//!   each may do ([`Grants`]), the sites the user allowed or blocked each on
//!   ([`SiteAccess`]), and whether updates come on their own.
//!
//! Zoom is a preference about how a site is shown, not a protection, so it lives here and not
//! with the shields' per-site policies; both key on the kernel's [`Site`].
//!
//! Extensions live here too, not with the shields: an extension is software the user chose to
//! add to the profile (a password manager as much as an ad blocker), and what it may do is the
//! user's grant, not a protection policy. Their per-site access keys on the same [`Site`].
//!
//! Each preference is an aggregate the store saves in one transaction, through a plain snapshot
//! that comes back through a validated `restore`. Pure: no I/O, no async.

mod extensions;
mod zoom;

pub use extensions::{
    CorruptExtensions, ExtensionError, ExtensionId, ExtensionIdError, ExtensionSnapshot,
    ExtensionSource, ExtensionVersion, Grants, Installed, InstalledExtension, InstalledExtensions,
    InstalledExtensionsSnapshot, Package, SiteAccess, VersionError, site_patterns,
};
pub use quickie_domain_kernel::{Site, SiteError, Timestamp};
pub use zoom::{CorruptZooms, SiteZooms, SiteZoomsSnapshot, ZoomLevel, ZoomStep};
