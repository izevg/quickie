//! The drag areas: invisible views that move the window where the page does not cover it.
//!
//! - The strip ([`DragArea::strip`]) is a band along the window's top edge, since the page
//!   fills the window up to its top and takes every click there. It sits above the page and
//!   below the overlays: the page's frame is at the bottom of the content view and the overlays
//!   are added on top later, so a palette, sheet, or scrim over the strip still gets its
//!   clicks. With *Inset page* on it lies in the band above the card, which it never covers.
//! - The backdrop ([`DragArea::backdrop`]) fills the window under the page's frame: with
//!   *Inset page* on, the inset around the card is the window's chrome and moves it.
//! - They draw nothing, change no cursor, and react to nothing but a press: no hover. The
//!   window buttons live in the titlebar, above the whole content view, and keep their own
//!   behaviour.
//! - In full screen they are not there: hidden from the moment the window starts entering full
//!   screen until it has left, so the page gets the whole screen. `AppKit` posts nothing when
//!   entering fails, so then they stay hidden until the window next leaves full screen.
//! - The trade-off: edge to edge, the page loses the [`HEIGHT`] points under the strip to
//!   clicks. Kept thin for that.
//!
//! A press drags through the system (`performWindowDragWithEvent:`), so snapping and tiling
//! work as from a titlebar. A double click does what the user chose in System Settings, Desktop
//! & Dock, "Double-click a window's title bar to".

use objc2::rc::Retained;
use objc2::runtime::NSObjectProtocol;
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSEvent, NSView, NSWindowDidExitFullScreenNotification,
    NSWindowStyleMask, NSWindowWillEnterFullScreenNotification,
};
use objc2_foundation::{
    NSNotification, NSNotificationCenter, NSPoint, NSRect, NSSize, NSString, NSUserDefaults,
};

/// How tall the strip is, in points.
pub const HEIGHT: f64 = 10.0;

/// What a double click on a drag area does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DoubleClick {
    Zoom,
    Minimize,
    Nothing,
}

impl DoubleClick {
    /// The user's choice, from `AppleActionOnDoubleClick` (`Maximize`, `Fill`, `Minimize`,
    /// `None`), else the older `AppleMiniaturizeOnDoubleClick`. `Fill` zooms: `zoom:` fills the
    /// screen's visible frame, and `AppKit` has no public call for the system's own fill.
    #[must_use]
    pub fn from_defaults(action: Option<&str>, miniaturize: bool) -> Self {
        match action {
            Some("Minimize") => Self::Minimize,
            Some("None") => Self::Nothing,
            None if miniaturize => Self::Minimize,
            Some(_) | None => Self::Zoom,
        }
    }

    fn of_user() -> Self {
        let defaults = NSUserDefaults::standardUserDefaults();
        let action = defaults
            .stringForKey(&NSString::from_str("AppleActionOnDoubleClick"))
            .map(|action| action.to_string());
        let miniaturize = defaults.boolForKey(&NSString::from_str("AppleMiniaturizeOnDoubleClick"));
        Self::from_defaults(action.as_deref(), miniaturize)
    }
}

