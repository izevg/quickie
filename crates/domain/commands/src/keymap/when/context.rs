//! What is true when a key is pressed, and the context keys a `when` clause names.
//!
//! The shell sets Quickie's own keys, which a clause interns when it is parsed: a flag is a bit,
//! `focusedView` an enum. Resolving a key press then neither allocates nor compares strings. Any
//! other key, such as one a user copied from VS Code, still works by name.

/// A context flag Quickie sets, under the name a `when` clause gives it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ContextFlag {
    /// `pageFocused`: the page has the keyboard, no overlay is open.
    PageFocused,
    /// `inputFocused`: a text field has the keyboard, the page's or an overlay's own.
    InputFocused,
    /// `paletteOpen`: the command palette is open.
    PaletteOpen,
    /// `addressOpen`: the address field is open.
    AddressOpen,
    /// `overviewOpen`: the tab overview is open.
    OverviewOpen,
    /// `hintsOpen`: link hints are showing.
    HintsOpen,
    /// `switcherOpen`: a `⌃⇥` cycle is running.
    SwitcherOpen,
    /// `popoverOpen`: a popover is open.
    PopoverOpen,
    /// `agentOpen`: the agent panel is open.
    AgentOpen,
}

impl ContextFlag {
    /// Every flag.
    pub const ALL: [Self; 9] = [
        Self::PageFocused,
        Self::InputFocused,
        Self::PaletteOpen,
        Self::AddressOpen,
        Self::OverviewOpen,
        Self::HintsOpen,
        Self::SwitcherOpen,
        Self::PopoverOpen,
        Self::AgentOpen,
    ];

    /// The flag's name in a `when` clause.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::PageFocused => "pageFocused",
            Self::InputFocused => "inputFocused",
            Self::PaletteOpen => "paletteOpen",
            Self::AddressOpen => "addressOpen",
            Self::OverviewOpen => "overviewOpen",
            Self::HintsOpen => "hintsOpen",
            Self::SwitcherOpen => "switcherOpen",
            Self::PopoverOpen => "popoverOpen",
            Self::AgentOpen => "agentOpen",
        }
    }

    /// The flag a clause names, if it is one of Quickie's.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|flag| flag.name() == name)
    }

    /// The flag's bit in a [`WhenContext`].
    const fn bit(self) -> u16 {
        1 << self as u16
    }
}

/// The value of `focusedView`: what has the keyboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FocusedView {
    /// `'page'`: the page, or its find bar.
    Page,
    /// `'palette'`: the command palette.
    Palette,
    /// `'address'`: the address field.
    Address,
    /// `'overview'`: the tab overview.
    Overview,
    /// `'settings'`: the Settings window.
    Settings,
}

impl FocusedView {
    /// The context key that holds the view.
    pub const KEY: &'static str = "focusedView";

    /// Every view.
    pub const ALL: [Self; 5] = [
        Self::Page,
        Self::Palette,
        Self::Address,
        Self::Overview,
        Self::Settings,
    ];

    /// The view's value in a `when` clause, unquoted.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Page => "page",
            Self::Palette => "palette",
            Self::Address => "address",
            Self::Overview => "overview",
            Self::Settings => "settings",
        }
    }

    /// The view a value names, if it is one of Quickie's.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|view| view.name() == name)
    }
}

/// A context key as a clause names it, interned when the clause is parsed.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) enum ContextKey {
    /// One of Quickie's flags.
    Flag(ContextFlag),
    /// `focusedView`.
    View,
    /// Any other key, by name.
    Other(String),
}

impl ContextKey {
    pub(super) fn of(name: &str) -> Self {
        if name == FocusedView::KEY {
            return Self::View;
        }
        ContextFlag::from_name(name).map_or_else(|| Self::Other(name.to_owned()), Self::Flag)
    }

    pub(super) fn name(&self) -> &str {
        match self {
            Self::Flag(flag) => flag.name(),
            Self::View => FocusedView::KEY,
            Self::Other(name) => name,
        }
    }
}

/// The value a clause compares a key with; a view when the key is `focusedView` and the value
/// names one of Quickie's views.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) enum ContextValue {
    /// One of Quickie's views.
    View(FocusedView),
    /// Any other value.
    Text(String),
}

impl ContextValue {
    pub(super) fn of(key: &ContextKey, text: String) -> Self {
        match (key, FocusedView::from_name(&text)) {
            (ContextKey::View, Some(view)) => Self::View(view),
            _ => Self::Text(text),
        }
    }

    pub(super) fn as_str(&self) -> &str {
        match self {
            Self::View(view) => view.name(),
            Self::Text(text) => text,
        }
    }
}

/// What is true right now: the flags that are set and the values of context keys, such as
/// `inputFocused` and `focusedView == 'palette'`. The shell fills it on each key press.
///
/// Quickie's own keys are a bit set and an enum, so the context costs nothing to build; any other
/// key is kept by name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WhenContext {
    flags: u16,
    view: Option<FocusedView>,
    other_flags: Vec<String>,
    other_values: Vec<(String, String)>,
}

impl WhenContext {
    /// A context where nothing is set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The same context with one of Quickie's flags set.
    #[must_use]
    pub fn with(mut self, flag: ContextFlag) -> Self {
        self.flags |= flag.bit();
        self
    }

    /// The same context with `focusedView` holding `view`.
    #[must_use]
    pub fn in_view(mut self, view: FocusedView) -> Self {
        self.view = Some(view);
        self.other_values.retain(|(key, _)| key != FocusedView::KEY);
        self
    }

    /// The same context with the flag `key` set.
    #[must_use]
    pub fn with_flag(mut self, key: &str) -> Self {
        if let Some(flag) = ContextFlag::from_name(key) {
            return self.with(flag);
        }
        if !self.other_flags.iter().any(|set| set == key) {
            self.other_flags.push(key.to_owned());
        }
        self
    }

    /// The same context with `key` holding `value`, in place of what it held.
    #[must_use]
    pub fn with_value(mut self, key: &str, value: &str) -> Self {
        if key == FocusedView::KEY {
            if let Some(view) = FocusedView::from_name(value) {
                return self.in_view(view);
            }
            self.view = None;
        }
        self.other_values.retain(|(held, _)| held != key);
        self.other_values.push((key.to_owned(), value.to_owned()));
        self
    }

    /// A key is true when it is a set flag or holds a non-empty value, as in VS Code. Every
    /// view of Quickie's has a name, so a set view is true.
    pub(super) fn is_truthy(&self, key: &ContextKey) -> bool {
        let set = match key {
            ContextKey::Flag(flag) => self.flags & flag.bit() != 0,
            ContextKey::View => self.view.is_some() || self.has_other_flag(key),
            ContextKey::Other(_) => self.has_other_flag(key),
        };
        set || self.text(key).is_some_and(|value| !value.is_empty())
    }

    /// Whether `key` holds `value`. A view of Quickie's is only ever held as the enum, any
    /// other value only by name.
    pub(super) fn holds(&self, key: &ContextKey, value: &ContextValue) -> bool {
        match (key, value) {
            (ContextKey::View, ContextValue::View(view)) => self.view == Some(*view),
            _ => self.text(key) == Some(value.as_str()),
        }
    }

    fn has_other_flag(&self, key: &ContextKey) -> bool {
        self.other_flags.iter().any(|set| set == key.name())
    }

    /// The value `key` holds by name.
    fn text(&self, key: &ContextKey) -> Option<&str> {
        self.other_values
            .iter()
            .find(|(held, _)| held == key.name())
            .map(|(_, value)| value.as_str())
    }
}
