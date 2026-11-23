//! Quickie, the composition root: wires the main engine, the `SQLite` store, the Keychain,
//! the LLM client, and the clock to the browsing, library, palette, shields, and agent use cases
//! and the keymap, puts them on screen, and runs the app.
//!
//! The main engine is a cargo feature: `main-webkit` (the default) or `main-chromium`, and this
//! file is the only one that names it. For now this is the development app on the
//! `engine-webkit` adapter (`docs/architecture.md` §3, P0). Run it from the repository root with:
//!
//! ```sh
//! mise exec -- cargo run -p quickie [-- <url or file>…]
//! mise run bundle && open target/debug/Quickie.app   # the app bundle, for LaunchServices
//! ```
//!
//! As a bundle it opens the links and files other apps hand it (`application:openURLs:`), and
//! the `App` commands ask macOS to make it the default browser or show the profile in Finder.
//! The bundled executable accepts `--automation-check <url>` for an isolated navigation check.
//!
//! The session, the history, and the bookmarks live in
//! `~/Library/Application Support/Quickie/shell.sqlite`. Every key is a command of the keymap
//! (`docs/design/inventory.md`, README "Default keys"): `⇧⌘P` / `⌘K` open the command palette,
//! `⌘P` goes to a tab, bookmark, or visited page, `⌘L` opens the address overlay, `⌘T` /
//! `⇧⌘N` open a tab / a throwaway tab, `⌘W` closes into the trash, `⌘N` / `⌘⇧W` open / close
//! a window, `⌘⇧T` reopens, `⌃⇥` /
//! `⌃⇧⇥` switch in most-recently-used order, `⌘1` … `⌘9` go to a pinned slot and `⌥⌘1` …
//! `⌥⌘9` pin there, `⌘D` bookmarks, `⇧⌘C` / `⌥⌘C` copy the URL / a Markdown link, `⌘J`
//! shows the downloads (saved to `~/Downloads`), `⌥⌘S` the site's shields, `⌘=` / `⌘-` / `⌘0`
//! zoom the site in, out, and back to 100 %, `⌥⌘P` prints, `⇧⌘R` reloads past the cache, `?`
//! in the palette asks the agent about the page and `⌘.` stops it. `⌘,` opens Settings, where
//! `keybindings.json` (in the same folder) opens and reloads. `⌘Q` quits.
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
mod updates;
mod window;

use std::path::PathBuf;
use std::process::ExitCode;
use std::ptr::NonNull;
use std::time::{SystemTime, UNIX_EPOCH};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{NSObjectProtocol, ProtocolObject};
use objc2::{AllocAnyThread, MainThreadMarker};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationWillTerminateNotification, NSImage,
    NSView,
};
use objc2_foundation::{NSBundle, NSNotification, NSNotificationCenter, ns_string};
use quickie_application::{
    Agent, AppearanceSettings, Browsing, Clock, Downloads, Engine, EnginePort, Extensions, Library,
    Shields, Zoom,
};
use quickie_domain_browsing::{Profile, Timestamp};
use quickie_domain_commands::{Catalog, Platform};
use quickie_store::{KeybindingsFile, PendingWrites, WriteBehindStore};
use quickie_system::{ExtensionFiles, KeychainSecrets, SystemEntropy};
use url::Url;

use browser::{Browser, Services, install_shortcuts, keymap_from, launch_urls};
use delegate::AppDelegate;

#[cfg(all(feature = "main-webkit", feature = "main-chromium"))]
compile_error!("one main engine: the `main-webkit` feature or `main-chromium`, not both");

#[cfg(not(any(feature = "main-webkit", feature = "main-chromium")))]
compile_error!("no main engine: turn on the `main-webkit` feature or `main-chromium`");

/// The engine of the main profile's pages, picked by a cargo feature, named here and nowhere
/// else; the rest of the shell reaches it through the application's engine ports.
#[cfg(feature = "main-webkit")]
pub type MainEngine = quickie_engine_webkit::WebKitEngine;

/// The engine of the main profile's pages, picked by a cargo feature, named here and nowhere
/// else; the rest of the shell reaches it through the application's engine ports.
#[cfg(all(feature = "main-chromium", not(feature = "main-webkit")))]
pub type MainEngine = quickie_engine_chromium::ChromiumEngine;

/// An engine the macOS shell can put on screen: its pages are `NSView`s and their pictures
/// `NSImage`s. The shell's code that works on any engine is generic over this one bound.
pub trait MacEngine: Engine<View = NSView, Picture = Retained<NSImage>> {}

impl<E: Engine<View = NSView, Picture = Retained<NSImage>>> MacEngine for E {}

/// A page the main engine opened by itself, for `Browsing::adopt`.
pub type OpenedPage = <MainEngine as EnginePort>::Page;

/// The main engine's network, for the browser's own requests on worker threads.
pub type Network = <MainEngine as Engine>::Http;