define_class!(
    // SAFETY: `NSView` has no subclassing requirements, and `DragArea` implements no `Drop`.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "QuickieDragArea"]
    pub struct DragArea;

    unsafe impl NSObjectProtocol for DragArea {}

    impl DragArea {
        /// Follows its window in and out of full screen. The center drops a selector observer
        /// when it is deallocated, so a closed window's area leaves nothing behind.
        #[unsafe(method(viewDidMoveToWindow))]
        fn view_did_move_to_window(&self) {
            // SAFETY: `viewDidMoveToWindow` is `NSView`'s, called by AppKit on the main thread.
            let () = unsafe { msg_send![super(self), viewDidMoveToWindow] };
            let center = NSNotificationCenter::defaultCenter();
            // SAFETY: `self` is a live object; removing an observer that is not there is a no-op.
            unsafe { center.removeObserver(self) };
            let Some(window) = self.window() else {
                return;
            };
            // SAFETY: an immutable AppKit constant, read after AppKit has loaded.
            let will_enter = unsafe { NSWindowWillEnterFullScreenNotification };
            // SAFETY: as above.
            let did_exit = unsafe { NSWindowDidExitFullScreenNotification };
            for (selector, name) in [
                (sel!(fullScreenWillEnter:), will_enter),
                (sel!(fullScreenDidExit:), did_exit),
            ] {
                // SAFETY: the selector is a method of this class that takes one
                // `NSNotification`, and the object is the window the notification is about.
                unsafe {
                    center.addObserver_selector_name_object(self, selector, Some(name), Some(&window));
                }
            }
            self.setHidden(window.styleMask().contains(NSWindowStyleMask::FullScreen));
        }

        #[unsafe(method(fullScreenWillEnter:))]
        fn full_screen_will_enter(&self, _notification: &NSNotification) {
            self.setHidden(true);
        }

        #[unsafe(method(fullScreenDidExit:))]
        fn full_screen_did_exit(&self, _notification: &NSNotification) {
            self.setHidden(false);
        }

        /// The press moves the window even while it is not the key window, as a titlebar does.
        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }

        /// The area moves the window itself in `mouseDown:`. Saying no here keeps AppKit from
        /// moving it too, or from zooming on a double click of its own.
        #[unsafe(method(mouseDownCanMoveWindow))]
        fn mouse_down_can_move_window(&self) -> bool {
            false
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            let Some(window) = self.window() else {
                return;
            };
            if event.clickCount() < 2 {
                window.performWindowDragWithEvent(event);
                return;
            }
            match DoubleClick::of_user() {
                DoubleClick::Zoom => window.zoom(None),
                DoubleClick::Minimize => window.miniaturize(None),
                DoubleClick::Nothing => {}
            }
        }
    }
);

impl DragArea {
    /// A strip across the top of `host`, following its width as the window resizes.
    pub fn strip(host: &NSView, mtm: MainThreadMarker) -> Retained<Self> {
        let bounds = host.bounds();
        let frame = NSRect::new(
            NSPoint::new(0.0, bounds.size.height - HEIGHT),
            NSSize::new(bounds.size.width, HEIGHT),
        );
        Self::with_frame(
            frame,
            NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewMinYMargin,
            mtm,
        )
    }

    /// An area over the whole of `host`, following it as the window resizes.
    pub fn backdrop(host: &NSView, mtm: MainThreadMarker) -> Retained<Self> {
        Self::with_frame(
            host.bounds(),
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
            mtm,
        )
    }

    fn with_frame(
        frame: NSRect,
        resizing: NSAutoresizingMaskOptions,
        mtm: MainThreadMarker,
    ) -> Retained<Self> {
        // SAFETY: `initWithFrame:` is `NSView`'s designated initializer; there are no ivars.
        let this: Retained<Self> = unsafe { msg_send![Self::alloc(mtm), initWithFrame: frame] };
        this.setAutoresizingMask(resizing);
        this
    }
}

#[cfg(test)]
mod tests {
    use super::DoubleClick;

    #[test]
    fn follows_the_users_choice_over_the_older_setting() {
        for (action, expected) in [
            ("Maximize", DoubleClick::Zoom),
            ("Fill", DoubleClick::Zoom),
            ("Minimize", DoubleClick::Minimize),
            ("None", DoubleClick::Nothing),
        ] {
            for miniaturize in [false, true] {
                assert_eq!(
                    DoubleClick::from_defaults(Some(action), miniaturize),
                    expected
                );
            }
        }
    }

    #[test]
    fn falls_back_to_the_older_setting_then_zoom() {
        assert_eq!(
            DoubleClick::from_defaults(None, true),
            DoubleClick::Minimize
        );
        assert_eq!(DoubleClick::from_defaults(None, false), DoubleClick::Zoom);
    }
}
