//! What an engine reports and what it is asked with, as plain values in the domain's words.
//! Every engine adapter converts its own mechanism (`WebKit` delegates, Chromium observers)
//! into these, so the shell reads the same types whichever engine runs the pages.

use std::path::PathBuf;

use quickie_domain_browsing::{FindMatches, Focus, Jar, TabId};
use url::Url;

use crate::Permission;

/// What pages did, in the order they did it. An engine queues them and the shell drains them on
/// the main thread after the engine's waker ran, never inside an engine call, so no engine call
/// ever re-enters a use case. `P` is the engine's handle on a page it opened by itself, the type
/// `Browsing::adopt` takes.
#[derive(Debug, PartialEq)]
pub enum EngineEvent<P> {
    /// The page committed a navigation: a visit to `url`, titled `title` so far.
    Committed { tab: TabId, url: Url, title: String },
    /// The page's title changed after its commit was reported: the visit to `url` is renamed.
    Titled { tab: TabId, url: Url, title: String },
    /// How much of the page has loaded, `0.0 ..= 1.0`, while it loads; `None` once it stopped.
    /// One per tab between drains: the latest.
    Loading { tab: TabId, progress: Option<f64> },
    /// A page opened a page of its own (a popup, a `target=_blank` link, a `⌘`-click), to adopt
    /// into a tab next to its opener's, in its jar. `url` is `about:blank` for a page opened
    /// without one.
    Opened {
        opener: TabId,
        page: P,
        url: Url,
        focus: Focus,
    },
    /// The page closed itself (`window.close()`).
    ClosedByPage { tab: TabId },
    /// The process that rendered the page is gone, and the page with it; the tab stays and a
    /// reload brings it back.
    Crashed { tab: TabId },
    /// The page entered (`true`) or left full screen.
    Fullscreen { tab: TabId, entered: bool },
    /// The page asks the user a question and waits: answer it through the engine with the
    /// ask's token. Closing the tab declines it.
    Asked(Ask),
    /// The matches of a find.
    Found(Found),
    /// The links and controls on screen, in document order, for the link hints that `ticket`
    /// asked for.
    Targets {
        tab: TabId,
        ticket: Ticket,
        targets: Vec<LinkTarget>,
    },
    /// What following the link hint that `ticket` named left for the browser to do.
    Followed {
        tab: TabId,
        ticket: Ticket,
        followed: Followed,
    },
    /// The rendered text of the page, for the read that `ticket` asked for; empty when the page
    /// has none or could not be read.
    Text {
        tab: TabId,
        ticket: Ticket,
        text: String,
    },
    /// A new picture of the page can be read.
    Pictured { tab: TabId },
    /// The content blocker kept `requests` of the page's loads from happening, since the last
    /// report; they belong to the page the tab committed last.
    Blocked { tab: TabId, requests: u32 },
    /// A download moved on.
    Download(DownloadEvent),
}

/// Names a request for a page's content (its text, its link targets, following one): the event
/// that answers it carries the same ticket. The engine mints it; the shell only compares it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Ticket(u64);

impl Ticket {
    /// The ticket an engine numbered `raw`; unique among its requests for as long as it runs.
    #[must_use]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }
}

/// Where a find goes from the current selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FindStep {
    /// The query changed: the first match from the start of the selection, so typing on
    /// keeps the match under the selection when it still matches.
    Refine,
    /// The match after the selection.
    Next,
    /// The match before the selection.
    Previous,
}

/// The matches of a find in a tab's page, answering the engine's `find`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub tab: TabId,
    /// The query they match, to tell an answer to an older query apart.
    pub query: String,
    pub matches: FindMatches,
}

/// A rectangle in a page view, in points, the origin at the view's top left. Each shell turns
/// it into its toolkit's geometry.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PageRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// A link or control the user can see, for a link hint.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LinkTarget {
    /// Where it is in the page view.
    pub frame: PageRect,
    /// It is a link with a URL, so it can open in another tab.
    pub link: bool,
}

/// What following a link hint left for the browser to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Followed {
    /// The page did it: a click, a focus, or the target was gone.
    Done,
    /// Open this URL in a background tab.
    Background(Url),
    /// Open this URL in a new tab in front: the link asked for a new window.
    NewTab(Url),
}

/// Names a question a page waits on until it is answered. The engine mints it; the shell only
/// hands it back.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ReplyToken(u64);

impl ReplyToken {
    /// The token an engine numbered `raw`; unique among its asks for as long as it runs.
    #[must_use]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }
}

/// A JavaScript dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Dialog {
    /// `alert()`: a message and OK.
    Alert { message: String },
    /// `confirm()`: OK or Cancel.
    Confirm { message: String },
    /// `prompt()`: a line of text, OK or Cancel.
    Prompt { message: String, default: String },
}

/// What a page asks the user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AskKind {
    Dialog(Dialog),
    Permission(Permission),
}

/// A question a page waits on: answer it through the engine with its token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ask {
    pub token: ReplyToken,
    pub tab: TabId,
    /// The origin that asks, as a URL (`https://meet.example/`); `about:blank` for a frame
    /// without one.
    pub origin: Url,
    pub kind: AskKind,
}

/// The user's answer to an ask.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    /// OK, or allow; a prompt's text.
    Accept(Option<String>),
    /// Cancel, or deny: the safe default.
    Decline,
}

/// The engine's handle on one download, for as long as Quickie runs. The engine mints it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DownloadToken(u64);

impl DownloadToken {
    /// The token an engine numbered `raw`; unique among its downloads for as long as it runs.
    #[must_use]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }
}

/// What a download did since the shell last asked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DownloadEvent {
    /// It has a file and is fetching into it.
    Started {
        token: DownloadToken,
        /// The tab whose page started it, and that page's jar.
        tab: TabId,
        jar: Jar,
        /// Where the bytes come from.
        url: Url,
        /// The file it is saved to; it did not exist before.
        file: PathBuf,
    },
    /// Bytes arrived; `total` is the size the server announced, if it did.
    Progress {
        token: DownloadToken,
        received: u64,
        total: Option<u64>,
    },
    /// Every byte is in the file.
    Finished { token: DownloadToken },
    /// It stopped, for `reason`.
    Failed {
        token: DownloadToken,
        reason: String,
    },
    /// It never started: no file could be picked for it, for `reason`.
    Refused {
        token: DownloadToken,
        reason: String,
    },
    /// A file Quickie made from a page, such as a PDF of it, is written whole: it starts and
    /// ends at once.
    Saved {
        token: DownloadToken,
        /// The tab whose page it was made from, and that page's jar.
        tab: TabId,
        jar: Jar,
        /// The page.
        url: Url,
        /// The file; it did not exist before.
        file: PathBuf,
    },
}

impl DownloadEvent {
    /// The download it is about.
    #[must_use]
    pub fn token(&self) -> DownloadToken {
        match self {
            Self::Started { token, .. }
            | Self::Progress { token, .. }
            | Self::Finished { token }
            | Self::Failed { token, .. }
            | Self::Refused { token, .. }
            | Self::Saved { token, .. } => *token,
        }
    }
}
