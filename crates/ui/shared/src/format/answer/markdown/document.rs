//! A completed answer as structured, native text. The parser produces events only.

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use url::Url;

use super::{Span, Style};

#[cfg(test)]
mod tests;

/// Parses a completed answer with `CommonMark` block and inline structure.
///
/// Raw HTML stays visible text, and images become their description. Only an HTTP or HTTPS
/// link written in the Markdown can be clicked, with its real host shown next to it.
#[must_use]
pub fn document(markdown: &str) -> Vec<Span> {
    let mut document = Document::default();
    for event in Parser::new_ext(markdown, Options::ENABLE_TABLES) {
        document.event(event);
    }
    document.spans
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Block {
    #[default]
    Body,
    Title,
    Heading,
    Code,
    TableRow,
    Rule,
}

/// Native geometry for the paragraph that starts at a span. The following spans share it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Paragraph {
    pub block: Block,
    pub list_depth: u8,
    pub quote_depth: u8,
    pub marker: bool,
}

#[derive(Default)]
struct Document {
    spans: Vec<Span>,
    block: Block,
    paragraph_open: bool,
    lists: Vec<List>,
    quote_depth: usize,
    strong: usize,
    emphasis: usize,
    link: Option<Link>,
    image: Option<Image>,
    code: String,
    table_cells: usize,
}

struct List {
    next: Option<u64>,
    marker: Option<String>,
}

struct Link {
    address: String,
    url: Option<Url>,
    start: usize,
}

#[derive(Default)]
struct Image {
    alt: String,
    nested: usize,
}

