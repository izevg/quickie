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

use std::path::{Path, PathBuf};

use quickie_domain_browsing::{TabId, WindowId};
use quickie_domain_settings::ZoomLevel;
use url::Url;

pub use event::{
    Answer, Ask, AskKind, Dialog, DownloadEvent, DownloadToken, EngineEvent, FindStep, Followed,
    Found, Fullscreen, LinkTarget, PageRect, ReplyToken, Ticket,
};
pub use extensions::{
    Access, Expected, ExtensionCommand, ExtensionEvent, ExtensionHost, ExtensionInfo,
    ExtensionPackages, NoExtensions, PackageSource, Unpacked, WindowTabs,
};
pub use queue::EventQueue;

use crate::{ContentBlockerPort, EngineError, EnginePort, HttpPort};

/// What an engine can do that the UI only degrades without, declared once per engine as
/// [`Engine::CAPS`], so the shell decides at compile time what to offer. A capability a use
/// case cannot work without is a trait bound instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capabilities {
    /// Web extensions run in the main profile's pages ([`ExtensionHost`]).
    pub web_extensions: bool,
    /// How the page's developer tools open.
    pub inspector: Inspector,
    /// The engine reports what its content blocker kept from loading
    /// ([`EngineEvent::Blocked`]), so a page's blocked count means something.
    pub blocked_counts: bool,
}

/// How a page's developer tools open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Inspector {
    /// [`PageSurface::inspect`] opens them, on the panel asked.
    Programmatic,
    /// Only the user opens them, from the page's context menu; [`PageSurface::inspect`] only
    /// says whether they can attach to the page.
    FromContextMenu,
}

/// What the developer tools are asked to do on a page ([`PageSurface::inspect`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DevTools {
    /// Open them where they were, or close them when they show.
    Toggle,
    /// Open them and let the user pick an element of the page to inspect.
    InspectElement,
    /// Open them on a panel.
    Panel(DevToolsPanel),
}

/// A panel of the developer tools, by the names the palette's commands give them; each engine
/// shows its own closest panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DevToolsPanel {
    /// The page's DOM and styles.
    Elements,
    /// The page's console.
    Console,
    /// The page's requests.
    Network,
    /// The page's scripts and its debugger.
    Sources,
    /// A recording of what the page does over time.
    Performance,
    /// The page's storage: cookies, local storage, databases.
    Application,
    /// The page's memory over time.
    Memory,
}

/// Putting pages on screen: each window's host view, which a tab's page fills while the window
/// shows it, and what the page view is up to. The application never looks inside a view or a
/// picture; the shell names their types for its toolkit.
pub trait PageSurface {
    /// A view of the toolkit: what a window's pages fill, and what each page is drawn in.
    type View: ?Sized;
    /// A picture of a page, as the toolkit draws it.
    type Picture;

    /// Makes `host` the view a window's pages fill; the tab shown there, if any, moves in.
    ///
    /// # Errors
    ///
    /// The window already has another host the engine cannot let go of, or the page that waited
    /// for it could not be shown.
    fn attach_host(&mut self, window: WindowId, host: &Self::View) -> Result<(), EngineError>;

    /// Forgets a window's host view; its page leaves it.
    fn detach_host(&mut self, window: WindowId);

    /// The view a tab's page is drawn in, to give it the keyboard; `None` for a tab without a
    /// page.
    fn view(&self, tab: TabId) -> Option<&Self::View>;

    /// True when the focused element of the tab's page takes text, so a key typed now is the
    /// page's rather than a browser shortcut. Asked on every key: the engine answers from what
    /// the page last reported, never by asking it.
    fn input_focused(&self, tab: TabId) -> bool;

    /// How much of a tab's page has loaded, `0.0 ..= 1.0`, while it loads; `None` when it is not
    /// loading or has no page. [`EngineEvent::Loading`] tells when it moves.
    fn loading(&self, tab: TabId) -> Option<f64>;

    /// Takes a new picture of the tab's page; [`EngineEvent::Pictured`] says when
    /// [`Self::picture`] has it. Nothing for a tab without a page or a page not drawn yet.
    fn capture(&mut self, tab: TabId);

    /// The tab's last picture, `None` before its page was first drawn or for a tab without a
    /// page.
    fn picture(&self, tab: TabId) -> Option<Self::Picture>;

    /// Opens the print panel for the tab's page, on its window.
    ///
    /// # Errors
    ///
    /// The tab has no page, or its page is not in a window.
    fn print(&mut self, tab: TabId) -> Result<(), EngineError>;

