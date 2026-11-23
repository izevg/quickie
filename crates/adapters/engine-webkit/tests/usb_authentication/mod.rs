//! Checks that a normal page receives the fork's `WebAuthentication` panel callback.

use objc2::sel;
use objc2_foundation::NSObjectProtocol;
use objc2_web_kit::WKWebView;
use quickie_application::conformance::Harness;
use quickie_application::{EnginePort, PageSurface};
use quickie_domain_browsing::{Jar, JarId, Profile, TabId};
use url::Url;

use super::WebKit;

pub(super) fn check(harness: &mut WebKit) -> Result<(), String> {
    let tab = TabId::new(Profile::Main, 1);
    let url = Url::parse("about:blank").map_err(|error| error.to_string())?;
    let mut engine = harness.engine();
    engine
        .create(tab, &url, Jar::Throwaway(JarId::new(1)))
        .map_err(|error| error.to_string())?;
    let view = engine
        .view(tab)
        .and_then(|view| view.downcast_ref::<WKWebView>())
        .ok_or("the tab has no web view")?;
    // SAFETY: the offscreen view is live on the main thread.
    let delegate = unsafe { view.UIDelegate() }.ok_or("the tab has no UI delegate")?;
    let implemented = delegate.respondsToSelector(sel!(
        _webView:runWebAuthenticationPanel:initiatedByFrame:completionHandler:
    ));
    if implemented {
        Ok(())
    } else {
        Err("the tab's delegate cannot receive a USB authentication panel".to_owned())
    }
}
