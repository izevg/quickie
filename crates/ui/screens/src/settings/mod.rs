//! The Settings window: macOS's standard one, a toolbar of panes that resizes the window to the
//! pane, opened (or brought forward, there is only one) by `⌘,` and closed by `⌘W`. No glass:
//! it is a window of its own, not something floating above a page.
//!
//! - [`general`]: the default browser and the downloads folder.
//! - [`shields`]: where per-site shields live, the filter lists, and keeping them fresh.
//! - [`keys`]: `keybindings.json` and the entries that were left out.
//!
//! The Agent pane is the agent settings view, handed in by the browser. Every button and
//! switch asks the browser through one [`SettingsAction`] handler; the panes only show what
//! the browser gives them.

mod general;
mod keys;
mod shields;

use std::cell::RefCell;
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, Sel};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSBezelStyle, NSButton, NSColor, NSImage, NSLineBreakMode, NSTabViewController,
    NSTabViewControllerTabStyle, NSTabViewItem, NSTextField, NSView, NSViewController, NSWindow,
    NSWindowStyleMask,
};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use quickie_domain_shields::FilterListId;
use quickie_ui_shared::tokens::{TextRole, color, space};

pub use general::GeneralState;
pub use keys::KeysState;
pub use shields::{ListState, ShieldsState};

/// The width of every pane.
pub const PANE_WIDTH: CGFloat = 540.0;

/// A pane of the window, in toolbar order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pane {
    General,
    Shields,
    Agent,
    Keys,
}

impl Pane {
    const ALL: [Self; 4] = [Self::General, Self::Shields, Self::Agent, Self::Keys];

    fn title(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Shields => "Shields",
            Self::Agent => "Agent",
            Self::Keys => "Keys",
        }
    }

    fn symbol(self) -> &'static str {
        match self {
            Self::General => "gearshape",
            Self::Shields => "shield.lefthalf.filled",
            Self::Agent => "sparkles",
            Self::Keys => "keyboard",
        }
    }

    fn index(self) -> isize {
        match self {
            Self::General => 0,
            Self::Shields => 1,
            Self::Agent => 2,
            Self::Keys => 3,
        }
    }
}

/// What a button or a switch in the window asks of the browser.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsAction {
    /// Ask macOS to make Quickie the default browser.
    MakeDefaultBrowser,
    /// Show the downloads folder in Finder.
    ShowDownloads,
    /// Turn a filter list on or off.
    SetList(FilterListId, bool),
    /// Fetch the enabled lists now.
    RefreshLists,
    /// Fetch stale lists on their own (`true`) or only on request.
    KeepListsFresh(bool),
    /// Open `keybindings.json` in the default editor, creating it when missing.
    OpenKeybindings,
    /// Read `keybindings.json` again and apply it.
    ReloadKeybindings,
}

type Handler = Rc<dyn Fn(SettingsAction)>;

pub struct Ivars {
    handler: RefCell<Option<Handler>>,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements, and `SettingsTarget` implements no
    // `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "QuickieSettingsTarget"]
    #[ivars = Ivars]
    pub struct SettingsTarget;

    unsafe impl NSObjectProtocol for SettingsTarget {}

    impl SettingsTarget {
        #[unsafe(method(makeDefault:))]
        fn make_default(&self, _sender: Option<&NSButton>) {
            self.ask(SettingsAction::MakeDefaultBrowser);
        }

        #[unsafe(method(showDownloads:))]
        fn show_downloads(&self, _sender: Option<&NSButton>) {
            self.ask(SettingsAction::ShowDownloads);
        }

        #[unsafe(method(refreshLists:))]
        fn refresh_lists(&self, _sender: Option<&NSButton>) {
            self.ask(SettingsAction::RefreshLists);
        }

        #[unsafe(method(openKeybindings:))]
        fn open_keybindings(&self, _sender: Option<&NSButton>) {
            self.ask(SettingsAction::OpenKeybindings);
        }

        #[unsafe(method(reloadKeybindings:))]
        fn reload_keybindings(&self, _sender: Option<&NSButton>) {
            self.ask(SettingsAction::ReloadKeybindings);
        }
    }
);

