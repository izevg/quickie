use super::{Block, Paragraph, Span, Style, document};

fn shown(spans: &[Span]) -> String {
    spans.iter().map(|span| span.text.as_str()).collect()
}

#[test]
fn list_items_have_hanging_geometry_and_nested_items_have_an_inset() {
    let spans = document("## Категории\n\n3. Игры\n   - Песочницы\n4. История");

    let paragraphs: Vec<_> = spans.iter().filter_map(|span| span.paragraph).collect();

    assert_eq!(
        paragraphs,
        [
            Paragraph {
                block: Block::Title,
                list_depth: 0,
                quote_depth: 0,
                marker: false
            },
            Paragraph {
                block: Block::Body,
                list_depth: 1,
                quote_depth: 0,
                marker: true
            },
            Paragraph {
                block: Block::Body,
                list_depth: 2,
                quote_depth: 0,
                marker: true
            },
            Paragraph {
                block: Block::Body,
                list_depth: 1,
                quote_depth: 0,
                marker: true
            },
        ]
    );
}

#[test]
fn web_hosts_are_disclosed_and_other_targets_are_inert() {
    let source = "[**docs**](https://example.test/abc) [script](javascript:alert(1))\n\n![*alt*](https://image.test/token) <img src=https://image.test/token>";

    let spans = document(source);
    let links: Vec<_> = spans
        .iter()
        .filter_map(|span| match &span.style {
            Style::Link(url) => Some((span.text.as_str(), url.as_str())),
            _ => None,
        })
        .collect();

    assert_eq!(links, [("docs", "https://example.test/abc")]);
    assert_eq!(
        shown(&spans),
        "docs (example.test) script\n[image: alt] <img src=https://image.test/token>"
    );
}

#[test]
fn multiline_code_preserves_whitespace_without_interpreting_markup() {
    let spans = document("Begin\n\n```text\n**literal**\n\n  x\n```\n\nEnd");

    assert_eq!(shown(&spans), "Begin\n**literal**\n\n  x\nEnd");
    assert!(
        spans
            .iter()
            .any(|span| { span.style == Style::Code && span.text == "**literal**\n\n  x" })
    );
}

#[test]
fn a_hard_break_stays_within_its_native_paragraph() {
    let spans = document("> One  \n> two\n\nNew paragraph");

    assert_eq!(shown(&spans), "One\u{2028}two\nNew paragraph");
    let quotes = spans
        .iter()
        .filter_map(|span| span.paragraph.map(|p| p.quote_depth));
    assert!(quotes.eq([1, 0]));
}
