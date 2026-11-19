//! The browser window: content first. The page fills the window edge to edge under a
//! transparent, hidden titlebar with the traffic lights hidden; overlays float above it.

use std::ptr::NonNull;
use std::rc::Rc;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSAutoresizingMaskOptions, NSBackingStoreType, NSEvent, NSEventMask,
    NSEventModifierFlags, NSView, NSWindow, NSWindowButton, NSWindowStyleMask,
    NSWindowTitleVisibility,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use quickie_engine_webkit::{PageEvent, Storage, WebKitEngine, WebPage};

use crate::address::AddressOverlay;
use crate::input::destination::destination;
use crate::input::shortcut::{self, Action, Modifiers};

const APP_NAME: &str = "Quickie";

/// The start page: inline, with no external resources, so launching opens no connection.
const START_PAGE: &str = r#"<!doctype html>
<html><head><meta charset="utf-8"><title>Quickie</title>
<meta name="color-scheme" content="light dark">
<style>
  html, body { height: 100%; margin: 0; }
  body { display: grid; place-items: center; font: 15px -apple-system, system-ui, sans-serif;
         color: CanvasText; background: Canvas; }
  main { text-align: center; }
  h1 { font-size: 34px; font-weight: 600; letter-spacing: -0.5px; margin: 0 0 12px; }
  p { margin: 0; opacity: 0.6; }
  kbd { font: 13px ui-monospace, monospace; padding: 2px 6px; border-radius: 6px;
        border: 1px solid color-mix(in srgb, CanvasText 20%, transparent); }
</style></head>
<body><main><h1>Quickie</h1><p>Press <kbd>⌘L</kbd> to go somewhere.</p></main></body></html>
"#;

pub struct Browser {
    window: Retained<NSWindow>,
    page: WebPage,
    address: AddressOverlay,
    mtm: MainThreadMarker,
}

impl Browser {
    pub fn new(engine: &WebKitEngine, mtm: MainThreadMarker) -> Rc<Self> {
        let window = window(mtm);
        let titled = window.clone();
        let page = engine.create_page(Storage::Persistent, move |event| {
            if let PageEvent::TitleChanged(title) = event {
                let title = title.unwrap_or_else(|| APP_NAME.to_owned());
                titled.setTitle(&NSString::from_str(&title));
            }
        });
        let content = NSView::new(mtm);
        window.setContentView(Some(&content));
        let view = page.view();
        view.setFrame(content.bounds());
        view.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        content.addSubview(view);
        page.load_html(START_PAGE);
        window.makeFirstResponder(Some(view));
        Rc::new(Self {
            window,
            page,
            address: AddressOverlay::new(mtm),
            mtm,
        })
    }

    pub fn show(&self) {
        self.window.center();
        self.window.makeKeyAndOrderFront(None);
    }

    /// Handles a key press if it is a browser shortcut; the page never sees it then.
    fn key_down(&self, event: &NSEvent) -> bool {
        let action = shortcut::resolve(
            event.keyCode(),
            modifiers(event.modifierFlags()),
            self.address.is_open(),
        );
        action.is_some_and(|action| self.perform(action))
    }

    fn perform(&self, action: Action) -> bool {
        match action {
            Action::OpenAddress => self.open_address(),
            Action::Back => self.page.go_back(),
            Action::Forward => self.page.go_forward(),
            Action::Reload => self.page.reload(),
            Action::Quit => NSApplication::sharedApplication(self.mtm).terminate(None),
            Action::Submit => return self.submit(),
            Action::Dismiss => self.close_address(),
        }
        true
    }

    fn open_address(&self) {
        let Some(host) = self.window.contentView() else {
            return;
        };
        let url = self.page.url().filter(|url| url != "about:blank");
        self.address
            .show(&host, &self.window, url.as_deref().unwrap_or_default());
    }

    /// ⏎ in the overlay. While an input method composes, ⏎ is the field's.
    fn submit(&self) -> bool {
        if self.address.is_composing() {
            return false;
        }
        let target = destination(&self.address.text());
        self.close_address();
        if let Some(url) = target {
            // A destination is always an absolute URL, so the engine accepts it.
            let _ = self.page.load(&url);
        }
        true
    }

    fn close_address(&self) {
        self.address.hide();
        self.window.makeFirstResponder(Some(self.page.view()));
    }
}

/// Routes key presses to the browser before the window dispatches them to the page or a field.
/// The returned monitor must stay alive for as long as shortcuts should work.
pub fn install_shortcuts(browser: &Rc<Browser>) -> Option<Retained<AnyObject>> {
    let browser = Rc::clone(browser);
    let handler = RcBlock::new(move |event: NonNull<NSEvent>| -> *mut NSEvent {
        // SAFETY: AppKit passes a valid event that outlives this call.
        let key = unsafe { event.as_ref() };
        if browser.key_down(key) {
            std::ptr::null_mut()
        } else {
            event.as_ptr()
        }
    });
    // SAFETY: the handler returns either the event it was given or null, as the API requires,
    // and AppKit calls it on the main thread, where the captured browser lives.
    unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::KeyDown, &handler) }
}

fn modifiers(flags: NSEventModifierFlags) -> Modifiers {
    Modifiers::NONE
        .with(
            Modifiers::COMMAND,
            flags.contains(NSEventModifierFlags::Command),
        )
        .with(
            Modifiers::SHIFT,
            flags.contains(NSEventModifierFlags::Shift),
        )
        .with(
            Modifiers::OPTION,
            flags.contains(NSEventModifierFlags::Option),
        )
        .with(
            Modifiers::CONTROL,
            flags.contains(NSEventModifierFlags::Control),
        )
}

fn window(mtm: MainThreadMarker) -> Retained<NSWindow> {
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
    window.setTitle(&NSString::from_str(APP_NAME));
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
