//! Quickie in the system: the links and files other apps open in it (`open -a Quickie …`, a
//! link clicked in Mail, `open page.html`), asking macOS to make it the default browser, and
//! the profile folder in Finder. URLs and files named on the command line open the same way.
//!
//! macOS asks the user itself before it changes the default browser; Quickie only makes the
//! request, and only from the app bundle (`cargo xtask bundle`), never from a bare binary.

#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use block2::RcBlock;
use dispatch2::{DispatchQueue, MainThreadBound};
use objc2::MainThreadMarker;
use objc2_app_kit::NSWorkspace;
use objc2_foundation::{NSBundle, NSError, NSString, NSURL};
use quickie_domain_browsing::{Focus, JarChoice};
use url::Url;

use super::Browser;

/// The schemes a default browser opens.
pub const WEB_SCHEMES: [&str; 2] = ["http", "https"];

/// What other apps may hand Quickie to show in a tab: web pages and local files.
const OPENABLE: [&str; 3] = ["http", "https", "file"];

/// True when other apps may open `url` in a tab.
#[must_use]
pub fn openable(url: &Url) -> bool {
    OPENABLE.contains(&url.scheme())
}

/// What the command line asks to open, as `quickie https://example.com page.html` would: a
/// URL Quickie opens as it is, a path to an existing file (relative to `cwd`) as a file URL.
/// Flags (`-NSDocumentRevisionsDebugMode YES` from a debugger) and anything else are skipped.
#[must_use]
pub fn launch_urls(args: &[String], cwd: &Path) -> Vec<Url> {
    args.iter()
        .filter(|arg| !arg.starts_with('-'))
        .filter_map(|arg| match Url::parse(arg) {
            Ok(url) if openable(&url) => Some(url),
            _ => {
                let path = cwd.join(arg);
                path.is_file()
                    .then(|| Url::from_file_path(&path).ok())
                    .flatten()
            }
        })
        .collect()
}

/// The request that makes an app the default browser: the app bundle, for every web scheme.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DefaultBrowserRequest {
    /// The `.app` bundle macOS will open web links with.
    pub app: PathBuf,
    /// The schemes to claim, in order.
    pub schemes: Vec<&'static str>,
}

impl DefaultBrowserRequest {
    /// The request for the app whose main bundle is at `bundle`, or `None` when the binary
    /// runs bare, outside an `.app` (`cargo run`): macOS cannot hand links to a bare binary.
    #[must_use]
    pub fn for_bundle(bundle: &Path) -> Option<Self> {
        let is_app = bundle
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("app"));
        is_app.then(|| Self {
            app: bundle.to_path_buf(),
            schemes: WEB_SCHEMES.to_vec(),
        })
    }
}

/// The main bundle's folder: `Quickie.app` for the bundle, `target/debug` for a bare binary.
fn main_bundle() -> Option<PathBuf> {
    NSBundle::mainBundle()
        .bundleURL()
        .path()
        .map(|path| PathBuf::from(path.to_string()))
}

fn file_url(path: &Path) -> objc2::rc::Retained<NSURL> {
    NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()))
}

/// True when macOS already opens `http` links with the app at `app`.
fn is_default_browser(app: &Path) -> bool {
    let Some(probe) = NSURL::URLWithString(&NSString::from_str("http://example.com/")) else {
        return false;
    };
    NSWorkspace::sharedWorkspace()
        .URLForApplicationToOpenURL(&probe)
        .and_then(|handler| handler.path())
        .is_some_and(|handler| Path::new(&handler.to_string()) == app)
}

impl Browser {
    /// Opens what another app asked for, each a foreground tab in the main jar of the key
    /// window, or of a new window when none is left. What Quickie cannot show is told in a
    /// toast.
    pub fn open_urls(&self, urls: Vec<Url>) {
        let (urls, refused): (Vec<Url>, Vec<Url>) = urls.into_iter().partition(openable);
        for url in refused {
            self.toast(&format!("Can't open {}: links", url.scheme()));
        }
        if urls.is_empty() {
            return;
        }
        self.close_overlays();
        self.hide_navigation();
        let window = if let Some((window, _)) = self.current() {
            window
        } else {
            let Some(window) = self.run(crate::App::open_window) else {
                return;
            };
            self.add_window(window);
            window
        };
        for url in urls {
            self.run(|browsing| browsing.open(window, url, JarChoice::Main, Focus::Foreground));
        }
        if let Some(ns_window) = self.window(window) {
            ns_window.makeKeyAndOrderFront(None);
        }
        self.settle(window);
    }

    /// Asks macOS to open web links with this app. The system confirms with the user, then
    /// the outcome is told in a toast.
    pub(super) fn set_default_browser(&self) {
        let Some(request) =
            main_bundle().and_then(|bundle| DefaultBrowserRequest::for_bundle(&bundle))
        else {
            self.toast("Only the app bundle can be the default browser: run `mise run bundle`");
            return;
        };
        if is_default_browser(&request.app) {
            self.toast("Quickie is already the default browser");
            return;
        }
        self.claim_schemes(request, 0);
    }

    /// Claims `request.schemes[next..]` one after the other: each answer comes back on the
    /// main queue before the next scheme is asked for.
    fn claim_schemes(&self, request: DefaultBrowserRequest, next: usize) {
        let Some(&scheme) = request.schemes.get(next) else {
            self.toast("Quickie is now the default browser");
            return;
        };
        let app = file_url(&request.app);
        let waiting = Mutex::new(Some((
            MainThreadBound::new(self.this.clone(), self.mtm),
            request,
        )));
        let answered = RcBlock::new(move |error: *mut NSError| {
            // SAFETY: the system passes nil or a valid error that lives through this call.
            let refused =
                unsafe { error.as_ref() }.map(|error| error.localizedDescription().to_string());
            let Some((browser, request)) = waiting.lock().ok().and_then(|mut slot| slot.take())
            else {
                return;
            };
            DispatchQueue::main().exec_async(move || {
                let Some(mtm) = MainThreadMarker::new() else {
                    return;
                };
                let Some(browser) = browser.into_inner(mtm).upgrade() else {
                    return;
                };
                match refused {
                    Some(reason) => {
                        browser.toast(&format!("Default browser not changed: {reason}"));
                    }
                    None => browser.claim_schemes(request, next + 1),
                }
            });
        });
        NSWorkspace::sharedWorkspace()
            .setDefaultApplicationAtURL_toOpenURLsWithScheme_completionHandler(
                &app,
                &NSString::from_str(scheme),
                Some(&answered),
            );
    }

    /// Opens the profile folder, `~/Library/Application Support/Quickie`, in Finder.
    pub(super) fn reveal_profile(&self) {
        if !NSWorkspace::sharedWorkspace().openURL(&file_url(&self.profile)) {
            self.toast(&format!("Couldn't show {}", self.profile.display()));
        }
    }
}
