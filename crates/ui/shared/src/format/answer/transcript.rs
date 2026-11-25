//! An answer as it streams in, as edits to the end of the text: lines that ended are rendered
//! once and never touched again, and only the line still coming in is rendered again with each
//! batch. A token costs the text view the length of its own line, not of the whole answer.

use super::markdown::{LinePart, Renderer, Span, Style};

/// Replace the text from `from` (in UTF-16 code units, as `NSString` counts) to its end with
/// `spans`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub from: usize,
    pub spans: Vec<Span>,
}

/// What came of the answer so far, and where its last line starts.
#[derive(Clone, Debug, Default)]
pub struct Transcript {
    renderer: Renderer,
    /// The line still coming in.
    line: Vec<LinePart>,
    /// Where the line still coming in starts in the text.
    committed: usize,
}

impl Transcript {
    /// Takes a batch of pieces in the order they came; returns the edit that shows them.
    pub fn push(&mut self, pieces: impl IntoIterator<Item = LinePart>) -> Edit {
        let from = self.committed;
        let mut spans = Vec::new();
        for piece in pieces {
            match piece {
                LinePart::Cite(quote) => self.line.push(LinePart::Cite(quote)),
                LinePart::Text(text) => {
                    let mut lines = text.split('\n');
                    if let Some(first) = lines.next() {
                        self.add(first);
                    }
                    for next in lines {
                        let ended = self.renderer.line(&self.line, true);
                        self.committed += utf16(&ended) + 1;
                        spans.extend(ended);
                        spans.push(Span::new("\n", Style::Body));
                        self.line.clear();
                        self.add(next);
                    }
                }
            }
        }
        spans.extend(self.renderer.line(&self.line, false));
        Edit { from, spans }
    }

    /// Appends `text` to the line, joining the text before it.
    fn add(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if let Some(LinePart::Text(last)) = self.line.last_mut() {
            last.push_str(text);
        } else {
            self.line.push(LinePart::Text(text.to_owned()));
        }
    }
}

/// The length of `spans` in UTF-16 code units.
#[must_use]
pub fn utf16(spans: &[Span]) -> usize {
    spans
        .iter()
        .map(|span| span.text.encode_utf16().count())
        .sum()
}

#[cfg(test)]
mod tests {
    use super::super::markdown::{LinePart, Span, Style};
    use super::{Edit, Transcript, utf16};

    fn text(text: &str) -> LinePart {
        LinePart::Text(text.to_owned())
    }

    fn body(text: &str) -> Span {
        Span::new(text, Style::Body)
    }

    /// The text an edit leaves behind, applied to `shown`.
    fn apply(shown: &mut Vec<u16>, edit: &Edit) {
        shown.truncate(edit.from);
        for span in &edit.spans {
            shown.extend(span.text.encode_utf16());
        }
    }

    #[test]
    fn tokens_of_one_line_rewrite_only_that_line() {
        let mut transcript = Transcript::default();

        assert_eq!(
            transcript.push([text("Hello")]),
            Edit {
                from: 0,
                spans: vec![body("Hello")]
            }
        );
        assert_eq!(
            transcript.push([text(", world")]),
            Edit {
                from: 0,
                spans: vec![body("Hello, world")]
            }
        );
    }

    #[test]
    fn an_ended_line_is_never_edited_again() {
        let mut transcript = Transcript::default();
        transcript.push([text("**One**\nTw")]);

        let edit = transcript.push([text("o")]);

        assert_eq!(edit.from, 4, "“One” and its line break");
        assert_eq!(edit.spans, [body("Two")]);
    }

    #[test]
    fn a_line_completes_its_marks_when_they_close() {
        let mut transcript = Transcript::default();
        transcript.push([text("a **b")]);

        let edit = transcript.push([text("** c")]);

        assert_eq!(edit.spans[1].style, Style::Strong);
    }

    #[test]
    fn the_edits_add_up_to_the_whole_answer() {
        let mut transcript = Transcript::default();
        let mut shown = Vec::new();
        let batches = [
            vec![text("# Ti"), text("tle\n- a")],
            vec![LinePart::Cite("x y z w".to_owned()), text("😀\n\n")],
            vec![text("end")],
        ];

        for batch in batches {
            let edit = transcript.push(batch);
            apply(&mut shown, &edit);
        }

        assert_eq!(
            String::from_utf16(&shown).unwrap(),
            "Title\n• a“x y z w”😀\n\nend"
        );
    }

    #[test]
    fn offsets_count_utf16_code_units() {
        let mut transcript = Transcript::default();
        transcript.push([text("😀é\n")]);

        assert_eq!(transcript.push([text("x")]).from, 4);
        assert_eq!(utf16(&[body("😀é")]), 3);
    }

    #[test]
    fn an_empty_batch_rewrites_the_open_line_only() {
        let mut transcript = Transcript::default();
        transcript.push([text("a\nb")]);

        assert_eq!(
            transcript.push([]),
            Edit {
                from: 2,
                spans: vec![body("b")]
            }
        );
    }
}
