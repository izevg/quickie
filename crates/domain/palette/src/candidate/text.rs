//! Candidate text the way the fuzzy matcher reads it. A title with one character beyond ASCII
//! (`·`, `—`, an accent) is matched by its characters, and cutting it into them costs more than
//! matching it; text kept in [`Texts`] is cut once, when the palette opens, instead of on every
//! keystroke.

use std::ops::Range;

use nucleo_matcher::{Utf32Str, chars};

/// The text of one candidate: a title or a detail, borrowed. Text kept in [`Texts`] comes
/// ready for matching; text made `From` a plain `&str` is cut into characters each time it is
/// matched.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Text<'a> {
    str: &'a str,
    form: Form<'a>,
}

/// How the matcher reads a text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Form<'a> {
    /// As its bytes: it is ASCII.
    Ascii,
    /// As these characters, one per grapheme, cut when it was kept.
    Chars(&'a [char]),
    /// As characters cut each time it is read.
    Uncut,
}

impl<'a> Text<'a> {
    /// The text as written.
    #[must_use]
    pub const fn as_str(self) -> &'a str {
        self.str
    }

    /// The text as the matcher reads it; `buf` holds the characters when they were not kept.
    pub(crate) fn haystack<'s>(self, buf: &'s mut Vec<char>) -> Utf32Str<'s>
    where
        'a: 's,
    {
        match self.form {
            Form::Ascii => Utf32Str::Ascii(self.str.as_bytes()),
            Form::Chars(kept) => Utf32Str::Unicode(kept),
            Form::Uncut => {
                buf.clear();
                cut(self.str, buf);
                Utf32Str::Unicode(buf)
            }
        }
    }
}

/// Appends the characters the matcher reads in `text`: the first of each grapheme. Most text
/// beyond ASCII is Latin, Greek, Cyrillic, punctuation, symbols, or CJK, where each character
/// is a grapheme of its own and segmenting would only cost time; text with a character that may
/// join its neighbours (a combining accent, a joiner, an Indic vowel sign, `\r\n`) is segmented.
fn cut(text: &str, into: &mut Vec<char>) {
    if !text.contains('\r') && text.chars().all(stands_alone) {
        into.extend(text.chars());
    } else {
        into.extend(chars::graphemes(text));
    }
}

/// Whether `c` is a grapheme of its own whatever its neighbours, as far as a quick check can
/// tell: true only within blocks that hold no combining, joining, or prepended character.
fn stands_alone(c: char) -> bool {
    c.is_ascii()
        || matches!(
            c,
            '\u{A0}'..='\u{2FF}'
                | '\u{370}'..='\u{482}'
                | '\u{48A}'..='\u{52F}'
                | '\u{2010}'..='\u{2027}'
                | '\u{2030}'..='\u{205E}'
                | '\u{20A0}'..='\u{20C0}'
                | '\u{2100}'..='\u{27BF}'
                | '\u{3000}'..='\u{3029}'
                | '\u{3030}'..='\u{303F}'
                | '\u{3041}'..='\u{3096}'
                | '\u{30A0}'..='\u{30FF}'
                | '\u{4E00}'..='\u{9FFF}'
                | '\u{FF01}'..='\u{FF9D}'
        )
}

impl<'a> From<&'a str> for Text<'a> {
    fn from(str: &'a str) -> Self {
        let form = if str.is_ascii() {
            Form::Ascii
        } else {
            Form::Uncut
        };
        Self { str, form }
    }
}

/// Where a text sits in [`Texts`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextSpan {
    utf8: Range<usize>,
    chars: Option<Range<usize>>,
}

/// The text of many candidates in two buffers, as written and as characters, so that keeping
/// fifty thousand of them allocates a few times, not fifty thousand.
#[derive(Clone, Debug, Default)]
pub struct Texts {
    utf8: String,
    chars: Vec<char>,
}

impl Texts {
    /// Keeps `text`, cut into characters unless it is ASCII, and tells where it is.
    pub fn keep(&mut self, text: &str) -> TextSpan {
        let start = self.utf8.len();
        self.utf8.push_str(text);
        let utf8 = start..self.utf8.len();
        let chars = (!text.is_ascii()).then(|| {
            let start = self.chars.len();
            cut(text, &mut self.chars);
            start..self.chars.len()
        });
        TextSpan { utf8, chars }
    }

    /// The text kept at `span`; empty for a span of other texts.
    #[must_use]
    pub fn get(&self, span: &TextSpan) -> Text<'_> {
        let str = self.utf8.get(span.utf8.clone()).unwrap_or_default();
        let form = match span.chars.clone() {
            None => Form::Ascii,
            Some(chars) => self.chars.get(chars).map_or(Form::Uncut, Form::Chars),
        };
        Text { str, form }
    }
}

#[cfg(test)]
mod tests {
    use nucleo_matcher::chars::graphemes;

    use super::{Form, Text, Texts, cut, stands_alone};

    fn cut_up(text: &str) -> Vec<char> {
        let mut chars = Vec::new();
        cut(text, &mut chars);
        chars
    }

    #[test]
    fn every_character_said_to_stand_alone_is_a_grapheme_of_its_own() {
        let standing: Vec<char> = ('\u{80}'..=char::MAX)
            .filter(|&c| stands_alone(c))
            .collect();

        let joined: Vec<char> = standing
            .iter()
            .copied()
            .filter(|&c| graphemes(&format!("a{c}{c}a")).count() != 4)
            .collect();

        assert!(standing.len() > 20_000);
        assert_eq!(joined, Vec::<char>::new());
    }

    #[test]
    fn a_combining_accent_joins_the_letter_before_it() {
        assert_eq!(cut_up("Cafe\u{301} ·"), ['C', 'a', 'f', 'e', ' ', '·']);
    }

    #[test]
    fn a_line_break_of_two_characters_is_one() {
        assert_eq!(cut_up("é\r\nx"), ['é', '\r', 'x']);
    }

    #[test]
    fn latin_punctuation_and_cjk_are_cut_into_their_characters() {
        assert_eq!(
            cut_up("Über · 東京"),
            ['Ü', 'b', 'e', 'r', ' ', '·', ' ', '東', '京']
        );
    }

    #[test]
    fn kept_texts_read_as_written() {
        let mut texts = Texts::default();

        let spans = ["Docs", "Über · uns", "", "rust"].map(|text| texts.keep(text));

        let read = spans.each_ref().map(|span| texts.get(span).as_str());
        assert_eq!(read, ["Docs", "Über · uns", "", "rust"]);
    }

    #[test]
    fn ascii_text_is_kept_as_written_only() {
        let mut texts = Texts::default();

        let span = texts.keep("Docs");

        assert_eq!(texts.get(&span), Text::from("Docs"));
    }

    #[test]
    fn text_beyond_ascii_is_kept_as_characters_too() {
        let mut texts = Texts::default();
        texts.keep("Docs");

        let span = texts.keep("Über");

        let kept = texts.get(&span);
        assert_eq!(kept.form, Form::Chars(&['Ü', 'b', 'e', 'r']));
    }

    #[test]
    fn plain_text_is_read_as_bytes_when_ascii_and_cut_when_not() {
        assert_eq!(Text::from("Docs").form, Form::Ascii);
        assert_eq!(Text::from("Über").form, Form::Uncut);
    }

    #[test]
    fn a_span_of_other_texts_reads_as_nothing() {
        let mut others = Texts::default();
        let span = others.keep("Über uns");
        let empty = Texts::default();

        let read = empty.get(&span);

        assert_eq!((read.as_str(), read.form), ("", Form::Uncut));
    }
}
