//! The cxx bridge to `//quickie/bridge` (`engine/quickie/bridge/quickie_bridge.h`). Both sides
//! change together: a signature here is a signature there.
//!
//! Chromium builds C++ without exceptions, so no function returns `Result`: failures come back as
//! `false` and the safe wrapper in `lib.rs` turns them into errors.

#[cxx::bridge(namespace = "quickie")]
pub(crate) mod bridge {
    /// How a tab's page navigates.
    #[derive(Debug)]
    #[repr(u8)]
    enum NavigationKind {
        /// Load the URL passed alongside.
        Load,
        /// History back.
        Back,
        /// History forward.
        Forward,
        /// Reload.
        Reload,
        /// Stop loading.
        Stop,
    }

    /// What an engine event reports.
    #[derive(Debug)]
    #[repr(u8)]
    enum EngineEventKind {
        /// A main-frame navigation committed; `text` is the URL.
        NavigationCommitted,
        /// The page title changed; `text` is the title.
        TitleChanged,
        /// The page opened a new tab or popup; `tab` is the engine's page handle until Rust adopts
        /// it, `opener` the tab that opened it, `text` the target URL.
        PageOpenedTab,
    }

    /// An event from the engine, delivered on the main thread.
    struct EngineEvent {
        kind: EngineEventKind,
        tab: u64,
        has_opener: bool,
        opener: u64,
        text: String,
    }

    extern "Rust" {
        /// Called once by the browser's main parts after browser start (`//quickie/main_parts.cc`).
        #[cxx_name = "ShellStart"]
        fn shell_start();

        /// Called by the C++ side for every engine event.
        #[cxx_name = "OnEngineEvent"]
        fn on_engine_event(event: &EngineEvent);

        /// Called by the C++ side with a key-down: before the renderer sees it, and again if the
        /// page did not handle it (`page_declined`). `true` consumes it. `key_code` is
        /// `NSEvent.keyCode`, `modifiers` `NSEvent.modifierFlags`, `character` the first UTF-16
        /// unit of `charactersIgnoringModifiers` (0 if none).
        #[cxx_name = "HandleKeyEquivalent"]
        fn handle_key_equivalent(
            key_code: u16,
            modifiers: u64,
            character: u32,
            page_declined: bool,
        ) -> bool;
    }

    unsafe extern "C++" {
        include!("quickie/bridge/quickie_bridge.h");

        /// The `NSView` of a shell window that hosts the shown page. Opaque on both sides.
        type HostView;

        /// Creates the page for `tab` in the main profile, or in throwaway jar `jar`.
        #[cxx_name = "CreateTab"]
        fn create_tab(tab: u64, url: &str, throwaway: bool, jar: u64) -> bool;

        /// Shows `tab` in shell window `window`, inside `host_view`.
        ///
        /// # Safety
        ///
        /// `host_view` is a live `NSView*` and outlives the window's use by the engine.
        #[cxx_name = "ShowTab"]
        unsafe fn show_tab(window: u64, host_view: *mut HostView, tab: u64) -> bool;

        /// Navigates `tab`; `url` is used by `NavigationKind::Load` only.
        #[cxx_name = "NavigateTab"]
        fn navigate_tab(tab: u64, kind: NavigationKind, url: &str) -> bool;

        /// Destroys `tab`'s page.
        #[cxx_name = "CloseTab"]
        fn close_tab(tab: u64) -> bool;

        /// Gives a page the engine opened (an `EngineEventKind::PageOpenedTab` handle) its tab id.
        #[cxx_name = "AdoptTab"]
        fn adopt_tab(page: u64, tab: u64) -> bool;

        /// Destroys throwaway jar `jar` and its storage; no tab uses it any more.
        #[cxx_name = "DestroyJar"]
        fn destroy_jar(jar: u64) -> bool;
    }
}

use bridge::{EngineEvent, EngineEventKind, NavigationKind};

pub(crate) fn on_engine_event(event: &EngineEvent) {
    crate::dispatch::deliver_event(crate::Event::from_bridge(event));
}

/// TODO(after the shell crate exists): start the shell here. The composition root is not an
/// adapter, so this crate cannot call it; the plan is an `extern "Rust"` symbol the shell crate
/// defines, declared here, resolved when GN links both (engine/README.md, open questions).
fn shell_start() {}

pub(crate) fn handle_key_equivalent(
    key_code: u16,
    modifiers: u64,
    character: u32,
    page_declined: bool,
) -> bool {
    crate::dispatch::deliver_key(crate::KeyEquivalent {
        key_code,
        modifiers,
        character: char::from_u32(character),
        page_declined,
    })
}

impl crate::Event {
    /// Reads a bridge event; an unknown kind (a newer C++ side) is reported as such.
    pub(crate) fn from_bridge(event: &EngineEvent) -> Self {
        let text = event.text.clone();
        match event.kind {
            EngineEventKind::NavigationCommitted => Self::NavigationCommitted {
                tab: event.tab,
                url: text,
            },
            EngineEventKind::TitleChanged => Self::TitleChanged {
                tab: event.tab,
                title: text,
            },
            EngineEventKind::PageOpenedTab => Self::PageOpenedTab {
                page: event.tab,
                opener: event.has_opener.then_some(event.opener),
                url: text,
            },
            _ => Self::Unknown,
        }
    }
}

impl<'a> crate::Navigation<'a> {
    /// The bridge's kind and URL argument (empty unless loading).
    pub(crate) fn to_bridge(self) -> (NavigationKind, &'a str) {
        match self {
            Self::Load(url) => (NavigationKind::Load, url),
            Self::Back => (NavigationKind::Back, ""),
            Self::Forward => (NavigationKind::Forward, ""),
            Self::Reload => (NavigationKind::Reload, ""),
            Self::Stop => (NavigationKind::Stop, ""),
        }
    }
}
