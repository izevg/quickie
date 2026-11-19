//! Driven ports: what the use cases need from the outside world.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use quickie_domain_agents::{
    ApiKey, BaseUrl, EndpointSnapshot, KeyRef, LlmEndpoint, LocalServer, ModelId, Prompt,
};
use quickie_domain_browsing::{Jar, JarId, SessionSnapshot, TabId, Timestamp, WindowId};
use quickie_domain_library::{BookmarksSnapshot, HistorySnapshot, SearchShortcutsSnapshot};
use quickie_domain_shields::{
    ContentHash, FilterListId, FilterListSetSnapshot, Site, SitePoliciesSnapshot,
};
use url::Url;

use crate::{BlockerError, EngineError, HttpError, LlmError, SecretError, StoreError};

/// The wall clock.
pub trait Clock {
    /// Now.
    fn now(&self) -> Timestamp;
}

/// Where the session persists (`shell.sqlite` in the product).
pub trait SessionRepo {
    /// The last saved session, `None` on first launch.
    ///
    /// # Errors
    ///
    /// The store could not be read.
    fn load(&self) -> Result<Option<SessionSnapshot>, StoreError>;

    /// Replaces the saved session, atomically.
    ///
    /// # Errors
    ///
    /// The store could not be written.
    fn save(&mut self, snapshot: &SessionSnapshot) -> Result<(), StoreError>;
}

/// The library as last saved. Each part is `None` until it is first saved, so a profile that
/// deleted every search shortcut does not get the defaults back.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LibrarySnapshot {
    /// The visited pages.
    pub history: Option<HistorySnapshot>,
    /// The saved pages and their tags.
    pub bookmarks: Option<BookmarksSnapshot>,
    /// The `!` keyword searches.
    pub shortcuts: Option<SearchShortcutsSnapshot>,
}

/// Where the library persists (`shell.sqlite` in the product). Each aggregate saves on its own,
/// so a visit never rewrites the bookmarks. Downloads are not persisted yet.
pub trait LibraryRepo {
    /// Every part of the library that was ever saved.
    ///
    /// # Errors
    ///
    /// The store could not be read.
    fn load_library(&self) -> Result<LibrarySnapshot, StoreError>;

    /// Replaces the saved history, atomically.
    ///
    /// # Errors
    ///
    /// The store could not be written.
    fn save_history(&mut self, history: &HistorySnapshot) -> Result<(), StoreError>;

    /// Replaces the saved bookmarks, atomically.
    ///
    /// # Errors
    ///
    /// The store could not be written.
    fn save_bookmarks(&mut self, bookmarks: &BookmarksSnapshot) -> Result<(), StoreError>;

    /// Replaces the saved search shortcuts, atomically.
    ///
    /// # Errors
    ///
    /// The store could not be written.
    fn save_shortcuts(&mut self, shortcuts: &SearchShortcutsSnapshot) -> Result<(), StoreError>;
}

/// One entry of the user's `keybindings.json`, as text: the store does not judge it, the
/// keymap use case does, so a bad entry is reported instead of losing the file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeybindingText {
    /// The chord, such as `cmd+k cmd+s`.
    pub key: String,
    /// The command id, or `-<id>` to remove a default binding.
    pub command: String,
    /// The `when` clause, if the entry has one.
    pub when: Option<String>,
}

/// Where the user's key bindings persist. Saving arrives with the keymap editor.
pub trait KeymapRepo {
    /// The user's entries, in the order they were written.
    ///
    /// # Errors
    ///
    /// The store could not be read.
    fn load_keybindings(&self) -> Result<Vec<KeybindingText>, StoreError>;
}

/// A navigation inside a tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Navigation {
    /// Load a URL.
    Load(Url),
    /// History back.
    Back,
    /// History forward.
    Forward,
    /// Reload.
    Reload,
    /// Stop loading.
    Stop,
}

/// The browser engine: Chromium in the product, `WebKit` in development. It owns live pages;
/// it learns about tabs only through these calls, keyed by the domain's [`TabId`].
pub trait EnginePort {
    /// The engine's handle on a page it opened by itself (a popup, a ⌘-click), before the
    /// session gives it a [`TabId`].
    type Page;

    /// Binds a page the engine opened to the tab the session made for it. Called before the
    /// tab is shown, so the engine knows which page to show.
    ///
    /// # Errors
    ///
    /// The page is gone, or the tab already has a page.
    fn adopt(&mut self, page: Self::Page, tab: TabId) -> Result<(), EngineError>;

    /// Creates the page for a tab, in its jar. Called lazily: a restored tab gets a page only
    /// when it is first shown.
    ///
    /// # Errors
    ///
    /// The engine could not create the page.
    fn create(&mut self, tab: TabId, url: &Url, jar: Jar) -> Result<(), EngineError>;

