//! Tabs and windows as extensions see them (`chrome.tabs`, `chrome.windows`): one object per
//! browser tab and window, made from what the shell last arranged
//! ([`super::WebKitExtensions`]'s `arrange`). What an extension does to them (activate, close)
//! becomes an [`ExtensionEvent`] for the shell; loading and moving through history are the
//! page's own and happen at once.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use block2::DynBlock;
use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send};
use objc2_app_kit::NSWindow;
use objc2_foundation::{NSArray, NSError, NSRect, NSString, NSUInteger, NSURL, NSURLRequest};
use objc2_web_kit::{
    WKWebExtensionContext, WKWebExtensionTab, WKWebExtensionWindow, WKWebExtensionWindowState,
    WKWebExtensionWindowType, WKWebView,
};
use quickie_application::ExtensionEvent;
use quickie_domain_browsing::{TabId, WindowId};

use super::State;

/// What a navigation an extension asked for does.
enum Go {
    Load(Retained<NSURL>),
    Reload { from_origin: bool },
    Back,
    Forward,
}

pub(crate) struct TabIvars {
    tab: TabId,
    state: Weak<State>,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements, and `ExtensionTab` implements no
    // `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "QuickieExtensionTab"]
    #[ivars = TabIvars]
    pub(crate) struct ExtensionTab;

    unsafe impl NSObjectProtocol for ExtensionTab {}

    // SAFETY: every method matches the protocol's signature; the rest are optional.
    unsafe impl WKWebExtensionTab for ExtensionTab {
        #[unsafe(method_id(windowForWebExtensionContext:))]
        fn window_for(
            &self,
            _context: &WKWebExtensionContext,
        ) -> Option<Retained<ProtocolObject<dyn WKWebExtensionWindow>>> {
            self.window().map(ProtocolObject::from_retained)
        }

        #[unsafe(method(indexInWindowForWebExtensionContext:))]
        fn index_in_window(&self, _context: &WKWebExtensionContext) -> NSUInteger {
            self.index().unwrap_or(NSUInteger::MAX)
        }

        #[unsafe(method_id(webViewForWebExtensionContext:))]
        fn web_view_for(&self, _context: &WKWebExtensionContext) -> Option<Retained<WKWebView>> {
            self.web_view()
        }

        #[unsafe(method_id(titleForWebExtensionContext:))]
        fn title_for(&self, _context: &WKWebExtensionContext) -> Option<Retained<NSString>> {
            self.title()
        }

        #[unsafe(method_id(urlForWebExtensionContext:))]
        fn url_for(&self, _context: &WKWebExtensionContext) -> Option<Retained<NSURL>> {
            self.url()
        }

        #[unsafe(method(isLoadingCompleteForWebExtensionContext:))]
        fn loaded(&self, _context: &WKWebExtensionContext) -> bool {
            self.is_loaded()
        }

        #[unsafe(method(isSelectedForWebExtensionContext:))]
        fn selected(&self, _context: &WKWebExtensionContext) -> bool {
            self.is_selected()
        }

        #[unsafe(method(loadURL:forWebExtensionContext:completionHandler:))]
        fn load(
            &self,
            url: &NSURL,
            _context: &WKWebExtensionContext,
            done: &DynBlock<dyn Fn(*mut NSError)>,
        ) {
            self.go(Go::Load(url.retain()));
            done.call((std::ptr::null_mut(),));
        }

        #[unsafe(method(reloadFromOrigin:forWebExtensionContext:completionHandler:))]
        fn reload(
            &self,
            from_origin: bool,
            _context: &WKWebExtensionContext,
            done: &DynBlock<dyn Fn(*mut NSError)>,
        ) {
            self.go(Go::Reload { from_origin });
            done.call((std::ptr::null_mut(),));
        }

        #[unsafe(method(goBackForWebExtensionContext:completionHandler:))]
        fn back(&self, _context: &WKWebExtensionContext, done: &DynBlock<dyn Fn(*mut NSError)>) {
            self.go(Go::Back);
            done.call((std::ptr::null_mut(),));
        }

