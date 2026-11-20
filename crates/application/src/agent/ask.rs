//! Asking about pages, in two steps: reading them ([`PageReader`], on the main thread, through
//! the engine's [`PageContent`](crate::PageContent)), then answering ([`Asking`], on a worker
//! thread, through the [`LlmPort`](crate::LlmPort)).

mod answer;
mod read;

pub(super) use answer::key_of;
pub use answer::{AnswerEvent, AskError, Asking};
pub use read::PageReader;
