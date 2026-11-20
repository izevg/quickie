//! The engine as the application sees it: the values pages report and are asked with, owned
//! here so that no engine adapter's types reach the shell, and the capability ports next to
//! [`EnginePort`](crate::EnginePort). Each adapter converts to and from the values; the tokens
//! it mints ([`ReplyToken`], [`DownloadToken`], [`Ticket`]) are opaque to everyone else.
//! Everything a page reports arrives as one [`EngineEvent`] stream, which each adapter keeps in
//! an [`EventQueue`]. With the `conformance` feature, the `conformance` module checks that an
//! engine behaves as the application expects.

#[cfg(any(test, feature = "conformance"))]
pub mod conformance;
mod event;
mod extensions;
mod queue;
#[cfg(test)]
mod tests;

use std::path::PathBuf;

use quickie_domain_browsing::TabId;
use quickie_domain_settings::ZoomLevel;
use url::Url;

pub use event::{
    Answer, Ask, AskKind, Dialog, DownloadEvent, DownloadToken, EngineEvent, FindStep, Followed,
    Found, LinkTarget, PageRect, ReplyToken, Ticket,
};
pub use extensions::{
    Access, Expected, ExtensionCommand, ExtensionEvent, ExtensionHost, ExtensionInfo,
    ExtensionPackages, PackageSource, Unpacked, WindowTabs,
};
pub use queue::EventQueue;

use crate::EngineError;

/// What an engine can do that the UI only degrades without, declared once per engine as a
/// `const` (`CAPS` on the adapter's engine type), so the shell decides at compile time what to
/// offer. A capability a use case cannot work without is a trait bound instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capabilities {
    /// Web extensions run in the main profile's pages ([`ExtensionHost`]).
    pub web_extensions: bool,
}

/// What is on a tab's page, as the user sees it: the matches of a find, the links and controls
/// on screen for link hints, and the page's text for the agent.
///
/// Every answer comes later, as an [`EngineEvent`] drained after the engine's waker ran, never
/// inside the call: a find as [`EngineEvent::Found`]; link targets, a followed link, and text
/// as [`EngineEvent::Targets`], [`EngineEvent::Followed`], and [`EngineEvent::Text`], each with
/// the [`Ticket`] the call returned. How the engine gets at the page (a script in an isolated
/// world, a native find) is its own business.
pub trait PageContent {
    /// Selects a match of `query` in the tab's page as `step` says, marks every match, and
    /// reports them as [`EngineEvent::Found`]. A find of nothing takes the marks away and finds
    /// nothing.
    ///
    /// # Errors
    ///
    /// The tab has no page, or the engine cannot find in pages.
    fn find(&mut self, tab: TabId, query: &str, step: FindStep) -> Result<(), EngineError>;

    /// Takes away the marks and the selection a find left in the tab's page: a find of nothing,
    /// reported like one.
    ///
    /// # Errors
    ///
    /// The tab has no page, or the engine cannot find in pages.
    fn clear_find(&mut self, tab: TabId) -> Result<(), EngineError> {
        self.find(tab, "", FindStep::Refine)
    }

    /// Reads the text the tab's page shows (its main frame, without hidden elements or markup),
    /// at most `max_chars` characters of it. [`EngineEvent::Text`] with the ticket returned
    /// brings it, empty when the page has none or could not be read.
    ///
    /// # Errors
    ///
    /// The tab has no page, or the engine cannot read pages.
    fn read_text(&mut self, tab: TabId, max_chars: usize) -> Result<Ticket, EngineError>;

    /// Finds the links and controls the tab's page shows on screen, for link hints.
    /// [`EngineEvent::Targets`] with the ticket returned brings them in document order, placed
    /// in the page view.
    ///
    /// # Errors
    ///
    /// The tab has no page, or the engine cannot find link targets.
    fn link_targets(&mut self, tab: TabId) -> Result<Ticket, EngineError>;

    /// Follows target `index` of the tab's last [`Self::link_targets`]: the page clicks or
    /// focuses it, or [`EngineEvent::Followed`] with the ticket returned brings a link to open
    /// in another tab (in the background when `background`).
    ///
    /// # Errors
    ///
    /// The tab has no page, or the engine cannot follow link targets.
    fn follow_link(
        &mut self,
        tab: TabId,
        index: usize,
        background: bool,
    ) -> Result<Ticket, EngineError>;
}

/// How large a tab's page is drawn, in the domain's own zoom levels. Each engine maps a level
/// to its own unit (a page zoom factor, a zoom level's logarithm); which page gets which level
/// is the [`Zoom`](crate::Zoom) use case's choice.
pub trait PageZoom {
    /// Draws the tab's page at `level` from now on, its next navigations included, until it is
    /// set again: text and layout scale, as Safari's zoom does. A tab without a page is left
    /// alone.
    fn set_zoom(&mut self, tab: TabId, level: ZoomLevel);
}

/// Answers to what pages asked ([`EngineEvent::Asked`]): a dialog's OK or Cancel and its text,
/// a device request's allow or deny. Which answer a page gets, and whether the user is asked at
/// all, is the [`Prompts`](crate::Prompts) use case's choice.
pub trait PageAsks {
    /// Hands `answer` to the page that waits on `token`, which goes on at once; false when no
    /// page waits on it any more (it was answered, or its tab closed).
    fn answer(&mut self, token: ReplyToken, answer: &Answer) -> bool;
}

/// Downloads, and files made from pages. Each moves on as [`EngineEvent::Download`], under the
/// [`DownloadToken`] the engine gave it when it started; which download in the list a token is,
/// and what the user sees of it, is the [`Library`](crate::Library) use case's business.
pub trait Downloads {
    /// Saves downloads into `directory` from now on, creating it when the first one comes.
    /// Until it is set, every download is refused.
    fn set_directory(&mut self, directory: PathBuf);

    /// Downloads `url` from the tab's page, in its jar, as a retry does; it is reported like
    /// any other download.
    ///
    /// # Errors
    ///
    /// The tab has no page, or the engine cannot download.
    fn download(&mut self, tab: TabId, url: &Url) -> Result<(), EngineError>;

    /// Stops a running download and deletes what it wrote; nothing more is reported for it. A
    /// download that is not running is left alone.
    fn cancel(&mut self, token: DownloadToken);

    /// Saves the tab's page as a PDF in the downloads folder, under its title and a free name,
    /// reported as [`DownloadEvent::Saved`], or [`DownloadEvent::Refused`] when it could not be
    /// made or written.
    ///
    /// # Errors
    ///
    /// The tab has no page, its page has not committed yet, or the engine cannot make PDFs.
    fn save_pdf(&mut self, tab: TabId) -> Result<(), EngineError>;
}