    /// Shows a tab's page in a window, in place of the one shown before.
    ///
    /// # Errors
    ///
    /// The engine has no page for the tab or no such window.
    fn show(&mut self, window: WindowId, tab: TabId) -> Result<(), EngineError>;

    /// Navigates a tab's page.
    ///
    /// # Errors
    ///
    /// The engine has no page for the tab.
    fn navigate(&mut self, tab: TabId, navigation: &Navigation) -> Result<(), EngineError>;

    /// Destroys a tab's page.
    ///
    /// # Errors
    ///
    /// The engine failed to close the page.
    fn close(&mut self, tab: TabId) -> Result<(), EngineError>;

    /// Destroys a throwaway jar's cookies and storage; no live tab uses it any more.
    ///
    /// # Errors
    ///
    /// The engine failed to clear the partition.
    fn destroy_jar(&mut self, jar: JarId) -> Result<(), EngineError>;
}

/// The shields as last saved. A profile that never saved them has none of each: no exceptions,
/// every list off and never fetched.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShieldsSnapshot {
    /// The per-site exceptions.
    pub sites: SitePoliciesSnapshot,
    /// The filter lists' states.
    pub lists: FilterListSetSnapshot,
    /// The content last fetched of each list, as it arrived.
    pub bodies: Vec<(FilterListId, Vec<u8>)>,
}

/// Where the shields persist (`shell.sqlite` in the product). Each part saves on its own, so
/// toggling a site never rewrites a list's content.
pub trait ShieldsRepo {
    /// Everything ever saved of the shields.
    ///
    /// # Errors
    ///
    /// The store could not be read.
    fn load_shields(&self) -> Result<ShieldsSnapshot, StoreError>;

    /// Replaces the saved per-site exceptions, atomically.
    ///
    /// # Errors
    ///
    /// The store could not be written.
    fn save_sites(&mut self, sites: &SitePoliciesSnapshot) -> Result<(), StoreError>;

    /// Replaces the saved filter list states, atomically.
    ///
    /// # Errors
    ///
    /// The store could not be written.
    fn save_lists(&mut self, lists: &FilterListSetSnapshot) -> Result<(), StoreError>;

    /// Replaces the saved content of one list, atomically.
    ///
    /// # Errors
    ///
    /// The store could not be written.
    fn save_list_body(&mut self, list: FilterListId, body: &[u8]) -> Result<(), StoreError>;
}

/// The network, for the browser's own requests, such as fetching a filter list or asking the
/// LLM endpoint. Chromium's network stack in the product, so the egress test sees every
/// request; `NSURLSession` in development.
pub trait HttpPort {
    /// GETs `url` and returns the body of a successful (2xx) response. Blocks until the whole
    /// body arrived: call it on a worker thread, never on the main thread.
    ///
    /// # Errors
    ///
    /// No connection, a non-2xx status, a timeout, or a call on the main thread.
    fn get(&self, url: &Url) -> Result<Vec<u8>, HttpError>;

    /// Sends `request` and hands its answer to `on_event` as it arrives: the status first, then
    /// each piece of the body, whatever the status, so a caller can read an error's body too.
    /// Blocks until the body ended or `stop` was signalled, which it notices within a fraction
    /// of a second even while no byte arrives: call it on a worker thread, never on the main
    /// thread. `on_event` runs on the calling thread.
    ///
    /// # Errors
    ///
    /// No connection, a timeout, a body cut off, or a call on the main thread.
    fn stream(
        &self,
        request: &HttpRequest,
        stop: &StopSignal,
        on_event: &mut dyn FnMut(HttpEvent<'_>),
    ) -> Result<Ended, HttpError>;
}

/// What an [`HttpPort::stream`] request does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HttpMethod {
    /// GET, without a body.
    Get,
    /// POST with this body.
    Post(Vec<u8>),
}

/// A request for [`HttpPort::stream`]. It carries no cookies or credentials but the headers
/// given here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpRequest {
    /// GET or POST.
    pub method: HttpMethod,
    /// Where to.
    pub url: Url,
    /// Header names and values, in order.
    pub headers: Vec<(String, String)>,
}

/// What arrives of an answer, in order: [`HttpEvent::Status`] once, then the body in pieces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HttpEvent<'a> {
    /// The status code of the answer.
    Status(u16),
    /// The next piece of the body, cut wherever the network cut it.
    Body(&'a [u8]),
}

/// How a stream ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ended {
    /// The whole answer arrived.
    Complete,
    /// The stop signal cut it short.
    Stopped,
}

/// Asks a running stream to stop: the user pressed `Stop ⌘.`. Clones share one signal, so the
/// main thread keeps one and the worker thread streaming the answer keeps another.
#[derive(Clone, Debug, Default)]
pub struct StopSignal(Arc<AtomicBool>);

