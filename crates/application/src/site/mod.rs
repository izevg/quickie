//! What the user chose for a site: how its pages are zoomed, and how its pages' dialogs and
//! device requests are answered. Each is a policy keyed by where a page comes from (its
//! registrable domain or its origin), not by the tab that shows it.

mod prompts;
mod zoom;

pub use prompts::{
    DialogAnswer, DialogGate, Permission, PermissionAnswer, PermissionGate, Prompts, Waiting,
};
pub use zoom::Zoom;
