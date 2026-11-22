//! Web Inspector, opened by the app on our `WebKit` fork.
//!
//! The fork is our own engine, so its SPI is our API: `-[WKWebView _inspector]` hands out the
//! page's `_WKInspector`, which shows, closes, picks an element, and, with the fork's I1 patch
//! (`engine/webkit/PATCHES.md` §2.2), selects a tab (`showTab:`) and docks to a side
//! (`attachToSide:`); `-[WKPreferences _setDeveloperExtrasEnabled:]` lets it open at all and
//! puts Inspect Element into the page's context menu. Apple's system `WebKit` has the older of
//! these selectors as private API, which Quickie never calls: everything here asks first whether
//! the running `WebKit` is the fork ([`on_fork`]), by whether `_WKInspector` answers I1's
//! selectors, and on the system `WebKit` does nothing (the inspector then attaches only from
//! Safari's Develop menu, through the public `isInspectable`).
//!
//! The inspector docks inside the page's own frame view (`WebPage::frame`), next to the page,
//! so it leaves the window with its tab and comes back with it.

#[cfg(test)]
mod tests;

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use objc2::{msg_send, sel};
use objc2_foundation::NSString;
use objc2_web_kit::{WKPreferences, WKWebView};
use quickie_application::{DevTools, DevToolsPanel, EngineError};

/// Whether the running `WebKit` is our fork: its `_WKInspector` answers I1's `showTab:` and
/// `attachToSide:`. Asking the runtime calls nothing of the system `WebKit`'s.
pub(crate) fn on_fork() -> bool {
    AnyClass::get(c"_WKInspector").is_some_and(|class| {
        class.responds_to(sel!(showTab:)) && class.responds_to(sel!(attachToSide:))
    })
}

/// Lets the inspector open on pages made with `preferences`, and offers Inspect Element in their
/// context menu; nothing on the system `WebKit`.
pub(crate) fn allow(preferences: &WKPreferences) {
    if on_fork() {
        // SAFETY: the fork's `WKPreferences` has this `BOOL` setter (`WKPreferencesPrivate.h`).
        let () = unsafe { msg_send![preferences, _setDeveloperExtrasEnabled: true] };
    }
}

/// Does what `asked` says with the inspector of `view`'s page.
///
/// # Errors
///
/// The running `WebKit` is not our fork, or it has no inspector for the page.
pub(crate) fn open(view: &WKWebView, asked: DevTools) -> Result<(), EngineError> {
    let inspector = inspector(view)?;
    match asked {
        DevTools::Toggle => {
            // SAFETY: `_WKInspector`'s `BOOL` getter.
            let visible: bool = unsafe { msg_send![&inspector, isVisible] };
            if visible {
                // SAFETY: closing an inspector that shows has no preconditions.
                let () = unsafe { msg_send![&inspector, close] };
            } else {
                // SAFETY: showing has no preconditions; it connects the inspector first.
                let () = unsafe { msg_send![&inspector, show] };
            }
        }
        DevTools::InspectElement => {
            // SAFETY: as above.
            let () = unsafe { msg_send![&inspector, show] };
            // SAFETY: `_WKInspector`'s `BOOL` getter.
            let picking: bool = unsafe { msg_send![&inspector, isElementSelectionActive] };
            if !picking {
                // SAFETY: starts picking; the inspector is connected by `show`.
                let () = unsafe { msg_send![&inspector, toggleElementSelection] };
            }
        }
        DevTools::Panel(panel) => {
            show_panel(&inspector, panel);
        }
    }
    Ok(())
}

fn show_panel(inspector: &AnyObject, panel: DevToolsPanel) {
    // SAFETY: _WKInspector inherits NSObject's selector query, which has no preconditions.
    let has_heap_view: bool =
        unsafe { msg_send![inspector, respondsToSelector: sel!(showHeapAllocations)] };
    if panel == DevToolsPanel::Memory && has_heap_view {
        // SAFETY: the checked selector opens the existing Heap Allocations timeline.
        let () = unsafe { msg_send![inspector, showHeapAllocations] };
    } else {
        let tab = NSString::from_str(tab_identifier(panel));
        // SAFETY: I1's `showTab:` takes any string and ignores one it does not know.
        let () = unsafe { msg_send![inspector, showTab: &*tab] };
    }
}

/// The tab of Web Inspector that shows `panel`: Performance is the Timelines tab, Application
/// the Storage tab, and Memory, which `WebKit` records as a timeline, the Timelines tab too.
pub(crate) fn tab_identifier(panel: DevToolsPanel) -> &'static str {
    match panel {
        DevToolsPanel::Elements => "elements",
        DevToolsPanel::Console => "console",
        DevToolsPanel::Network => "network",
        DevToolsPanel::Sources => "sources",
        DevToolsPanel::Performance | DevToolsPanel::Memory => "timeline",
        DevToolsPanel::Application => "storage",
    }
}

/// The page's `_WKInspector`, on the fork only.
fn inspector(view: &WKWebView) -> Result<Retained<AnyObject>, EngineError> {
    if !on_fork() {
        return Err(EngineError(
            "this WebKit opens Web Inspector only from Safari: Develop → Quickie → this page"
                .to_owned(),
        ));
    }
    // SAFETY: the fork's `WKWebView` answers `_inspector` with its page's inspector or nil.
    let inspector: Option<Retained<AnyObject>> = unsafe { msg_send![view, _inspector] };
    inspector.ok_or_else(|| EngineError("the page has no Web Inspector".to_owned()))
}
