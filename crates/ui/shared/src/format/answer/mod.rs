//! Safe, selectable answer text shared by conversation and answer surfaces.
//!
//! Markdown remains attributed text. Images never load, and only an explicit click can
//! deliver a web link to the owner; the renderer never opens it itself.

pub mod markdown;
pub mod text;
pub mod transcript;
