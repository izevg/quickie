//! Finds quote marks, `[[cite: …]]`, in an answer as it streams in token by token. A mark can
//! arrive split across any number of tokens, so the scanner holds back only what may still turn
//! into one, and lets every other character through at once.

/// Opens a quote mark.
const OPEN: &str = "[[cite:";
/// Closes a quote mark.
const CLOSE: &str = "]]";
/// The longest quote waited for, in bytes; an open mark that runs longer was no mark, and its
/// text goes out as it came.
const MAX_QUOTE_BYTES: usize = 2_000;

/// A piece of the answer between marks, or the quote inside one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Piece {
    /// Answer text.
    Text(String),
    /// What a mark quoted, trimmed.
    Quote(String),
}

/// The scanner's state between tokens.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Scanner {
    /// Text held back: inside a mark, or the start of what may become one.
    held: String,
    /// True inside an open mark.
    quoting: bool,
}

impl Scanner {
    /// Scans the next token; returns what is certain now.
    pub(super) fn push(&mut self, token: &str) -> Vec<Piece> {
        self.held.push_str(token);
        let mut pieces = Vec::new();
        loop {
            if self.quoting {
                let Some(end) = self.held.find(CLOSE) else {
                    if self.held.len() > MAX_QUOTE_BYTES {
                        let text = format!("{OPEN}{}", std::mem::take(&mut self.held));
                        pieces.push(Piece::Text(text));
                        self.quoting = false;
                    }
                    return pieces;
                };
                let quote = self.held[..end].trim().to_owned();
                self.held.replace_range(..end + CLOSE.len(), "");
                self.quoting = false;
                if !quote.is_empty() {
                    pieces.push(Piece::Quote(quote));
                }
            } else if let Some(start) = self.held.find(OPEN) {
                let text = self.held[..start].to_owned();
                self.held.replace_range(..start + OPEN.len(), "");
                self.quoting = true;
                push_text(&mut pieces, text);
            } else {
                let keep = partial_open(&self.held);
                let text = self.held[..self.held.len() - keep].to_owned();
                self.held.replace_range(..self.held.len() - keep, "");
                push_text(&mut pieces, text);
                return pieces;
            }
        }
    }

    /// The answer ended: whatever was held back is text after all, an unclosed mark included.
    pub(super) fn finish(self) -> Option<Piece> {
        let text = if self.quoting {
            format!("{OPEN}{}", self.held)
        } else {
            self.held
        };
        (!text.is_empty()).then_some(Piece::Text(text))
    }
}

fn push_text(pieces: &mut Vec<Piece>, text: String) {
    if !text.is_empty() {
        pieces.push(Piece::Text(text));
    }
}

/// The length of the longest end of `text` that is the start of [`OPEN`] without being all of
/// it: what must wait for the next token.
fn partial_open(text: &str) -> usize {
    (1..OPEN.len())
        .rev()
        .find(|&len| text.ends_with(&OPEN[..len]))
        .unwrap_or(0)
}
