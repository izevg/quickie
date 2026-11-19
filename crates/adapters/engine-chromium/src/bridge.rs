//! The seam between the engine's bookkeeping and the cxx bridge. [`Bridge`] mirrors the C++
//! entry points one to one, in raw ids, so `ChromiumEngine` is tested against a fake; [`Ffi`] is
//! the real thing. What the C++ side reports comes back through [`dispatch`].

pub(crate) mod dispatch;

use std::ffi::c_void;
use std::fmt;
use std::ptr::NonNull;

use crate::ffi;
pub(crate) use crate::ffi::bridge::NavigationKind;

/// What the engine reports, on the main thread, in the bridge's raw ids.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Event {
    /// A main-frame navigation committed.
    NavigationCommitted { tab: u64, url: String },
    /// The page title changed.
    TitleChanged { tab: u64, title: String },
    /// A page opened a tab or a popup (`window.open`, a link with a target). The page has no tab
    /// id yet; `page` is the engine's handle for it until it is adopted.
    PageOpenedTab {
        page: u64,
        opener: Option<u64>,
        url: String,
    },
    /// An event this version does not know (the C++ side is newer).
    Unknown,
}

/// A key-down the engine offers to the shell.
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

/// Why a bridge call failed. Chromium gives no reason across the bridge; it logs one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BridgeError {
    /// The C++ side refused or failed the call.
    #[cfg_attr(
        not(any(feature = "chromium", test)),
        expect(
            dead_code,
            reason = "only the C++ side rejects, and it is compiled out"
        )
    )]
    Rejected,
    /// Built without the `chromium` feature: there is no engine to call.
    #[cfg_attr(
        feature = "chromium",
        expect(
            dead_code,
            reason = "the C++ side is linked in, so it is always available"
        )
    )]
    Unavailable,
}

impl fmt::Display for BridgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Rejected => "Chromium rejected it (the reason is in Chromium's log)",
            Self::Unavailable => "built without the Chromium engine (the `chromium` feature)",
        })
    }
}

/// The C++ entry points of `quickie_bridge.h`.
pub(crate) trait Bridge {
    /// `CreateTab`.
    fn create_tab(&self, tab: u64, url: &str, throwaway: bool, jar: u64)
    -> Result<(), BridgeError>;

    /// `ShowTab`.
    ///
    /// # Safety
    ///
    /// `host_view` is the `NSView*` of `window` and stays alive while the engine shows pages in
    /// it.
    unsafe fn show_tab(
        &self,
        window: u64,
        host_view: NonNull<c_void>,
        tab: u64,
    ) -> Result<(), BridgeError>;

    /// `NavigateTab`.
    fn navigate_tab(&self, tab: u64, kind: NavigationKind, url: &str) -> Result<(), BridgeError>;

    /// `CloseTab`.
    fn close_tab(&self, tab: u64) -> Result<(), BridgeError>;

    /// `AdoptTab`.
    fn adopt_tab(&self, page: u64, tab: u64) -> Result<(), BridgeError>;

    /// `DestroyJar`.
    fn destroy_jar(&self, jar: u64) -> Result<(), BridgeError>;

    /// `ScheduleWake`: `RunWake` follows on a later turn of the main loop.
    fn schedule_wake(&self);
}

/// The C++ side, linked in under GN with the `chromium` feature.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Ffi;

impl Bridge for Ffi {
    fn create_tab(
        &self,
        tab: u64,
        url: &str,
        throwaway: bool,
        jar: u64,
    ) -> Result<(), BridgeError> {
        call(|| ffi::bridge::create_tab(tab, url, throwaway, jar))
    }

    unsafe fn show_tab(
        &self,
        window: u64,
        host_view: NonNull<c_void>,
        tab: u64,
    ) -> Result<(), BridgeError> {
        call(|| {
            // SAFETY: the caller guarantees `host_view` is a live NSView* for as long as the
            // engine uses it; the C++ side only stores it and adds a subview to it.
            unsafe { ffi::bridge::show_tab(window, host_view.as_ptr().cast(), tab) }
        })
    }

    fn navigate_tab(&self, tab: u64, kind: NavigationKind, url: &str) -> Result<(), BridgeError> {
        call(|| ffi::bridge::navigate_tab(tab, kind, url))
    }

    fn close_tab(&self, tab: u64) -> Result<(), BridgeError> {
        call(|| ffi::bridge::close_tab(tab))
    }

    fn adopt_tab(&self, page: u64, tab: u64) -> Result<(), BridgeError> {
        call(|| ffi::bridge::adopt_tab(page, tab))
    }

    fn destroy_jar(&self, jar: u64) -> Result<(), BridgeError> {
        call(|| ffi::bridge::destroy_jar(jar))
    }

    fn schedule_wake(&self) {
        // Without the engine nothing reports, so nothing ever needs a wake.
        let _ = call(|| {
            ffi::bridge::schedule_wake();
            true
        });
    }
}

/// Runs a bridge call when the C++ side is linked in.
#[cfg(feature = "chromium")]
fn call(bridge_call: impl FnOnce() -> bool) -> Result<(), BridgeError> {
    if bridge_call() {
        Ok(())
    } else {
        Err(BridgeError::Rejected)
    }
}

/// Without the C++ side there is nothing to call; the closure is dropped uncalled, so no symbol of
/// the C++ side is referenced and the crate links on its own.
#[cfg(not(feature = "chromium"))]
fn call(bridge_call: impl FnOnce() -> bool) -> Result<(), BridgeError> {
    drop(bridge_call);
    Err(BridgeError::Unavailable)
}

#[cfg(test)]
mod tests;
