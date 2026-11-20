//! The engine as the application sees it: the values pages report and are asked with, owned
//! here so that no engine adapter's types reach the shell. Each adapter converts to and from
//! them; the tokens it mints ([`ReplyToken`], [`DownloadToken`]) are opaque to everyone else.

mod event;
#[cfg(test)]
mod tests;

pub use event::{
    Answer, Ask, AskKind, Dialog, DownloadEvent, DownloadToken, FindStep, Followed, Found,
    LinkTarget, Navigated, PageRect, ReplyToken,
};
