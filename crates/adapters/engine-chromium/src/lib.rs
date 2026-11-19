//! Engine adapter on Chromium: the Rust side of the cxx bridge to `//quickie`
//! (`engine/quickie/bridge`), and a thin safe wrapper over it.
//!
//! GN builds this crate into the product with the `chromium` feature (`engine/quickie/BUILD.gn`).
//! Cargo builds it without the feature: the C++ side exists only inside a Chromium checkout, so
//! the calls into it are compiled out and the wrapper reports [`EngineError::Unavailable`].
//!
//! TODO(after #6 merges): impl `EnginePort` (`quickie-application`) for a `ChromiumEngine` on top
//! of [`Engine`], mapping `TabId`, `WindowId` and `Jar` to the raw ids used here.
#![expect(
    unsafe_code,
    reason = "FFI: the cxx bridge to Chromium and the NSView pointer of a shell window"
)]

mod dispatch;
mod ffi;

use std::marker::PhantomData;
use std::ptr::NonNull;

/// Where a tab keeps its cookies and storage.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Jar {
    /// The profile's own storage.
    Main,
    /// A throwaway jar: an off-the-record profile of its own, gone with [`Engine::destroy_jar`].
    Throwaway(u64),
}

/// A navigation inside a tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Navigation<'a> {
    /// Load a URL.
    Load(&'a str),
    /// History back.
    Back,
    /// History forward.
    Forward,
    /// Reload.
    Reload,
    /// Stop loading.
    Stop,
}

/// What the engine reports, on the main thread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// A main-frame navigation committed.
    NavigationCommitted {
        /// The tab.
        tab: u64,
        /// The committed URL.
        url: String,
    },
    /// The page title changed.
    TitleChanged {
        /// The tab.
        tab: u64,
        /// The new title.
        title: String,
    },
    /// A page opened a tab or a popup (`window.open`, a link with a target). The page has no tab
    /// id yet: give it one with [`Engine::adopt_tab`], or close it.
    PageOpenedTab {
        /// The engine's handle for the new page.
        page: u64,
        /// The tab that opened it, if the engine knows.
        opener: Option<u64>,
        /// The URL it is opening.
        url: String,
    },
    /// An event this version does not know (the C++ side is newer).
    Unknown,
}

/// A key-down the engine offers before the page sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyEquivalent {
    /// `NSEvent.keyCode`: the physical key.
    pub key_code: u16,
    /// `NSEvent.modifierFlags`.
    pub modifiers: u64,
    /// The first of `charactersIgnoringModifiers`, if any.
    pub character: Option<char>,
    /// `false` before the page sees the key (reserved shortcuts), `true` once the page did not
    /// handle it (everything else; docs/architecture.md, section 4.4).
    pub page_declined: bool,
}

/// Why an engine call failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineError {
    /// The engine refused or failed the call (unknown tab, window or jar; see the Chromium log).
    Rejected,
    /// Built without the `chromium` feature: there is no engine to call.
    Unavailable,
}

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Rejected => f.write_str("the engine rejected the call"),
            Self::Unavailable => f.write_str("built without the Chromium engine"),
        }
    }
}

impl std::error::Error for EngineError {}

/// The Chromium engine, as seen from the main thread. Creating it installs the event and key
/// handlers; dropping it removes them. Not `Send`: Chromium's UI thread is the main thread.
pub struct Engine {
    _main_thread: PhantomData<*const ()>,
}

impl Engine {
    /// Connects to the engine. `events` receives every [`Event`]; `keys` decides whether a key
    /// equivalent is the shell's (`true`) or the page's.
    pub fn new(
        events: impl FnMut(Event) + 'static,
        keys: impl FnMut(KeyEquivalent) -> bool + 'static,
    ) -> Self {
        dispatch::install(Box::new(events), Box::new(keys));
        Self {
            _main_thread: PhantomData,
        }
    }

    /// Creates the page for `tab` in `jar` and starts loading `url`.
    ///
    /// # Errors
    ///
    /// The engine rejected the call, or there is no engine.
    pub fn create_tab(&self, tab: u64, url: &str, jar: Jar) -> Result<(), EngineError> {
        let (throwaway, jar) = match jar {
            Jar::Main => (false, 0),
            Jar::Throwaway(id) => (true, id),
        };
        call(|| ffi::bridge::create_tab(tab, url, throwaway, jar))
    }

    /// Shows `tab` in shell window `window`, filling `host_view`.
    ///
    /// # Errors
    ///
    /// The engine rejected the call, or there is no engine.
    ///
    /// # Safety
    ///
    /// `host_view` is the `NSView*` of window `window` and stays alive while the engine shows
    /// pages in it, that is until the window's tabs are closed or shown elsewhere.
    pub unsafe fn show_tab(
        &self,
        window: u64,
        host_view: NonNull<std::ffi::c_void>,
        tab: u64,
    ) -> Result<(), EngineError> {
        call(|| {
            // SAFETY: the caller guarantees `host_view` is a live NSView* for as long as the
            // engine uses it; the C++ side only stores it and adds a subview to it.
            unsafe { ffi::bridge::show_tab(window, host_view.as_ptr().cast(), tab) }
        })
    }

    /// Navigates `tab`.
    ///
    /// # Errors
    ///
    /// The engine rejected the call, or there is no engine.
    pub fn navigate(&self, tab: u64, navigation: Navigation<'_>) -> Result<(), EngineError> {
        let (kind, url) = navigation.to_bridge();
        call(|| ffi::bridge::navigate_tab(tab, kind, url))
    }

    /// Destroys `tab`'s page.
    ///
    /// # Errors
    ///
    /// The engine rejected the call, or there is no engine.
    pub fn close_tab(&self, tab: u64) -> Result<(), EngineError> {
        call(|| ffi::bridge::close_tab(tab))
    }

    /// Gives a page the engine opened ([`Event::PageOpenedTab`]) its tab id.
    ///
    /// # Errors
    ///
    /// The engine rejected the call, or there is no engine.
    pub fn adopt_tab(&self, page: u64, tab: u64) -> Result<(), EngineError> {
        call(|| ffi::bridge::adopt_tab(page, tab))
    }

    /// Destroys throwaway jar `jar` and everything it stored. No tab may still use it.
    ///
    /// # Errors
    ///
    /// The engine rejected the call, or there is no engine.
    pub fn destroy_jar(&self, jar: u64) -> Result<(), EngineError> {
        call(|| ffi::bridge::destroy_jar(jar))
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        dispatch::uninstall();
    }
}

/// Runs a bridge call when the C++ side is linked in.
#[cfg(feature = "chromium")]
fn call(bridge_call: impl FnOnce() -> bool) -> Result<(), EngineError> {
    if bridge_call() {
        Ok(())
    } else {
        Err(EngineError::Rejected)
    }
}

/// Without the C++ side there is nothing to call; the closure is dropped uncalled, so no symbol of
/// the C++ side is referenced and the crate links on its own.
#[cfg(not(feature = "chromium"))]
fn call(bridge_call: impl FnOnce() -> bool) -> Result<(), EngineError> {
    drop(bridge_call);
    Err(EngineError::Unavailable)
}

#[cfg(test)]
mod tests;
