//! Development engine of Quickie: pages on `WKWebView`, so UI work never waits for a Chromium
//! build (`docs/architecture.md` §3, P0). It is one whole [`Engine`] of the application: its
//! pages behind [`EnginePort`] and the capability ports, `NSView`s and `NSImage`s on its
//! [`PageSurface`].
//!
//! The engine keeps one page per tab, keyed by the domain's [`TabId`]. Main-jar pages share the
//! default data store; each throwaway jar gets one non-persistent store, shared by its tabs and
//! dropped with the jar. The shell registers each window's host view
//! ([`PageSurface::attach_host`]); [`EnginePort::show`] puts a tab's page into it, under any
//! overlays.
//!
//! Everything pages report flows back as one stream of [`EngineEvent`]s, without re-entering
//! the use cases: pages record what they did in one queue, the engine schedules the shell's
//! waker ([`Engine::set_waker`]) on the main run loop, and the shell drains the queue
//! ([`Engine::drain`]). Answers to requests (a find, link targets, page text, a picture)
//! come the same way. An explicit agent link read completes on the main thread with its own
//! callback.
//!
//! Pages open pages of their own (`window.open`, `target=_blank`, `⌘`-click): the engine makes
//! them at once, from the configuration `WebKit` passes, so they share the opener's process and
//! jar, and the shell hands each one ([`EngineEvent::Opened`]) to `Browsing::adopt`. A page's
//! dialogs and permission requests wait as asks ([`EngineEvent::Asked`]) until the shell
//! answers them. A page whose web content process died is reported ([`EngineEvent::Crashed`])
//! and stays blank until it is reloaded.
//!
//! Main-jar pages share one user content controller and throwaway-jar pages another, so the
//! [`WebKitContentBlocker`] the engine hands out ([`Engine::content_blocker`]) installs
//! the same rules in both and blocks on every page, popups included, and on pages made before
//! its rules were compiled. Its cosmetic filters and scriptlets ride in the same controllers; a
//! frame's document waits, briefly, for its host's. Extensions' content scripts land only in
//! the main jar's controller, so they never reach a throwaway page. [`UrlSessionHttp`] is the
//! network for the browser's own requests ([`Engine::http`]), such as fetching filter lists.
//!
//! Navigations that become downloads go to one download center, which saves them into the
//! folder [`Downloads::set_directory`] names and reports [`EngineEvent::Download`]s; a page
//! saved as a PDF ([`Downloads::save_pdf`]) is written there too, under a free name.
//!
//! Web Inspector is [`Inspector::Programmatic`] on our `WebKit` fork, which the app ships
//! with: the developer tools' commands open it on a panel through the fork's `_WKInspector`,
//! docked under the page, and the page's context menu offers Inspect Element. On the system
//! `WebKit` (a plain `cargo run`) [`PageSurface::inspect`] fails and says that Safari's Develop
//! menu attaches it, as every page is inspectable (`WKWebView.isInspectable`). Each page sits
//! in a frame view of its own, which the engine puts into windows, so a docked inspector goes
//! with its tab. Page zoom ([`PageZoom`]) and printing ([`PageSurface::print`]) are the view's
//! own.
//!
//! Web extensions ([`WebKitExtensions`], from [`Engine::web_extensions`]) run in the
//! main jar's pages through one `WKWebExtensionController`, on macOS 15.4 and later; never in
//! a throwaway jar's.

#![cfg_attr(
    target_os = "macos",
    expect(
        unsafe_code,
        reason = "WebKit through objc2: its bindings mark most methods unsafe; KVO, the navigation \
              UI, download delegates, the scheme handler, and the extension controller's \
              delegate, tabs and windows need classes; content rule lists cross threads"
    )
)]

#[path = "shields/rules.rs"]
pub mod rules;

#[path = "downloads/name.rs"]
pub mod download_name;

#[path = "shields/cosmetic/filters.rs"]
pub mod cosmetic_rules;

#[path = "page/scripts/hints.rs"]
pub mod link_hints;

#[path = "page/observation/mod.rs"]
pub mod observation;

#[cfg(target_os = "macos")]
include!("appkit.rs");
