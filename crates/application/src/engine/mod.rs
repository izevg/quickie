//! The engine as the application sees it: the values pages report and are asked with, owned
//! here so that no engine adapter's types reach the shell. Each adapter converts to and from
//! them; the tokens it mints ([`ReplyToken`], [`DownloadToken`], [`Ticket`]) are opaque to
//! everyone else. Everything a page reports arrives as one [`EngineEvent`] stream, which each
//! adapter keeps in an [`EventQueue`]. With the `conformance` feature, the `conformance` module
//! checks that an engine behaves as the application expects.

#[cfg(any(test, feature = "conformance"))]
pub mod conformance;
mod event;
mod queue;
#[cfg(test)]
mod tests;

pub use event::{
    Answer, Ask, AskKind, Dialog, DownloadEvent, DownloadToken, EngineEvent, FindStep, Followed,
    Found, LinkTarget, PageRect, ReplyToken, Ticket,
};
pub use queue::EventQueue;
