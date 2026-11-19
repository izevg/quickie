//! The browser window: content first. The page fills the window edge to edge under a
//! transparent, hidden titlebar with the traffic lights hidden; overlays float above it.

use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSBackingStoreType, NSView, NSWindow, NSWindowButton, NSWindowStyleMask,
    NSWindowTitleVisibility,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

pub const APP_NAME: &str = "Quickie";

/// A new, hidden browser window whose content view hosts the pages and overlays.
pub fn browser_window(mtm: MainThreadMarker) -> Retained<NSWindow> {
    let style = NSWindowStyleMask::Titled
        | NSWindowStyleMask::Closable
        | NSWindowStyleMask::Miniaturizable
        | NSWindowStyleMask::Resizable
        | NSWindowStyleMask::FullSizeContentView;
    let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1280.0, 800.0));
    // SAFETY: a fresh allocation, a valid style mask, and buffered backing.
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            frame,
            style,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    // SAFETY: `Retained` owns the window, so AppKit must not release it on close as well.
    unsafe { window.setReleasedWhenClosed(false) };
    window.setContentView(Some(&NSView::new(mtm)));
    set_title(&window, "");
    window.setTitleVisibility(NSWindowTitleVisibility::Hidden);
    window.setTitlebarAppearsTransparent(true);
    window.setMinSize(NSSize::new(480.0, 320.0));
    for button in [
        NSWindowButton::CloseButton,
        NSWindowButton::MiniaturizeButton,
        NSWindowButton::ZoomButton,
    ] {
        if let Some(button) = window.standardWindowButton(button) {
            button.setHidden(true);
        }
    }
    window
}

/// Names the window after its page, for the Window menu and Mission Control; the title bar
/// itself stays hidden.
pub fn set_title(window: &NSWindow, title: &str) {
    let title = if title.is_empty() { APP_NAME } else { title };
    window.setTitle(&NSString::from_str(title));
}
