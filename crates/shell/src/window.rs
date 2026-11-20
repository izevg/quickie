//! The browser window: content first. The page fills the window edge to edge, the top where a
//! titlebar would be included: the titlebar is transparent and untitled, and the system's
//! close, minimize, and full-screen buttons float over the page's top-left corner, as in
//! Safari's compact layout. Overlays float above the page. With *Inset page* on, the page is a
//! card inside the window ([`PageFrame`]) and the window buttons sit in the band above it.
//!
//! - [`drag_area`]: the invisible views that move the window: a band along the top edge, and
//!   the backdrop under the page's frame.

mod drag_area;

use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSBackingStoreType, NSView, NSWindow, NSWindowOrderingMode, NSWindowStyleMask,
    NSWindowTitleVisibility,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use quickie_ui_widgets::PageFrame;

use drag_area::DragArea;

pub const APP_NAME: &str = "Quickie";

/// A new, hidden browser window and its page frame. Its content view, the whole window, holds
/// from the bottom up the backdrop that moves the window, the page's frame (whose host the
/// engine fills), the drag strip, and later the overlays.
pub fn browser_window(mtm: MainThreadMarker) -> (Retained<NSWindow>, PageFrame) {
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
    let page = PageFrame::new(mtm);
    page.install(&content);
    content.addSubview_positioned_relativeTo(
        &DragArea::backdrop(&content, mtm),
        NSWindowOrderingMode::Below,
        Some(page.view()),
    );
    content.addSubview(&DragArea::strip(&content, mtm));
    set_title(&window, "");
    window.setTitleVisibility(NSWindowTitleVisibility::Hidden);
    window.setTitlebarAppearsTransparent(true);
    window.setMinSize(NSSize::new(480.0, 320.0));
    (window, page)
}

/// Names the window after its page, for the Window menu and Mission Control; the title itself
/// is never drawn.
pub fn set_title(window: &NSWindow, title: &str) {
    let title = if title.is_empty() { APP_NAME } else { title };
    window.setTitle(&NSString::from_str(title));
}