        #[unsafe(method(goForwardForWebExtensionContext:completionHandler:))]
        fn forward(&self, _context: &WKWebExtensionContext, done: &DynBlock<dyn Fn(*mut NSError)>) {
            self.go(Go::Forward);
            done.call((std::ptr::null_mut(),));
        }

        #[unsafe(method(activateForWebExtensionContext:completionHandler:))]
        fn activate(
            &self,
            _context: &WKWebExtensionContext,
            done: &DynBlock<dyn Fn(*mut NSError)>,
        ) {
            self.report(ExtensionEvent::ActivateTab(self.ivars().tab));
            done.call((std::ptr::null_mut(),));
        }

        #[unsafe(method(closeForWebExtensionContext:completionHandler:))]
        fn close(&self, _context: &WKWebExtensionContext, done: &DynBlock<dyn Fn(*mut NSError)>) {
            self.report(ExtensionEvent::CloseTab(self.ivars().tab));
            done.call((std::ptr::null_mut(),));
        }
    }
);

impl ExtensionTab {
    pub(crate) fn new(tab: TabId, state: Weak<State>, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(TabIvars { tab, state });
        // SAFETY: `init` is `NSObject`'s designated initializer and the ivars are set.
        unsafe { msg_send![super(this), init] }
    }

    fn state(&self) -> Option<Rc<State>> {
        self.ivars().state.upgrade()
    }

    fn window(&self) -> Option<Retained<ExtensionWindow>> {
        let state = self.state()?;
        let windows = state.windows.borrow();
        windows
            .iter()
            .find(|window| window.ivars().tabs.borrow().contains(&self.ivars().tab))
            .cloned()
    }

    fn index(&self) -> Option<NSUInteger> {
        let window = self.window()?;
        let tabs = window.ivars().tabs.borrow();
        tabs.iter().position(|&tab| tab == self.ivars().tab)
    }

    fn web_view(&self) -> Option<Retained<WKWebView>> {
        let state = self.state()?;
        let views = state.shared.views.borrow();
        views.get(&self.ivars().tab).cloned()
    }

    fn title(&self) -> Option<Retained<NSString>> {
        let view = self.web_view()?;
        // SAFETY: a getter with no preconditions.
        unsafe { view.title() }
    }

    fn url(&self) -> Option<Retained<NSURL>> {
        let view = self.web_view()?;
        // SAFETY: a getter with no preconditions.
        unsafe { view.URL() }
    }

    fn is_loaded(&self) -> bool {
        // SAFETY: a getter with no preconditions.
        self.web_view()
            .is_some_and(|view| !unsafe { view.isLoading() })
    }

    fn is_selected(&self) -> bool {
        self.window()
            .is_some_and(|window| window.ivars().active.get() == Some(self.ivars().tab))
    }

    fn go(&self, go: Go) {
        let Some(view) = self.web_view() else {
            return;
        };
        // Plain navigations of a live view; the navigation each returns is not needed.
        let navigation = match go {
            // SAFETY: a valid request.
            Go::Load(url) => unsafe { view.loadRequest(&NSURLRequest::requestWithURL(&url)) },
            // SAFETY: no preconditions.
            Go::Reload { from_origin: true } => unsafe { view.reloadFromOrigin() },
            // SAFETY: no preconditions.
            Go::Reload { from_origin: false } => unsafe { view.reload() },
            // SAFETY: no preconditions.
            Go::Back => unsafe { view.goBack() },
            // SAFETY: no preconditions.
            Go::Forward => unsafe { view.goForward() },
        };
        drop(navigation);
    }

    fn report(&self, event: ExtensionEvent) {
        if let Some(state) = self.state() {
            State::report(&state, event);
        }
    }
}

pub(crate) struct WindowIvars {
    pub(crate) window: WindowId,
    pub(crate) tabs: RefCell<Vec<TabId>>,
    pub(crate) active: Cell<Option<TabId>>,
    state: Weak<State>,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements, and `ExtensionWindow` implements no
    // `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "QuickieExtensionWindow"]
    #[ivars = WindowIvars]
    pub(crate) struct ExtensionWindow;

    unsafe impl NSObjectProtocol for ExtensionWindow {}