impl StopSignal {
    /// A signal not given yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Gives the signal; every clone sees it.
    pub fn stop(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    /// True once the signal was given.
    #[must_use]
    pub fn is_stopped(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// Where API keys are kept: the Keychain. Keys never go to the store or to a file; the store
/// keeps only a [`KeyRef`]. Takes `&self` throughout, as the Keychain is shared state of the
/// system: the agent writes a key on the main thread and a worker reads it before asking.
pub trait SecretStore {
    /// The key kept under `key`, or `None` when there is none.
    ///
    /// # Errors
    ///
    /// The Keychain could not be read, or what it holds is no API key.
    fn read(&self, key: &KeyRef) -> Result<Option<ApiKey>, SecretError>;

    /// Keeps `value` under `key`, replacing any key there.
    ///
    /// # Errors
    ///
    /// The Keychain could not be written.
    fn write(&self, key: &KeyRef, value: &ApiKey) -> Result<(), SecretError>;

    /// Removes the key kept under `key`; nothing there is no error.
    ///
    /// # Errors
    ///
    /// The Keychain could not be written.
    fn delete(&self, key: &KeyRef) -> Result<(), SecretError>;
}

/// Where the agent's settings persist (`shell.sqlite` in the product): the endpoint, with a
/// reference to its key but never the key.
pub trait AgentRepo {
    /// The saved endpoint, `None` when none was ever set up or it was forgotten.
    ///
    /// # Errors
    ///
    /// The store could not be read.
    fn load_endpoint(&self) -> Result<Option<EndpointSnapshot>, StoreError>;

    /// Replaces the saved endpoint, atomically; `None` forgets it.
    ///
    /// # Errors
    ///
    /// The store could not be written.
    fn save_endpoint(&mut self, endpoint: Option<&EndpointSnapshot>) -> Result<(), StoreError>;
}

/// One chat completion to stream.
#[derive(Clone, Copy, Debug)]
pub struct ChatRequest<'a> {
    /// Where to ask, and which model.
    pub endpoint: &'a LlmEndpoint,
    /// The endpoint's key, when it has one; it goes in the `Authorization` header.
    pub key: Option<&'a ApiKey>,
    /// The messages.
    pub prompt: &'a Prompt,
}

/// What a streaming chat completion delivers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChatEvent {
    /// The next piece of the answer, usually one token.
    Delta(String),
    /// The server's count of the tokens it generated, sent at the end by servers that count.
    Usage {
        /// Tokens in the answer.
        completion_tokens: u64,
    },
}

/// An LLM through the OpenAI-compatible API, on top of [`HttpPort`]. Every call blocks: call it
/// on a worker thread, never on the main thread.
pub trait LlmPort {
    /// Streams the answer to `request` into `on_event`, on the calling thread, until the answer
    /// ends or `stop` is signalled.
    ///
    /// # Errors
    ///
    /// The request failed, the endpoint refused it or failed mid-answer, or the answer is not
    /// the API's.
    fn chat(
        &self,
        request: &ChatRequest<'_>,
        stop: &StopSignal,
        on_event: &mut dyn FnMut(ChatEvent),
    ) -> Result<Ended, LlmError>;

    /// The models an endpoint offers (`<base>/models`), for picking one in settings.
    ///
    /// # Errors
    ///
    /// As for [`Self::chat`].
    fn models(&self, base: &BaseUrl, key: Option<&ApiKey>) -> Result<Vec<ModelId>, LlmError>;

    /// The models a local server has, asked at its [`LocalServer::probe_url`].
    ///
    /// # Errors
    ///
    /// Nothing listens there, or it answered unlike the server it should be.
    fn probe(&self, server: LocalServer) -> Result<Vec<ModelId>, LlmError>;
}

/// One filter list's content, as the content blocker compiles it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListRules {
    /// Which list.
    pub list: FilterListId,
    /// The hash of `body`, which names the compiled rules.
    pub hash: ContentHash,
    /// The list as fetched: filter rules in Adblock Plus syntax. Shared, not copied, with the
    /// thread that compiles it.
    pub body: Arc<[u8]>,
}

/// Everything the content blocker needs: the enabled lists' content and the sites whose
/// shields are off.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BlockerRules {
    /// The enabled lists that have content, in [`FilterListId::ALL`] order.
    pub lists: Vec<ListRules>,
    /// The sites whose pages load everything, in the order their shields went off.
    pub unshielded: Vec<Site>,
}

/// The engine's request blocker: `adblock` in Chromium's network service in the product, a
/// `WKContentRuleList` in development.
pub trait ContentBlockerPort {
    /// Makes `rules` the ones every page loads with from now on, and returns at once: compiling
    /// takes seconds, so it happens off the main thread, and pages keep the previous rules until
    /// it is done. Rules equal to the installed ones cost nothing.
    ///
    /// # Errors
    ///
    /// The blocker could not start; a failure while compiling is the adapter's to report.
    fn install(&mut self, rules: &BlockerRules) -> Result<(), BlockerError>;
}
