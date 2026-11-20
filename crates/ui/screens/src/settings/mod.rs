//! The Settings window: a standard window with a sidebar of panes, as System Settings has
//! (`docs/design/mocks/settings`), opened (or brought forward, there is only one) by `⌘,` and
//! closed by `⌘W`. No glass: it is a window of its own, not something floating above a page,
//! so the sidebar is the system's sidebar material and the panes are plain.
//!
//! Every pane has its title in the titlebar's band, the window's buttons above the sidebar.
//! The keyboard goes everywhere: `↑` `↓` in the sidebar switch panes, `⇥` moves through each
//! pane top down (its buttons and switches too with Full Keyboard Access), and `⌘F` goes to the
//! keymap's search.
//!
//! - [`chrome`]: what the panes are made of, and the sidebar.
//! - [`panes`]: General, Privacy & Shields, Agent.
//! - [`keymap`]: Keyboard, the keymap editor.
//!
//! The Agent pane shows the agent settings form, handed in by the browser. Every button,
//! switch, and recorded key asks the browser through one [`SettingsAction`] handler; the panes
//! only show what the browser gives them.

mod chrome;
mod keymap;
mod panes;

use std::cell::Cell;
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSBackingStoreType, NSEventModifierFlags, NSView, NSWindow, NSWindowStyleMask,
    NSWindowTitleVisibility,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use quickie_domain_commands::{Chord, CommandId};
use quickie_domain_shields::FilterListId;
use quickie_ui_widgets::AgentSettings;

use chrome::sidebar::{self, Sidebar};
use chrome::{Flipped, HEIGHT, PANE_WIDTH, SettingsTarget, hairline};
pub use keymap::{KeysState, Recorded};
pub use panes::general::GeneralState;
pub use panes::shields::{ListState, ShieldsState};

/// A pane of the window, in sidebar order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pane {
    General,
    Shields,
    Keys,
    Agent,
}

impl Pane {
    const ALL: [Self; 4] = [Self::General, Self::Shields, Self::Keys, Self::Agent];

    fn title(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Shields => "Privacy & Shields",
            Self::Keys => "Keyboard",
            Self::Agent => "Agent",
        }
    }

    fn symbol(self) -> &'static str {
        match self {
            Self::General => "gearshape",
            Self::Shields => "hand.raised",
            Self::Keys => "keyboard",
            Self::Agent => "sparkles",
        }
    }

    fn index(self) -> usize {
        match self {
            Self::General => 0,
            Self::Shields => 1,
            Self::Keys => 2,
            Self::Agent => 3,
        }
    }
}

/// A key pressed while a command records its keys, as `AppKit` gave it: the browser reads it
/// the way its keymap reads every press.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyPress {
    /// The hardware key code.
    pub code: u16,
    /// What the key types, ignoring modifiers.
    pub typed: String,
    /// The modifiers held, `fn` included.
    pub flags: NSEventModifierFlags,
}

/// What a button, a switch, or a recorded key in the window asks of the browser.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettingsAction {
    /// Ask macOS to make Quickie the default browser.
    MakeDefaultBrowser,
    /// Show the downloads folder in Finder.
    ShowDownloads,
    /// Turn a filter list on or off.
    SetList(FilterListId, bool),
    /// Turn every filter list on, which fetches them.
    EnableLists,
    /// Fetch one list now.
    RefreshList(FilterListId),
    /// Fetch the enabled lists now.
    RefreshLists,
    /// Fetch stale lists on their own (`true`) or only on request.
    KeepListsFresh(bool),
    /// Open `keybindings.json` in the default editor, creating it when missing.
    OpenKeybindings,
    /// Read `keybindings.json` again and apply it.
    ReloadKeybindings,
    /// A key was pressed while the command recorded its keys: say what the keymap makes of
    /// it, with [`SettingsScreen::show_recorded`].
    Record(CommandId, KeyPress),
    /// Bind the command to the chord in place of what it has.
    Rebind(CommandId, Chord),
    /// Give the command its default bindings back.
    ResetBinding(CommandId),
}

/// The root view and the panes it switches between.
struct Frame {
    root: Retained<Flipped>,
    views: [Retained<NSView>; 4],
    current: Cell<Option<Pane>>,
    keys: Rc<keymap::Keys>,
}

impl Frame {
    fn show(&self, pane: Pane) {
        if self.current.get() == Some(pane) {
            return;
        }
        if let Some(old) = self.current.get() {
            self.views[old.index()].removeFromSuperview();
        }
        self.keys.stop_recording();
        let view = &self.views[pane.index()];
        view.setFrameOrigin(NSPoint::new(sidebar::WIDTH, 0.0));
        self.root.addSubview(view);
        self.current.set(Some(pane));
        // `⇥` runs from the sidebar through the pane top down, and back to the sidebar.
        if let Some(window) = self.root.window() {
            window.recalculateKeyViewLoop();
        }
    }
}