    // SAFETY: every method matches the protocol's signature; the rest are optional.
    unsafe impl WKWebExtensionWindow for ExtensionWindow {
        #[unsafe(method_id(tabsForWebExtensionContext:))]
        fn tabs_for(
            &self,
            _context: &WKWebExtensionContext,
        ) -> Retained<NSArray<ProtocolObject<dyn WKWebExtensionTab>>> {
            self.tab_list()
        }

        #[unsafe(method_id(activeTabForWebExtensionContext:))]
        fn active_for(
            &self,
            _context: &WKWebExtensionContext,
        ) -> Option<Retained<ProtocolObject<dyn WKWebExtensionTab>>> {
            self.active_tab()
        }

        #[unsafe(method(windowTypeForWebExtensionContext:))]
        fn kind(&self, _context: &WKWebExtensionContext) -> WKWebExtensionWindowType {
            WKWebExtensionWindowType::Normal
        }

        #[unsafe(method(windowStateForWebExtensionContext:))]
        fn shown(&self, _context: &WKWebExtensionContext) -> WKWebExtensionWindowState {
            WKWebExtensionWindowState::Normal
        }

        /// Throwaway jars are not windows of their own; their tabs are not listed.
        #[unsafe(method(isPrivateForWebExtensionContext:))]
        fn private(&self, _context: &WKWebExtensionContext) -> bool {
            false
        }

        #[unsafe(method(frameForWebExtensionContext:))]
        fn frame(&self, _context: &WKWebExtensionContext) -> NSRect {
            self.window_frame()
        }

        #[unsafe(method(screenFrameForWebExtensionContext:))]
        fn screen_frame(&self, _context: &WKWebExtensionContext) -> NSRect {
            self.screen_rect()
        }

        #[unsafe(method(focusForWebExtensionContext:completionHandler:))]
        fn focus(&self, _context: &WKWebExtensionContext, done: &DynBlock<dyn Fn(*mut NSError)>) {
            self.bring_forward();
            done.call((std::ptr::null_mut(),));
        }
    }
);

impl ExtensionWindow {
    pub(crate) fn new(
        window: WindowId,
        state: Weak<State>,
        mtm: MainThreadMarker,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(WindowIvars {
            window,
            tabs: RefCell::new(Vec::new()),
            active: Cell::new(None),
            state,
        });
        // SAFETY: `init` is `NSObject`'s designated initializer and the ivars are set.
        unsafe { msg_send![super(this), init] }
    }

    fn tab(&self, tab: TabId) -> Option<Retained<ProtocolObject<dyn WKWebExtensionTab>>> {
        let state = self.ivars().state.upgrade()?;
        let tabs = state.tabs.borrow();
        tabs.get(&tab)
            .map(|tab| ProtocolObject::from_retained(tab.clone()))
    }

    fn tab_list(&self) -> Retained<NSArray<ProtocolObject<dyn WKWebExtensionTab>>> {
        let tabs: Vec<Retained<ProtocolObject<dyn WKWebExtensionTab>>> = self
            .ivars()
            .tabs
            .borrow()
            .iter()
            .filter_map(|&tab| self.tab(tab))
            .collect();
        NSArray::from_retained_slice(&tabs)
    }

    fn active_tab(&self) -> Option<Retained<ProtocolObject<dyn WKWebExtensionTab>>> {
        self.tab(self.ivars().active.get()?)
    }

    fn bring_forward(&self) {
        if let (Some(tab), Some(state)) = (self.ivars().active.get(), self.ivars().state.upgrade())
        {
            State::report(&state, ExtensionEvent::ActivateTab(tab));
        }
    }

    fn window_frame(&self) -> NSRect {
        self.ns_window()
            .map_or(NSRect::ZERO, |window| window.frame())
    }

    fn screen_rect(&self) -> NSRect {
        self.ns_window()
            .and_then(|window| window.screen())
            .map_or(NSRect::ZERO, |screen| screen.frame())
    }

    /// The `NSWindow` its pages are in, through the page on screen.
    fn ns_window(&self) -> Option<Retained<NSWindow>> {
        let state = self.ivars().state.upgrade()?;
        let views = state.shared.views.borrow();
        self.ivars()
            .tabs
            .borrow()
            .iter()
            .filter_map(|tab| views.get(tab))
            .find_map(|view| view.window())
    }
}
