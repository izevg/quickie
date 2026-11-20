//! The browser window: content first. The page fills the window edge to edge, the top where a
//! titlebar would be included: the titlebar is transparent and untitled, and the system's
//! close, minimize, and full-screen buttons float over the page's top-left corner, as in
//! Safari's compact layout. Overlays float above the page.
//!
//! - [`drag_strip`]: the invisible band along the top edge that moves the window.

mod drag_strip;

use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSBackingStoreType, NSView, NSWindow, NSWindowStyleMask, NSWindowTitleVisibility,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use drag_strip::DragStrip;

pub const APP_NAME: &str = "Quickie";

/// A new, hidden browser window whose content view, the whole window, hosts the pages and
/// overlays, with the drag strip between them.
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
    let content = NSView::new(mtm);
    window.setContentView(Some(&content));
    content.addSubview(&DragStrip::new(&content, mtm));
    set_title(&window, "");
    window.setTitleVisibility(NSWindowTitleVisibility::Hidden);
    window.setTitlebarAppearsTransparent(true);
    window.setMinSize(NSSize::new(480.0, 320.0));
    window
}

/// Names the window after its page, for the Window menu and Mission Control; the title itself
/// is never drawn.
pub fn set_title(window: &NSWindow, title: &str) {
    let title = if title.is_empty() { APP_NAME } else { title };
    window.setTitle(&NSString::from_str(title));
}
