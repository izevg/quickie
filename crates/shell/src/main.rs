//! Quickie, the composition root: wires the `WebKit` engine, the `SQLite` store, the Keychain,
//! the LLM client, and the clock to the browsing, library, palette, shields, and agent use cases
//! and the keymap, puts them on screen, and runs the app.
//!
//! For now this is the development app on the `engine-webkit` adapter (`docs/architecture.md`
//! §3, P0). Run it from the repository root with:
//!
//! ```sh
//! mise exec -- cargo run -p quickie [-- <url or file>…]
//! mise run bundle && open target/debug/Quickie.app   # the app bundle, for LaunchServices
//! ```
//!
//! As a bundle it opens the links and files other apps hand it (`application:openURLs:`), and
//! the `App` commands ask macOS to make it the default browser or show the profile in Finder.
//!
//! The session, the history, the bookmarks, and the user's key bindings live in
//! `~/Library/Application Support/Quickie/shell.sqlite`. Every key is a command of the keymap
//! (`docs/design/inventory.md`, README "Default keys"): `⇧⌘P` / `⌘K` open the command palette,
//! `⌘P` goes to a tab, bookmark, or visited page, `⌘L` opens the address overlay, `⌘T` /
//! `⇧⌘N` open a tab / a throwaway tab, `⌘W` closes into the trash, `⌘N` / `⌘⇧W` open / close
//! a window, `⌘⇧T` reopens, `⌃⇥` /
//! `⌃⇧⇥` switch in most-recently-used order, `⌘1` … `⌘9` go to a pinned slot and `⌥⌘1` …
//! `⌥⌘9` pin there, `⌘D` bookmarks, `⇧⌘C` / `⌥⌘C` copy the URL / a Markdown link, `⌘J`
//! shows the downloads (saved to `~/Downloads`), `⌥⌘S` the site's shields, `?` in the palette
//! asks the agent about the page and `⌘.` stops it. `⌘Q` quits.
//! Launching opens no network connection; the start page is served from memory, and the agent
//! contacts its endpoint only when asked.

#![expect(
    unsafe_code,
    reason = "AppKit through objc2: windows, menus, the app delegate, timers, the key monitor, \
              the quit notification, the pasteboard, and the default browser request"
)]

mod browser;
mod delegate;
mod input;
mod menu;
mod window;

use std::path::PathBuf;
use std::process::ExitCode;
use std::ptr::NonNull;
use std::time::{SystemTime, UNIX_EPOCH};

use block2::RcBlock;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2::runtime::{NSObjectProtocol, ProtocolObject};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationWillTerminateNotification,
};
use objc2_foundation::{NSNotification, NSNotificationCenter};
use quickie_application::{Agent, Browsing, Clock, Library, LoadedKeymap, Shields, load_keymap};
use quickie_domain_browsing::Timestamp;
use quickie_domain_commands::{Catalog, Keymap, Platform};
use quickie_engine_webkit::{WebKitContentBlocker, WebKitEngine};
use quickie_store::{PendingWrites, WriteBehindStore};
use quickie_system::{KeychainSecrets, SystemEntropy};
use url::Url;

use browser::{Browser, Services, install_shortcuts, launch_urls};
use delegate::AppDelegate;

/// The browsing use cases as the app composes them.
pub type App = Browsing<WebKitEngine, WriteBehindStore, SystemClock>;

/// The library use cases as the app composes them.
pub type Lib = Library<WriteBehindStore, SystemClock>;

/// The palette query as the app composes it.
pub type Palette = quickie_application::Palette<SystemClock>;

/// The shields use cases as the app composes them.
pub type Guard = Shields<WriteBehindStore, SystemClock, WebKitContentBlocker>;

/// The desktop whose key conventions the keymap follows, chosen here once for the whole app.
pub const PLATFORM: Platform = Platform::MacOs;

/// The agent use cases as the app composes them: the endpoint in the store, its key in the
/// login Keychain, each request's fence marker from the system's generator.
pub type Agents = Agent<WriteBehindStore, KeychainSecrets, SystemEntropy>;

/// Where new tabs open: the start page, served by the engine from memory.
const HOME: &str = "quickie://start/";

/// The start page: no external resources, so launching opens no connection.
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

/// The wall clock.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        let since_epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        Timestamp::from_millis(u64::try_from(since_epoch.as_millis()).unwrap_or(u64::MAX))
    }
}

fn main() -> ExitCode {
    let Some(mtm) = MainThreadMarker::new() else {
        return ExitCode::FAILURE;
    };
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);

    let (browser, pending) = match start(mtm) {
        Ok(started) => started,
        Err(reason) => return fail(&reason),
    };
    // The application holds its delegate weakly: this binding keeps it for the app's lifetime.
    let delegate = AppDelegate::new(&browser, mtm);
    app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    menu::install(&app, &delegate, &Catalog::builtin(), mtm);
    let _flush_on_quit = flush_on_quit(pending);
    let _monitor = install_shortcuts(&browser);
    browser.show();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cwd = std::env::current_dir().unwrap_or_default();
    browser.open_urls(launch_urls(&args, &cwd));
    app.activate();
    app.run();
    ExitCode::SUCCESS
}