impl SettingsTarget {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars {
            handler: RefCell::new(None),
        });
        // SAFETY: `init` is `NSObject`'s designated initializer and the ivars are set.
        unsafe { msg_send![super(this), init] }
    }

    fn ask(&self, action: SettingsAction) {
        let handler = self.ivars().handler.borrow().clone();
        if let Some(handler) = handler {
            handler(action);
        }
    }
}

define_class!(
    // SAFETY: `NSView` has no subclassing requirements, and `PaneView` implements no `Drop`;
    // `isFlipped` is public API meant to be overridden.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "QuickieSettingsPane"]
    pub struct PaneView;

    impl PaneView {
        /// Panes lay out top down.
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }
    }
);

impl PaneView {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        // SAFETY: `init` is `NSView`'s initializer for a zero frame.
        unsafe { msg_send![super(this), init] }
    }
}

/// The Settings window and its panes.
pub struct SettingsScreen {
    window: Retained<NSWindow>,
    tabs: Retained<NSTabViewController>,
    panes: [Retained<NSViewController>; 4],
    general: general::General,
    shields: shields::Shields,
    keys: keys::Keys,
    agent: Retained<NSView>,
    target: Retained<SettingsTarget>,
}

impl SettingsScreen {
    /// The window, hidden, with `agent` as its Agent pane; `agent` is laid out for
    /// [`PANE_WIDTH`] by its owner and keeps its height.
    #[must_use]
    pub fn new(agent: &NSView, mtm: MainThreadMarker) -> Self {
        let target = SettingsTarget::new(mtm);
        let general = general::General::new(&target, mtm);
        let shields = shields::Shields::new(&target, mtm);
        let keys = keys::Keys::new(&target, mtm);
        let tabs = NSTabViewController::new(mtm);
        tabs.setTabStyle(NSTabViewControllerTabStyle::Toolbar);
        let agent_pane = PaneView::new(mtm);
        agent_pane.addSubview(agent);
        let views: [&NSView; 4] = [general.view(), shields.view(), &agent_pane, keys.view()];
        let panes = Pane::ALL.map(|pane| {
            let controller = NSViewController::new(mtm);
            controller.setTitle(Some(&NSString::from_str(pane.title())));
            controller
        });
        for ((controller, view), pane) in panes.iter().zip(views).zip(Pane::ALL) {
            controller.setView(view);
            let item = NSTabViewItem::tabViewItemWithViewController(controller);
            item.setLabel(&NSString::from_str(pane.title()));
            let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(
                &NSString::from_str(pane.symbol()),
                Some(&NSString::from_str(pane.title())),
            );
            item.setImage(image.as_deref());
            tabs.addTabViewItem(&item);
        }
        let window = NSWindow::windowWithContentViewController(&tabs);
        window.setStyleMask(NSWindowStyleMask::Titled | NSWindowStyleMask::Closable);
        // SAFETY: the screen owns the window, so AppKit must not release it on close as well.
        unsafe { window.setReleasedWhenClosed(false) };
        let screen = Self {
            window,
            tabs,
            panes,
            general,
            shields,
            keys,
            agent: agent_pane.into_super(),
            target,
        };
        screen.fit_agent();
        screen
    }

    /// Runs `handler` with what the user asks.
    pub fn on_action(&self, handler: impl Fn(SettingsAction) + 'static) {
        *self.target.ivars().handler.borrow_mut() = Some(Rc::new(handler));
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
            self.tabs.setSelectedTabViewItemIndex(pane.index());
        }
        self.fit_agent();
        if !self.window.isVisible() {
            self.window.center();
        }
        self.window.makeKeyAndOrderFront(None);
    }

    /// Closes the window, as its close button would.
    pub fn close(&self) {
        self.window.performClose(None);
    }

    /// The Agent pane changed height (a status line wrapped): the window follows.
    pub fn fit_agent(&self) {
        let height = self
            .agent
            .subviews()
            .iter()
            .map(|view| view.frame().size.height)
            .fold(0.0, CGFloat::max);
        for view in &self.agent.subviews() {
            view.setFrameOrigin(NSPoint::new(space::XL, space::XL));
        }
        self.panes[Pane::Agent.index().unsigned_abs()]
            .setPreferredContentSize(NSSize::new(PANE_WIDTH, height + space::XL * 2.0));
        self.agent
            .setFrameSize(NSSize::new(PANE_WIDTH, height + space::XL * 2.0));
    }

    pub fn set_general(&self, state: &GeneralState) {
        let height = self.general.update(state);
        self.resize(Pane::General, height);
    }

    pub fn set_shields(&self, state: &ShieldsState) {
        let height = self.shields.update(state);
        self.resize(Pane::Shields, height);
    }

    pub fn set_keys(&self, state: &KeysState) {
        let height = self.keys.update(state);
        self.resize(Pane::Keys, height);
    }

    fn resize(&self, pane: Pane, height: CGFloat) {
        self.panes[pane.index().unsigned_abs()]
            .setPreferredContentSize(NSSize::new(PANE_WIDTH, height));
    }
}