/// The Settings window and its panes.
pub struct SettingsScreen {
    window: Retained<NSWindow>,
    sidebar: Sidebar,
    frame: Rc<Frame>,
    general: panes::general::General,
    shields: panes::shields::Shields,
    _agent: panes::agent::Agent,
    keys: Rc<keymap::Keys>,
    target: Retained<SettingsTarget>,
}

impl SettingsScreen {
    /// The window, hidden, with `agent` as its Agent pane.
    #[must_use]
    pub fn new(agent: &AgentSettings, mtm: MainThreadMarker) -> Self {
        let target = SettingsTarget::new(mtm);
        let general = panes::general::General::new(&target, mtm);
        let shields = panes::shields::Shields::new(&target, mtm);
        let agent = panes::agent::Agent::new(agent, mtm);
        let keys = Rc::new(keymap::Keys::new(&target, mtm));
        let size = NSSize::new(sidebar::WIDTH + PANE_WIDTH, HEIGHT);
        let root = Flipped::new(NSRect::new(NSPoint::ZERO, size), mtm);
        let sidebar = Sidebar::new(mtm);
        root.addSubview(sidebar.view());
        root.addSubview(&hairline(
            NSRect::new(
                NSPoint::new(sidebar::WIDTH - 1.0, 0.0),
                NSSize::new(1.0, HEIGHT),
            ),
            mtm,
        ));
        let views = [general.view(), shields.view(), keys.view(), agent.view()].map(Retained::from);
        let frame = Rc::new(Frame {
            root,
            views,
            current: Cell::new(None),
            keys: Rc::clone(&keys),
        });
        let chooser = Rc::clone(&frame);
        sidebar.on_choose(move |pane| chooser.show(pane));

        let style = NSWindowStyleMask::Titled
            | NSWindowStyleMask::Closable
            | NSWindowStyleMask::Miniaturizable
            | NSWindowStyleMask::FullSizeContentView;
        // SAFETY: a buffered window of a valid frame and style, made on the main thread.
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                NSRect::new(NSPoint::ZERO, size),
                style,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        window.setTitle(&NSString::from_str("Settings"));
        window.setTitleVisibility(NSWindowTitleVisibility::Hidden);
        window.setTitlebarAppearsTransparent(true);
        window.setContentView(Some(&frame.root));
        window.setInitialFirstResponder(Some(sidebar.list()));
        // SAFETY: the screen owns the window, so AppKit must not release it on close as well.
        unsafe { window.setReleasedWhenClosed(false) };
        let screen = Self {
            window,
            sidebar,
            frame,
            general,
            shields,
            _agent: agent,
            keys,
            target,
        };
        screen.select(Pane::General);
        screen
    }

    /// Runs `handler` with what the user asks.
    pub fn on_action(&self, handler: impl Fn(SettingsAction) + 'static) {
        self.target.set_handler(handler);
    }

    /// The window, to tell its keys from a browser window's.
    #[must_use]
    pub fn window(&self) -> &NSWindow {
        &self.window
    }

    /// Whether the window is on screen.
    #[must_use]
    pub fn is_visible(&self) -> bool {
        self.window.isVisible()
    }

    /// Brings the window forward on `pane`, or on the pane it last showed; centred the first
    /// time.
    pub fn show(&self, pane: Option<Pane>) {
        if let Some(pane) = pane {
            self.select(pane);
        }
        if !self.window.isVisible() {
            self.window.center();
        }
        self.window.makeKeyAndOrderFront(None);
    }

    /// Closes the window, as its close button would.
    pub fn close(&self) {
        self.keys.stop_recording();
        self.window.performClose(None);
    }

    fn select(&self, pane: Pane) {
        self.sidebar.select(pane);
        self.frame.show(pane);
    }

    pub fn set_general(&self, state: &GeneralState) {
        self.general.update(state);
    }

    pub fn set_shields(&self, state: &ShieldsState) {
        self.shields.update(state);
    }

    pub fn set_keys(&self, state: &KeysState) {
        self.keys.update(state);
    }

    /// What the keymap makes of the key the user pressed while `command` recorded.
    pub fn show_recorded(&self, command: &CommandId, recorded: Recorded) {
        self.keys.show_recorded(command, recorded);
    }

    /// The keymap's search takes the keyboard, on the Keyboard pane.
    pub fn search_keymap(&self) {
        self.select(Pane::Keys);
        self.keys.focus_search();
    }
}