impl Document {
    fn event(&mut self, event: Event<'_>) {
        if let Some(image) = &mut self.image {
            match event {
                Event::Start(Tag::Image { .. }) => image.nested += 1,
                Event::End(TagEnd::Image) if image.nested == 0 => {
                    let alt = image.alt.trim();
                    let label = if alt.is_empty() {
                        "[image]".to_owned()
                    } else {
                        format!("[image: {alt}]")
                    };
                    self.image = None;
                    self.emit(&label, Style::Note);
                }
                Event::End(TagEnd::Image) => image.nested -= 1,
                Event::Text(value) | Event::Code(value) | Event::InlineHtml(value) => {
                    image.alt.push_str(&value);
                }
                Event::SoftBreak | Event::HardBreak => image.alt.push(' '),
                _ => {}
            }
            return;
        }
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) if self.block == Block::Code => self.code.push_str(&text),
            Event::Text(text) | Event::Html(text) | Event::InlineHtml(text) => {
                self.emit(&text, self.inline_style());
            }
            Event::Code(text) => self.emit(&text, Style::Code),
            Event::SoftBreak => self.emit(" ", self.inline_style()),
            Event::HardBreak => self.emit("\u{2028}", Style::Body),
            Event::Rule => {
                self.begin(Block::Rule);
                self.emit("────────────────", Style::Rule);
                self.begin(Block::Body);
            }
            _ => {}
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph | Tag::HtmlBlock => self.begin(Block::Body),
            Tag::Heading { level, .. } => self.begin(match level {
                HeadingLevel::H1 | HeadingLevel::H2 => Block::Title,
                _ => Block::Heading,
            }),
            Tag::BlockQuote(_) => {
                self.begin(Block::Body);
                self.quote_depth += 1;
            }
            Tag::CodeBlock(_) => self.begin(Block::Code),
            Tag::List(next) => {
                self.pending_marker();
                self.begin(Block::Body);
                self.lists.push(List { next, marker: None });
            }
            Tag::Item => {
                self.begin(Block::Body);
                if let Some(list) = self.lists.last_mut() {
                    list.marker = Some(
                        list.next
                            .map_or_else(|| "• ".to_owned(), |n| format!("{n}. ")),
                    );
                    list.next = list.next.map(|n| n.saturating_add(1));
                }
            }
            Tag::TableHead | Tag::TableRow => {
                self.begin(Block::TableRow);
                self.table_cells = 0;
            }
            Tag::TableCell => {
                if self.table_cells > 0 {
                    self.emit(" │ ", Style::Code);
                }
                self.table_cells += 1;
            }
            Tag::Strong => self.strong += 1,
            Tag::Emphasis => self.emphasis += 1,
            Tag::Image { .. } => self.image = Some(Image::default()),
            Tag::Link { dest_url, .. } => {
                self.link = Some(Link {
                    address: dest_url.to_string(),
                    url: web_url(&dest_url),
                    start: self.spans.len(),
                });
            }
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph
            | TagEnd::Heading(_)
            | TagEnd::TableHead
            | TagEnd::TableRow
            | TagEnd::HtmlBlock => self.begin(Block::Body),
            TagEnd::CodeBlock => {
                let code = std::mem::take(&mut self.code);
                self.emit(code.strip_suffix('\n').unwrap_or(&code), Style::Code);
                self.begin(Block::Body);
            }
            TagEnd::List(_) => {
                self.lists.pop();
                self.begin(Block::Body);
            }
            TagEnd::Item => {
                self.pending_marker();
                self.begin(Block::Body);
            }
            TagEnd::BlockQuote(_) => {
                self.quote_depth = self.quote_depth.saturating_sub(1);
                self.begin(Block::Body);
            }
            TagEnd::Strong => self.strong = self.strong.saturating_sub(1),
            TagEnd::Emphasis => self.emphasis = self.emphasis.saturating_sub(1),
            TagEnd::Link => {
                if let Some(link) = self.link.take() {
                    if link.start == self.spans.len() {
                        self.emit(
                            &link.address,
                            scoped_style(link.url.clone(), self.inline_style()),
                        );
                    }
                    if let Some(host) = link.url.as_ref().and_then(Url::host_str) {
                        self.emit(&format!(" ({host})"), Style::Note);
                    }
                }
            }
            _ => {}
        }
    }

    fn begin(&mut self, block: Block) {
        self.paragraph_open = false;
        self.block = block;
    }

    fn pending_marker(&mut self) {
        if self.lists.last().is_some_and(|list| list.marker.is_some()) {
            self.emit("", Style::Body);
        }
    }

    fn inline_style(&self) -> Style {
        if matches!(self.block, Block::Code | Block::TableRow) {
            return Style::Code;
        }
        match (self.strong != 0, self.emphasis != 0) {
            (true, true) => Style::StrongEmphasis,
            (true, false) => Style::Strong,
            (false, true) => Style::Emphasis,
            (false, false) => Style::Body,
        }
    }

    fn emit(&mut self, text: &str, style: Style) {
        if text.is_empty() && self.lists.last().is_none_or(|list| list.marker.is_none()) {
            return;
        }
        let scoped = self.link.as_ref().and_then(|link| link.url.clone());
        let mut span = Span::new(text, scoped_style(scoped, style));
        if !self.paragraph_open {
            if !self.spans.is_empty() {
                self.spans.push(Span::new("\n", Style::Body));
            }
            self.paragraph_open = true;
            let marker = self.lists.last_mut().and_then(|list| list.marker.take());
            let paragraph = Paragraph {
                block: self.block,
                list_depth: depth(self.lists.len()),
                quote_depth: depth(self.quote_depth),
                marker: marker.is_some(),
            };
            if let Some(marker) = marker {
                let mut marker = Span::new(marker, Style::Body);
                marker.paragraph = Some(paragraph);
                self.spans.push(marker);
            } else {
                span.paragraph = Some(paragraph);
            }
        }
        if !span.text.is_empty() {
            self.spans.push(span);
        }
    }
}

fn scoped_style(url: Option<Url>, style: Style) -> Style {
    url.map_or(style, Style::Link)
}

fn web_url(address: &str) -> Option<Url> {
    Url::parse(address)
        .ok()
        .filter(|url| matches!(url.scheme(), "http" | "https"))
}

fn depth(value: usize) -> u8 {
    u8::try_from(value.min(5)).unwrap_or(5)
}