/// The browsing use cases as the app composes them.
pub type App = Browsing<MainEngine, WriteBehindStore, SystemClock>;

/// The library use cases as the app composes them.
pub type Lib = Library<WriteBehindStore, SystemClock>;

/// The palette query as the app composes it.
pub type Palette = quickie_application::Palette<SystemClock>;

/// The shields use cases as the app composes them.
pub type Guard = Shields<WriteBehindStore, SystemClock, <MainEngine as Engine>::Blocker>;

/// The desktop whose key conventions the keymap follows, chosen here once for the whole app.
pub const PLATFORM: Platform = Platform::MacOs;

/// The agent use cases as the app composes them: the endpoint in the store, its key in the
/// login Keychain, each request's fence marker from the system's generator.
pub type Agents = Agent<WriteBehindStore, KeychainSecrets, SystemEntropy>;

/// The page zoom use cases as the app composes them.
pub type Zooms = Zoom<WriteBehindStore>;

/// The appearance use cases as the app composes them.
pub type Looks = AppearanceSettings<WriteBehindStore>;

/// The web extension use cases as the app composes them: the main profile's extensions on the
/// main engine's host, saved in the store.
pub type Addons = Extensions<WriteBehindStore, <MainEngine as Engine>::Extensions, SystemClock>;

/// Where new tabs open: the start page, served by the engine from memory.
const HOME: &str = "quickie://start/";

/// The start page: no external resources, so launching opens no connection. Served from memory
/// by an engine that can.
#[cfg(feature = "main-webkit")]
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
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.as_slice() == ["--version"] {
        use std::io::{self, Write};
        return match writeln!(io::stdout().lock(), "{}", updates::INSTALLED_VERSION) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => fail(&error.to_string()),
        };
    }
    let Some(mtm) = MainThreadMarker::new() else {
        return ExitCode::FAILURE;
    };
    if args.iter().any(|arg| arg == "--automation-check") {
        return automation_check(&args, mtm)
            .map_or_else(|reason| fail(&reason), |()| ExitCode::SUCCESS);
    }
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
    let icon = NSBundle::mainBundle()
        .URLForResource_withExtension(Some(ns_string!("Quickie")), Some(ns_string!("icns")))
        .and_then(|url| NSImage::initWithContentsOfURL(NSImage::alloc(), &url));
    if let Some(icon) = icon {
        // SAFETY: a decoded, non-null icon is installed on the main thread.
        unsafe { app.setApplicationIconImage(Some(&icon)) };
    }

    let (browser, pending) = match start(mtm) {
        Ok(started) => started,
        Err(reason) => return fail(&reason),
    };
    // The application holds its delegate weakly: this binding keeps it for the app's lifetime.
    let delegate = AppDelegate::new(&browser, mtm);
    app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    menu::install(&app, &delegate, browser.catalog(), mtm);
    let _flush_on_quit = flush_on_quit(pending);
    let _shortcuts = install_shortcuts(&browser);
    browser.show();
    let cwd = std::env::current_dir().unwrap_or_default();
    browser.open_urls(launch_urls(&args, &cwd));
    app.activate();
    app.run();
    ExitCode::SUCCESS
}

#[cfg(feature = "main-webkit")]
fn automation_check(args: &[String], mtm: MainThreadMarker) -> Result<(), String> {
    use std::io::{self, Write};

    use quickie_engine_webkit::LocalAutomation;

    let address = match args {
        [flag, address] if flag == "--automation-check" => address,
        _ => {
            return Err(String::from(
                "usage: quickie --automation-check <http(s) URL>",
            ));
        }
    };
    let url = Url::parse(address).map_err(|error| error.to_string())?;
    objc2::rc::autoreleasepool(|_| {
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Prohibited);
        let mut automation = LocalAutomation::open(mtm).map_err(|error| error.to_string())?;
        let [context] = &automation.contexts().map_err(|error| error.to_string())?[..] else {
            return Err(String::from("expected one isolated browsing context"));
        };
        automation
            .navigate(context, &url)
            .map_err(|error| error.to_string())?;
        writeln!(
            io::stdout().lock(),
            "Navigation completed in the isolated WebKit session"
        )
        .map_err(|error| error.to_string())
    })
}

#[cfg(not(feature = "main-webkit"))]
fn automation_check(_: &[String], _: MainThreadMarker) -> Result<(), String> {
    Err(String::from(
        "local automation requires the Quickie WebKit fork",
    ))
}

