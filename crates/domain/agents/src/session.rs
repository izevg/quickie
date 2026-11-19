//! One answer of the agent as it streams in: its text, the citations it quotes, and how fast it
//! came.

mod citation;
mod scan;
#[cfg(test)]
mod tests;

use std::fmt::Write;

use quickie_domain_kernel::Timestamp;

pub use citation::Citation;
use citation::Sources;
use scan::{Piece, Scanner};

use crate::Prompt;

/// How the agent works. Only Ask exists so far; Act, which acts in the agent profile under
/// approvals and the provenance rule, arrives later.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mode {
    /// Reads the pages the user gave it, on the user's explicit request, and answers. It
    /// cannot act.
    Ask,
}

/// A piece of the answer, ready for the answer panel, in the order it arrived.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AnswerPart {
    /// Answer text.
    Text(String),
    /// A quote the page contains, shown inline where it came.
    Citation(Citation),
}

/// How fast an answer came, for the readout under it: `24 tok/s · 0.4 s TTFT`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Speed {
    /// Milliseconds from the request to the first token; `None` when no token came.
    pub ttft_ms: Option<u64>,
    /// Tokens per second from the first token to the end; `None` when no time passed between
    /// them to measure.
    pub tokens_per_s: Option<f64>,
}

/// An answer that ended: what was still held back, and its speed.
#[derive(Clone, Debug, PartialEq)]
pub struct Finished {
    /// The last pieces of the answer, such as a mark that never closed.
    pub parts: Vec<AnswerPart>,
    /// How fast the answer came.
    pub speed: Speed,
    /// The whole answer as text, quotes in quotation marks, for copying.
    pub answer: String,
    /// Every citation, in order.
    pub citations: Vec<Citation>,
}

/// The agent answering one request in Ask mode. It turns the model's tokens into
/// [`AnswerPart`]s: text goes out at once, and a quote mark becomes a [`Citation`] if the pages
/// as sent contain the quote, or plain quoted text if they do not or it is too short to prove
/// anything.
#[derive(Clone, Debug)]
pub struct AgentSession {
    sources: Sources,
    scanner: Scanner,
    asked_at: Timestamp,
    first_token_at: Option<Timestamp>,
    tokens: u64,
    answer: String,
    citations: Vec<Citation>,
}

impl AgentSession {
    /// A session answering `prompt`, asked at `asked_at`. Its citations must quote the pages
    /// as the prompt sent them: never text past the budget the model did not see.
    #[must_use]
    pub fn new(prompt: &Prompt, asked_at: Timestamp) -> Self {
        Self {
            sources: Sources::of(prompt.sent_texts()),
            scanner: Scanner::default(),
            asked_at,
            first_token_at: None,
            tokens: 0,
            answer: String::new(),
            citations: Vec::new(),
        }
    }

    /// How the session works.
    #[must_use]
    pub fn mode(&self) -> Mode {
        Mode::Ask
    }

    /// The answer so far as text.
    #[must_use]
    pub fn answer(&self) -> &str {
        &self.answer
    }

    /// The citations so far.
    #[must_use]
    pub fn citations(&self) -> &[Citation] {
        &self.citations
    }

    /// Takes the next token, received at `at`; returns the parts it completes. Each call counts
    /// as one token, as OpenAI-compatible servers stream about one token per chunk; an empty
    /// token counts for nothing.
    pub fn received(&mut self, token: &str, at: Timestamp) -> Vec<AnswerPart> {
        if token.is_empty() {
            return Vec::new();
        }
        self.first_token_at.get_or_insert(at);
        self.tokens += 1;
        let pieces = self.scanner.push(token);
        pieces.into_iter().map(|piece| self.part(piece)).collect()
    }

    /// Ends the answer at `at`. `reported_tokens` is the server's own count of the tokens it
    /// generated, when it sent one; it is more exact than counting chunks.
    #[must_use]
    pub fn finish(mut self, at: Timestamp, reported_tokens: Option<u64>) -> Finished {
        let scanner = std::mem::take(&mut self.scanner);
        let parts = scanner
            .finish()
            .map(|piece| self.part(piece))
            .into_iter()
            .collect();
        let tokens = reported_tokens.unwrap_or(self.tokens);
        let speed = Speed {
            ttft_ms: self
                .first_token_at
                .map(|first| first.millis_since(self.asked_at)),
            tokens_per_s: self
                .first_token_at
                .map(|first| at.millis_since(first))
                .filter(|&millis| millis > 0)
                .map(|millis| per_second(tokens, millis)),
        };
        Finished {
            parts,
            speed,
            answer: self.answer,
            citations: self.citations,
        }
    }

    fn part(&mut self, piece: Piece) -> AnswerPart {
        match piece {
            Piece::Text(text) => {
                self.answer.push_str(&text);
                AnswerPart::Text(text)
            }
            Piece::Quote(quote) => {
                if let Some(citation) = self.sources.cite(&quote) {
                    let _ = write!(self.answer, "“{}”", citation.quote);
                    self.citations.push(citation.clone());
                    AnswerPart::Citation(citation)
                } else {
                    let text = format!("“{quote}”");
                    self.answer.push_str(&text);
                    AnswerPart::Text(text)
                }
            }
        }
    }
}

#[expect(
    clippy::cast_precision_loss,
    reason = "token counts and milliseconds stay far below 2^52, where f64 is exact"
)]
fn per_second(tokens: u64, millis: u64) -> f64 {
    tokens as f64 * 1000.0 / millis as f64
}