    /// Does what `asked` says with the developer tools of the tab's page when
    /// [`Capabilities::inspector`] is [`Inspector::Programmatic`]; otherwise only checks that
    /// they can attach to it from the page's context menu.
    ///
    /// # Errors
    ///
    /// The tab has no page, or the developer tools cannot attach to it; the error says why.
    fn inspect(&mut self, tab: TabId, asked: DevTools) -> Result<(), EngineError>;
}

/// A tab's video out of its page, for the mini player: the page of a tab the user left goes
/// into a view over another tab's page and shows only its video, which plays on. Which tab
/// floats and when is the [`Playback`](crate::Playback) use case's choice; the video is the
/// one the page reported last ([`EngineEvent::Video`]).
pub trait PageVideo: PageSurface {
    /// Puts the tab's page into `slot`, filling it and showing only its video, until
    /// [`Self::dock_video`] or until a window shows the tab again, which docks it. The page
    /// floating before, if another, docks first.
    ///
    /// # Errors
    ///
    /// The tab has no page, a window shows it, or the engine cannot float videos.
    fn float_video(&mut self, tab: TabId, slot: &Self::View) -> Result<(), EngineError>;

    /// The tab's page shows whole again and leaves the slot; nothing when it does not float.
    fn dock_video(&mut self, tab: TabId);

    /// Plays the tab's video, or pauses it when `play` is false; the page reports the change
    /// as [`EngineEvent::Video`].
    ///
    /// # Errors
    ///
    /// The tab has no page, or the engine cannot play videos.
    fn play_video(&mut self, tab: TabId, play: bool) -> Result<(), EngineError>;
}

/// One engine: everything a profile's pages need, the network the browser's own requests take,
/// and what the UI may offer on it. The composition root names one; everything else reaches it
/// through these ports.
pub trait Engine:
    EnginePort + PageSurface + PageContent + PageZoom + PageAsks + Downloads + PageVideo
{
    /// The engine's own network stack, for the browser's own requests (filter lists, the agent's
    /// endpoint, extension packages), so every request of the product goes the same way.
    type Http: HttpPort + Clone + Send + 'static;
    /// The content blocker of the engine's pages.
    type Blocker: ContentBlockerPort;
    /// What runs the main profile's web extensions; [`NoExtensions`] on an engine without them.
    type Extensions: ExtensionHost;

    /// What the UI may offer on this engine.
    const CAPS: Capabilities;

    /// A handle on the engine's network, for a worker thread.
    fn http(&self) -> Self::Http;

    /// The host of the web extensions that run in the engine's main-jar pages, made before the
    /// first such page so that every one runs them. `None` when the engine or the system cannot
    /// run them ([`Capabilities::web_extensions`] says whether the engine ever can).
    fn web_extensions(&mut self) -> Option<Self::Extensions>;

    /// The content blocker of every page, existing and future. It keeps what it compiles in
    /// `directory`, which it creates.
    ///
    /// # Errors
    ///
    /// The directory cannot be created or used, or the engine has no content blocker.
    fn content_blocker(&mut self, directory: &Path) -> Result<Self::Blocker, EngineError>;

    /// Sets what runs, later on the main loop and never inside an engine call, when events wait:
    /// it should [`Self::drain`] them. Events already waiting wake it at once. Set once, at
    /// start.
    fn set_waker(&mut self, wake: Box<dyn Fn()>);

    /// Moves what pages did since the last call to the end of `into`, oldest first.
    fn drain(&mut self, into: &mut Vec<EngineEvent<Self::Page>>);
}

/// Engine `E` seen through its ports alone: what a use case that owns an engine hands out, so
/// that no adapter's own methods are reachable from the shell. The types the ports name stay
/// `E`'s, for the composition root to name.
pub trait PortsOf<E: Engine>:
    Engine<
        Page = E::Page,
        View = E::View,
        Picture = E::Picture,
        Http = E::Http,
        Blocker = E::Blocker,
        Extensions = E::Extensions,
    >
{
}

impl<T, E> PortsOf<E> for T
where
    E: Engine,
    T: Engine<
            Page = E::Page,
            View = E::View,
            Picture = E::Picture,
            Http = E::Http,
            Blocker = E::Blocker,
            Extensions = E::Extensions,
        >,
{
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

    /// Takes the tab's page out of full screen, as its own exit control would: `⎋`, its window
    /// leaving full screen, or its tab going to the background. [`EngineEvent::Fullscreen`]
    /// with [`Fullscreen::Left`] says when it did. A page not in full screen, or a tab without
    /// a page, is left alone.
    fn exit_fullscreen(&mut self, tab: TabId);
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
