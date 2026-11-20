//! The extension controller's delegate: what extensions ask of the browser. Windows and tabs
//! are answered from what the shell last arranged; anything that needs the user (a permission,
//! a popup) or the session (a tab to open) waits as an [`ExtensionEvent`], its completion
//! handler kept until the shell answers. Only public delegate methods; native messaging is not
//! offered, so `WebKit` answers extensions that ask for it with an error.

use std::collections::BTreeSet;
use std::ptr::NonNull;
use std::rc::Weak;

use block2::DynBlock;
use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send};
use objc2_foundation::{NSArray, NSDate, NSError, NSSet, NSString, NSURL};
use objc2_web_kit::{
    WKWebExtensionAction, WKWebExtensionContext, WKWebExtensionController,
    WKWebExtensionControllerDelegate, WKWebExtensionMatchPattern, WKWebExtensionTab,
    WKWebExtensionTabConfiguration, WKWebExtensionWindow,
};
use quickie_application::ExtensionEvent;
use quickie_domain_settings::{ExtensionId, Grants};
use url::Url;

use super::manifest::patterns;
use super::tabs::ExtensionWindow;
use super::{Opening, Prompt, State};

/// Where a tab opened without a URL goes.
const BLANK: &str = "about:blank";

pub(crate) struct Ivars {
    state: Weak<State>,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements, and `ControllerDelegate` implements
    // no `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "QuickieExtensionControllerDelegate"]
    #[ivars = Ivars]
    pub(crate) struct ControllerDelegate;

    unsafe impl NSObjectProtocol for ControllerDelegate {}

    // SAFETY: every method matches the protocol's signature; the rest are optional.
    unsafe impl WKWebExtensionControllerDelegate for ControllerDelegate {
        #[unsafe(method_id(webExtensionController:openWindowsForExtensionContext:))]
        fn open_windows(
            &self,
            _controller: &WKWebExtensionController,
            _context: &WKWebExtensionContext,
        ) -> Retained<NSArray<ProtocolObject<dyn WKWebExtensionWindow>>> {
            let windows: Vec<Retained<ProtocolObject<dyn WKWebExtensionWindow>>> = self
                .windows()
                .into_iter()
                .map(ProtocolObject::from_retained)
                .collect();
            NSArray::from_retained_slice(&windows)
        }

        #[unsafe(method_id(webExtensionController:focusedWindowForExtensionContext:))]
        fn focused_window(
            &self,
            _controller: &WKWebExtensionController,
            _context: &WKWebExtensionContext,
        ) -> Option<Retained<ProtocolObject<dyn WKWebExtensionWindow>>> {
            self.windows()
                .into_iter()
                .next()
                .map(ProtocolObject::from_retained)
        }

        #[unsafe(method(webExtensionController:openNewTabUsingConfiguration:forExtensionContext:completionHandler:))]
        fn open_tab(
            &self,
            _controller: &WKWebExtensionController,
            configuration: &WKWebExtensionTabConfiguration,
            context: &WKWebExtensionContext,
            done: &DynBlock<dyn Fn(*mut ProtocolObject<dyn WKWebExtensionTab>, *mut NSError)>,
        ) {
            // SAFETY: getters with no preconditions.
            let url = unsafe { configuration.url() };
            // SAFETY: as above.
            let active = unsafe { configuration.shouldBeActive() };
            let url = url
                .and_then(|url| url.absoluteString())
                .and_then(|url| Url::parse(&url.to_string()).ok())
                .or_else(|| Url::parse(BLANK).ok());
            let (Some(state), Some(id), Some(url)) =
                (self.ivars().state.upgrade(), id_of(context), url)
            else {
                done.call((std::ptr::null_mut(), std::ptr::null_mut()));
                return;
            };
            let token = state.token();
            state
                .opens
                .borrow_mut()
                .insert(token, Opening::Tab(done.copy()));
            State::report(
                &state,
                ExtensionEvent::OpenTab {
                    token,
                    id,
                    url,
                    active,
                },
            );
        }

        #[unsafe(method(webExtensionController:openOptionsPageForExtensionContext:completionHandler:))]
        fn open_options(
            &self,
            _controller: &WKWebExtensionController,
            context: &WKWebExtensionContext,
            done: &DynBlock<dyn Fn(*mut NSError)>,
        ) {
            // SAFETY: a getter with no preconditions.
            let page = unsafe { context.optionsPageURL() }
                .and_then(|url| url.absoluteString())
                .and_then(|url| Url::parse(&url.to_string()).ok());
            let (Some(state), Some(id), Some(url)) =
                (self.ivars().state.upgrade(), id_of(context), page)
            else {
                done.call((std::ptr::null_mut(),));
                return;
            };
            let token = state.token();
            state
                .opens
                .borrow_mut()
                .insert(token, Opening::Page(done.copy()));
            State::report(
                &state,
                ExtensionEvent::OpenTab {
                    token,
                    id,
                    url,
                    active: true,
                },
            );
        }

