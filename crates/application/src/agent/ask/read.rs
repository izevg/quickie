//! Reading the pages an ask is about: which tabs it reads, then each page's text, asked of the
//! engine at once and collected by ticket as the answers come.

use quickie_domain_agents::{PageScope, PageText};
use quickie_domain_browsing::{Session, Tab, TabId, WindowId};
use url::Url;

use crate::{PageContent, Ticket};

/// The pages one ask reads, in the order the prompt lists them, until every text is in.
///
/// [`Self::start`] asks the engine for each page's text; the texts come later, as
/// [`EngineEvent::Text`](crate::EngineEvent::Text) after the engine's waker ran, and
/// [`Self::text_read`] collects them by their ticket. Once [`Self::is_complete`],
/// [`Self::into_pages`] gives them to [`Agent::ask`](crate::Agent::ask).
#[derive(Debug)]
pub struct PageReader {
    pages: Vec<Reading>,
}

/// One page of a [`PageReader`].
#[derive(Debug)]
struct Reading {
    url: Url,
    title: String,
    /// The engine's request for the page's text, while it is on its way.
    ticket: Option<Ticket>,
    text: Option<String>,
}

impl PageReader {
    /// The most text read from one page, in characters. The prompt's token budget cuts it much
    /// shorter; this bounds what crosses from the engine.
    pub const MAX_CHARS: usize = 200_000;

    /// The tabs a request of `scope` reads in `window`, the active one first: the active tab
    /// alone, or the window's open tabs, up to the scope's most. Only a tab whose page exists
    /// (`has_page`) has text to read; one without is left out and does not count.
    #[must_use]
    pub fn tabs(
        session: &Session,
        window: WindowId,
        scope: PageScope,
        has_page: impl Fn(TabId) -> bool,
    ) -> Vec<Tab> {
        let active = session.active(window);
        let mut tabs: Vec<&Tab> = session
            .tabs(window)
            .map_or_else(|_| Vec::new(), Iterator::collect);
        tabs.sort_by_key(|tab| Some(tab.id) != active);
        tabs.into_iter()
            .filter(|tab| scope == PageScope::OpenTabs || Some(tab.id) == active)
            .filter(|tab| has_page(tab.id))
            .take(scope.max_pages())
            .cloned()
            .collect()
    }

    /// Asks `content` for the text of each of `tabs`, at most [`Self::MAX_CHARS`] each, in
    /// their order. A page the engine cannot read reads as empty at once.
    pub fn start(content: &mut impl PageContent, tabs: &[Tab]) -> Self {
        let pages = tabs
            .iter()
            .map(|tab| {
                let ticket = content.read_text(tab.id, Self::MAX_CHARS).ok();
                Reading {
                    url: tab.url.clone(),
                    title: tab.title.clone(),
                    ticket,
                    text: ticket.map_or_else(|| Some(String::new()), |_| None),
                }
            })
            .collect();
        Self { pages }
    }

    /// A page's text arrived, answering `ticket`; false when it answers none of this reader's
    /// requests, such as one of an earlier ask's.
    pub fn text_read(&mut self, ticket: Ticket, text: String) -> bool {
        let Some(page) = self
            .pages
            .iter_mut()
            .find(|page| page.ticket == Some(ticket))
        else {
            return false;
        };
        page.ticket = None;
        page.text = Some(text);
        true
    }

    /// True once every page's text is in.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.pages.iter().all(|page| page.text.is_some())
    }

    /// The pages read, in order, each with its text; a text still on its way reads as empty.
    #[must_use]
    pub fn into_pages(self) -> Vec<PageText> {
        self.pages
            .into_iter()
            .map(|page| PageText::new(page.url, page.title, page.text.unwrap_or_default()))
            .collect()
    }
}