/// Stacks views top down in a flipped pane, `space-xl` in from its edges.
struct Column {
    top: CGFloat,
}

impl Column {
    const INNER: CGFloat = PANE_WIDTH - space::XL * 2.0;

    fn new() -> Self {
        Self { top: space::XL }
    }

    /// Places `view` at the full inner width, `height` tall.
    fn full(&mut self, view: &NSView, height: CGFloat) {
        view.setFrame(NSRect::new(
            NSPoint::new(space::XL, self.top),
            NSSize::new(Self::INNER, height),
        ));
        self.top += height;
    }

    /// Places buttons side by side at their own sizes.
    fn buttons(&mut self, buttons: &[&NSButton]) {
        let mut x = space::XL;
        let mut tallest: CGFloat = 0.0;
        for button in buttons {
            let size = button.frame().size;
            button.setFrameOrigin(NSPoint::new(x, self.top));
            x += size.width + space::SM;
            tallest = tallest.max(size.height);
        }
        self.top += tallest;
    }

    fn gap(&mut self, height: CGFloat) {
        self.top += height;
    }

    /// The pane's height so far, with the bottom margin.
    fn height(&self) -> CGFloat {
        self.top + space::XL
    }
}

/// A one-line label in `role` and `tint`.
fn label(
    text: &str,
    role: TextRole,
    tint: &NSColor,
    mtm: MainThreadMarker,
) -> Retained<NSTextField> {
    let field = NSTextField::labelWithString(&NSString::from_str(text), mtm);
    field.setFont(Some(&role.font()));
    field.setTextColor(Some(tint));
    field.setLineBreakMode(NSLineBreakMode::ByTruncatingMiddle);
    field.sizeToFit();
    field
}

/// A section heading: `caption`, secondary.
fn heading(text: &str, mtm: MainThreadMarker) -> Retained<NSTextField> {
    label(text, TextRole::Caption, &color::secondary_label(), mtm)
}

/// A label that wraps at the column's width.
fn paragraph(text: &str, mtm: MainThreadMarker) -> Retained<NSTextField> {
    let field = NSTextField::wrappingLabelWithString(&NSString::from_str(text), mtm);
    field.setFont(Some(&TextRole::RowDetail.font()));
    field.setTextColor(Some(&color::secondary_label()));
    field.setSelectable(false);
    field
}

/// The height `field` needs to wrap within the column.
fn wrapped_height(field: &NSTextField) -> CGFloat {
    field
        .cell()
        .map_or(field.frame().size.height, |cell| {
            cell.cellSizeForBounds(NSRect::new(
                NSPoint::new(0.0, 0.0),
                NSSize::new(Column::INNER, 10_000.0),
            ))
            .height
        })
        .ceil()
}

fn set(field: &NSTextField, text: &str) {
    field.setStringValue(&NSString::from_str(text));
}

/// A push button that sends `action` to the screen's target.
fn button(
    title: &str,
    action: Sel,
    target: &SettingsTarget,
    mtm: MainThreadMarker,
) -> Retained<NSButton> {
    let button = NSButton::new(mtm);
    button.setTitle(&NSString::from_str(title));
    button.setBezelStyle(NSBezelStyle::Push);
    let target: &AnyObject = target.as_ref();
    // SAFETY: the target is weak; the screen keeps it alive for as long as its buttons.
    unsafe { button.setTarget(Some(target)) };
    // SAFETY: the target implements `action`, which takes the sender.
    unsafe { button.setAction(Some(action)) };
    button.sizeToFit();
    button
}