        #[unsafe(method(webExtensionController:promptForPermissions:inTab:forExtensionContext:completionHandler:))]
        fn prompt_permissions(
            &self,
            _controller: &WKWebExtensionController,
            permissions: &NSSet<NSString>,
            tab: Option<&ProtocolObject<dyn WKWebExtensionTab>>,
            context: &WKWebExtensionContext,
            done: &DynBlock<dyn Fn(NonNull<NSSet<NSString>>, *mut NSDate)>,
        ) {
            let asked: Vec<Retained<NSString>> = permissions.iter().collect();
            let request = Grants {
                permissions: asked.iter().map(ToString::to_string).collect(),
                patterns: BTreeSet::new(),
            };
            self.ask(
                context,
                tab,
                request,
                Prompt::Permissions(done.copy(), asked),
            );
        }

        #[unsafe(method(webExtensionController:promptForPermissionToAccessURLs:inTab:forExtensionContext:completionHandler:))]
        fn prompt_urls(
            &self,
            _controller: &WKWebExtensionController,
            urls: &NSSet<NSURL>,
            tab: Option<&ProtocolObject<dyn WKWebExtensionTab>>,
            context: &WKWebExtensionContext,
            done: &DynBlock<dyn Fn(NonNull<NSSet<NSURL>>, *mut NSDate)>,
        ) {
            let asked: Vec<(Retained<NSURL>, String)> = urls
                .iter()
                .filter_map(|url| {
                    let host = url.host()?.to_string();
                    Some((url, format!("*://{host}/*")))
                })
                .collect();
            let request = Grants {
                permissions: BTreeSet::new(),
                patterns: asked.iter().map(|(_, pattern)| pattern.clone()).collect(),
            };
            self.ask(context, tab, request, Prompt::Urls(done.copy(), asked));
        }

        #[unsafe(method(webExtensionController:promptForPermissionMatchPatterns:inTab:forExtensionContext:completionHandler:))]
        fn prompt_patterns(
            &self,
            _controller: &WKWebExtensionController,
            asked: &NSSet<WKWebExtensionMatchPattern>,
            tab: Option<&ProtocolObject<dyn WKWebExtensionTab>>,
            context: &WKWebExtensionContext,
            done: &DynBlock<dyn Fn(NonNull<NSSet<WKWebExtensionMatchPattern>>, *mut NSDate)>,
        ) {
            let request = Grants {
                permissions: BTreeSet::new(),
                patterns: patterns(asked),
            };
            let asked = asked.iter().collect();
            self.ask(context, tab, request, Prompt::Patterns(done.copy(), asked));
        }

        #[unsafe(method(webExtensionController:presentPopupForAction:forExtensionContext:completionHandler:))]
        fn present_popup(
            &self,
            _controller: &WKWebExtensionController,
            action: &WKWebExtensionAction,
            context: &WKWebExtensionContext,
            done: &DynBlock<dyn Fn(*mut NSError)>,
        ) {
            if let (Some(state), Some(id)) = (self.ivars().state.upgrade(), id_of(context)) {
                *state.popup.borrow_mut() = Some((id.clone(), action.retain()));
                State::report(&state, ExtensionEvent::Popup { id });
            }
            done.call((std::ptr::null_mut(),));
        }
    }
);

impl ControllerDelegate {
    pub(crate) fn new(state: Weak<State>, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars { state });
        // SAFETY: `init` is `NSObject`'s designated initializer and the ivars are set.
        unsafe { msg_send![super(this), init] }
    }

    /// The windows as last arranged, the focused one first.
    fn windows(&self) -> Vec<Retained<ExtensionWindow>> {
        self.ivars()
            .state
            .upgrade()
            .map_or_default(|state| state.windows.borrow().clone())
    }

    /// Keeps a permission request's reply and reports it; a request nobody can answer gets
    /// nothing at once.
    fn ask(
        &self,
        context: &WKWebExtensionContext,
        tab: Option<&ProtocolObject<dyn WKWebExtensionTab>>,
        request: Grants,
        prompt: Prompt,
    ) {
        let (Some(state), Some(id)) = (self.ivars().state.upgrade(), id_of(context)) else {
            prompt.answer(&Grants::new());
            return;
        };
        let tab = tab.and_then(|tab| state.tab_of(tab));
        let token = state.token();
        state.prompts.borrow_mut().insert(token, prompt);
        State::report(
            &state,
            ExtensionEvent::Asked {
                token,
                id,
                tab,
                request,
            },
        );
    }
}

/// The id Quickie gave the context when it loaded it.
fn id_of(context: &WKWebExtensionContext) -> Option<ExtensionId> {
    // SAFETY: a getter with no preconditions.
    let id = unsafe { context.uniqueIdentifier() };
    ExtensionId::try_from(id.to_string().as_str()).ok()
}