/// Opens the store, restores the session and the library, loads the keymap, and puts it all
/// on screen. The stores write on their own threads; the shell keeps handles on their pending
/// writes.
fn start(mtm: MainThreadMarker) -> Result<(std::rc::Rc<Browser>, Vec<PendingWrites>), String> {
    let home = Url::parse(HOME).map_err(|error| error.to_string())?;
    let store = WriteBehindStore::open(&store_path()?).map_err(|error| error.to_string())?;
    let pending = store.pending();
    let mut engine = main_engine(&home, mtm);
    // Web extensions run in the main profile's pages, so their host exists before the session
    // restores its first page. None on an engine or a system without them.
    let extension_host = engine.web_extensions();
    let mut browsing = Browsing::start(Profile::Main, engine, store, SystemClock, &home)
        .map_err(|error| error.to_string())?;
    // The library has a writer, and a connection, of its own.
    let library_store =
        WriteBehindStore::open(&store_path()?).map_err(|error| error.to_string())?;
    let library_pending = library_store.pending();
    let catalog = Catalog::builtin();
    // The user's key bindings are a file they edit, and Settings → Keyboard edits too.
    let keybindings = KeybindingsFile::new(support_dir()?.join("keybindings.json"));
    let (keymap, keymap_problems) = keymap_from(&keybindings, &catalog);
    let notices = match keymap_problems.count() {
        0 => Vec::new(),
        n => vec![format!(
            "keybindings.json: {n} left out, see Settings → Keyboard"
        )],
    };
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
    // And the site zooms.
    let zoom_store = WriteBehindStore::open(&store_path()?).map_err(|error| error.to_string())?;
    let zoom_pending = zoom_store.pending();
    let zoom = Zoom::start(zoom_store).map_err(|error| error.to_string())?;
    // And the appearance.
    let appearance_store =
        WriteBehindStore::open(&store_path()?).map_err(|error| error.to_string())?;
    let appearance_pending = appearance_store.pending();
    let appearance =
        AppearanceSettings::start(appearance_store).map_err(|error| error.to_string())?;
    let downloads_folder = home_dir()?.join("Downloads");
    browsing
        .engine_mut()
        .set_directory(downloads_folder.clone());
    // The agent too. It contacts its endpoint only when asked (docs/privacy.md).
    let agent_store = WriteBehindStore::open(&store_path()?).map_err(|error| error.to_string())?;
    let agent_pending = agent_store.pending();
    let secrets = KeychainSecrets::login();
    let agent = Agent::start(agent_store, secrets.clone(), SystemEntropy)
        .map_err(|error| error.to_string())?;
    // And the extensions. They are read from disk now and run once read; nothing is fetched
    // unless the user asks or turned automatic updates on (docs/privacy.md).
    let extensions_store =
        WriteBehindStore::open(&store_path()?).map_err(|error| error.to_string())?;
    let extensions_pending = extensions_store.pending();
    let extensions = extension_host
        .map(|host| Extensions::start(extensions_store, host, SystemClock, PLATFORM))
        .transpose()
        .map_err(|error| error.to_string())?;
    let writes = vec![
        pending,
        library_pending,
        shields_pending,
        agent_pending,
        zoom_pending,
        appearance_pending,
        extensions_pending,
    ];
    let services = Services {
        browsing,
        library,
        shields,
        agent,
        secrets,
        zoom,
        appearance,
        palette: Palette::new(catalog.clone(), SystemClock),
        catalog,
        keymap,
        keybindings,
        keymap_problems,
        writes: writes.clone(),
        notices,
        downloads_folder,
        profile: support_dir()?,
        extensions,
        extension_files: ExtensionFiles::new(support_dir()?.join("Extensions")),
    };
    Ok((Browser::new(services, home, mtm), writes))
}

/// The `WebKit` engine, serving the start page from memory and drawing the overview's pictures at
/// the size of its widest card, so none costs more than a card shows at the screen's scale.
#[cfg(feature = "main-webkit")]
fn main_engine(home: &Url, mtm: MainThreadMarker) -> MainEngine {
    use objc2_foundation::NSSize;
    use quickie_ui_shared::tokens::overlay;

    let mut engine = MainEngine::new(&[(home.clone(), START_PAGE.to_owned())], mtm);
    engine.set_security_key_presenter(std::rc::Rc::new(move |parent, prompt, completed| {
        use quickie_engine_webkit::SecurityKeyStage;
        let pin = match prompt.stage {
            SecurityKeyStage::Touch => None,
            SecurityKeyStage::Pin {
                remaining,
                incorrect,
            } => Some((remaining, incorrect)),
        };
        quickie_ui_widgets::sheets::security_key::show(
            parent,
            &prompt.origin,
            &prompt.relying_party,
            pin,
            completed,
            mtm,
        )
    }));
    let card = overlay::THUMBNAIL_MAX_WIDTH;
    engine.set_thumbnail_size(NSSize::new(card, (card * overlay::THUMBNAIL_SHAPE).round()));
    engine
}

/// The Chromium engine. Its bridge serves no page from memory and draws no pictures yet. The
/// shell's key monitor sees every key before the page does, so a key the bridge offers is the
/// page's.
#[cfg(all(feature = "main-chromium", not(feature = "main-webkit")))]
fn main_engine(_home: &Url, _mtm: MainThreadMarker) -> MainEngine {
    MainEngine::new(|_key| false)
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