/// Opens the store, restores the session and the library, loads the keymap, and puts it all
/// on screen. The stores write on their own threads; the shell keeps handles on their pending
/// writes.
fn start(mtm: MainThreadMarker) -> Result<(std::rc::Rc<Browser>, Vec<PendingWrites>), String> {
    let home = Url::parse(HOME).map_err(|error| error.to_string())?;
    let store = WriteBehindStore::open(&store_path()?).map_err(|error| error.to_string())?;
    let pending = store.pending();
    let engine = WebKitEngine::new(&[(home.clone(), START_PAGE.to_owned())], mtm);
    let mut browsing =
        Browsing::start(engine, store, SystemClock, &home).map_err(|error| error.to_string())?;
    // The library and the key bindings have a writer, and a connection, of their own.
    let library_store =
        WriteBehindStore::open(&store_path()?).map_err(|error| error.to_string())?;
    let library_pending = library_store.pending();
    let catalog = Catalog::builtin();
    let (keymap, notices) = keymap(&library_store, &catalog);
    let library = Library::start(library_store, SystemClock).map_err(|error| error.to_string())?;
    // The shields too; they never fetch on their own (docs/privacy.md).
    let shields_store =
        WriteBehindStore::open(&store_path()?).map_err(|error| error.to_string())?;
    let shields_pending = shields_store.pending();
    let blocker = browsing
        .engine_mut()
        .content_blocker(&support_dir()?.join("ContentRules"))
        .map_err(|error| error.to_string())?;
    let shields =
        Shields::start(shields_store, SystemClock, blocker).map_err(|error| error.to_string())?;
    let downloads_folder = home_dir()?.join("Downloads");
    browsing
        .engine_mut()
        .set_download_directory(downloads_folder.clone());
    // The agent too. It contacts its endpoint only when asked (docs/privacy.md).
    let agent_store = WriteBehindStore::open(&store_path()?).map_err(|error| error.to_string())?;
    let agent_pending = agent_store.pending();
    let secrets = KeychainSecrets::login();
    let agent = Agent::start(agent_store, secrets.clone(), SystemEntropy)
        .map_err(|error| error.to_string())?;
    let writes = vec![pending, library_pending, shields_pending, agent_pending];
    let services = Services {
        browsing,
        library,
        shields,
        agent,
        secrets,
        palette: Palette::new(catalog.clone(), SystemClock),
        catalog,
        keymap,
        writes: writes.clone(),
        notices,
        downloads_folder,
        profile: support_dir()?,
    };
    Ok((Browser::new(services, home, mtm), writes))
}

/// The defaults with the user's bindings applied, and what to tell the user about entries
/// that were left out. A store that cannot be read leaves the defaults.
fn keymap(store: &WriteBehindStore, catalog: &Catalog) -> (Keymap, Vec<String>) {
    match load_keymap(store, catalog, PLATFORM) {
        Ok(LoadedKeymap { keymap, rejected }) => {
            let notices = rejected
                .iter()
                .map(|bad| format!("Key binding `{}` skipped: {}", bad.entry.key, bad.reason))
                .collect();
            (keymap, notices)
        }
        Err(error) => (
            Keymap::defaults(PLATFORM),
            vec![format!(
                "Key bindings not loaded, using the defaults: {error}"
            )],
        ),
    }
}

/// Writes the pending saves before the app exits: `terminate:` ends the process without
/// dropping the stores. The observer lives as long as the returned token.
fn flush_on_quit(pending: Vec<PendingWrites>) -> Retained<ProtocolObject<dyn NSObjectProtocol>> {
    let block = RcBlock::new(move |_note: NonNull<NSNotification>| {
        for writes in &pending {
            if let Err(error) = writes.flush() {
                unsaved(&error.to_string());
            }
        }
    });
    // SAFETY: an immutable string constant AppKit defines and never changes.
    let name = unsafe { NSApplicationWillTerminateNotification };
    // SAFETY: no object filter and no queue, so the block runs on the main thread that posts
    // the notification, and it captures only `Send` handles.
    unsafe {
        NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
            Some(name),
            None,
            None,
            &block,
        )
    }
}

#[expect(
    clippy::print_stderr,
    reason = "the app's last word as it quits: no window is left to show it"
)]
fn unsaved(reason: &str) {
    eprintln!("quickie: the last changes were not saved: {reason}");
}

/// `~/Library/Application Support/Quickie/shell.sqlite`, its directory created.
fn store_path() -> Result<PathBuf, String> {
    Ok(support_dir()?.join("shell.sqlite"))
}

/// `~/Library/Application Support/Quickie`, created.
fn support_dir() -> Result<PathBuf, String> {
    let directory = home_dir()?.join("Library/Application Support/Quickie");
    std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    Ok(directory)
}

/// `~`, from `HOME`: downloads go to `~/Downloads`.
fn home_dir() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME").ok_or("HOME is not set")?;
    Ok(PathBuf::from(home))
}

#[expect(
    clippy::print_stderr,
    reason = "the composition root's last word when the app cannot start: no window exists yet"
)]
fn fail(reason: &str) -> ExitCode {
    eprintln!("quickie: cannot start: {reason}");
    ExitCode::FAILURE
}
