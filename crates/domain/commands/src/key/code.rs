//! Keys as the ASCII-capable layout names them, independent of the active input language.

use super::Platform;

/// A key on the keyboard, named as on a US layout whatever layout is active.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    /// A letter key, `A` … `Z`.
    Letter(Letter),
    /// A digit key of the main row, `0` … `9`.
    Digit(Digit),
    /// A function key, `F1` … `F12`.
    Function(FunctionKey),
    /// `⇥`
    Tab,
    /// `↩`
    Enter,
    /// `⎋`
    Escape,
    /// The space bar.
    Space,
    /// `⌫`, delete to the left.
    Backspace,
    /// `⌦`, delete to the right.
    Delete,
    /// `↑`
    Up,
    /// `↓`
    Down,
    /// `←`
    Left,
    /// `→`
    Right,
    /// `↖`
    Home,
    /// `↘`
    End,
    /// `⇞`
    PageUp,
    /// `⇟`
    PageDown,
    /// `[`
    BracketLeft,
    /// `]`
    BracketRight,
    /// `-`
    Minus,
    /// `=`
    Equal,
    /// `,`
    Comma,
    /// `.`
    Period,
    /// `/`
    Slash,
    /// `\`
    Backslash,
    /// `;`
    Semicolon,
    /// `'`
    Quote,
    /// `` ` ``
    Backquote,
}

/// Every key without a payload, the ones parsed by their `keybindings.json` name.
const NAMED: [Key; 25] = [
    Key::Tab,
    Key::Enter,
    Key::Escape,
    Key::Space,
    Key::Backspace,
    Key::Delete,
    Key::Up,
    Key::Down,
    Key::Left,
    Key::Right,
    Key::Home,
    Key::End,
    Key::PageUp,
    Key::PageDown,
    Key::BracketLeft,
    Key::BracketRight,
    Key::Minus,
    Key::Equal,
    Key::Comma,
    Key::Period,
    Key::Slash,
    Key::Backslash,
    Key::Semicolon,
    Key::Quote,
    Key::Backquote,
];

impl Key {
    /// The key named as in `keybindings.json` (`p`, `5`, `f12`, `tab`, `[`), in any case.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        let name = name.to_ascii_lowercase();
        let mut chars = name.chars();
        if let (Some(only), None) = (chars.next(), chars.next()) {
            if let Some(letter) = Letter::new(only) {
                return Some(Self::Letter(letter));
            }
            if let Some(digit) = Digit::new(only) {
                return Some(Self::Digit(digit));
            }
        }
        if let Some(function) = name
            .strip_prefix('f')
            .and_then(|number| number.parse().ok())
            .and_then(FunctionKey::new)
        {
            return Some(Self::Function(function));
        }
        NAMED.into_iter().find(|key| key.name() == name)
    }

    /// The name `keybindings.json` uses for this key.
    #[must_use]
    pub fn name(self) -> String {
        match self {
            Self::Letter(letter) => letter.as_char().to_string(),
            Self::Digit(digit) => digit.as_char().to_string(),
            Self::Function(function) => format!("f{}", function.number()),
            Self::Tab => "tab".to_owned(),
            Self::Enter => "enter".to_owned(),
            Self::Escape => "escape".to_owned(),
            Self::Space => "space".to_owned(),
            Self::Backspace => "backspace".to_owned(),
            Self::Delete => "delete".to_owned(),
            Self::Up => "up".to_owned(),
            Self::Down => "down".to_owned(),
            Self::Left => "left".to_owned(),
            Self::Right => "right".to_owned(),
            Self::Home => "home".to_owned(),
            Self::End => "end".to_owned(),
            Self::PageUp => "pageup".to_owned(),
            Self::PageDown => "pagedown".to_owned(),
            other => other.symbol().to_string(),
        }
    }

    /// What a keycap shows on `platform`: `P`, `F5`, `[` on both; `⇥` on macOS where Linux writes
    /// `Tab`.
    #[must_use]
    pub fn glyph(self, platform: Platform) -> String {
        match self {
            Self::Letter(letter) => letter.as_char().to_ascii_uppercase().to_string(),
            Self::Digit(digit) => digit.as_char().to_string(),
            Self::Function(function) => format!("F{}", function.number()),
            other => other
                .caption(platform)
                .map_or_else(|| other.symbol().to_string(), str::to_owned),
        }
    }

    /// The keycap of a named key as `platform` prints it; `None` for punctuation, which prints
    /// its own character everywhere.
    fn caption(self, platform: Platform) -> Option<&'static str> {
        let (mac, linux) = match self {
            Self::Tab => ("⇥", "Tab"),
            Self::Enter => ("↩", "Enter"),
            Self::Escape => ("⎋", "Esc"),
            Self::Space => ("Space", "Space"),
            Self::Backspace => ("⌫", "Backspace"),
            Self::Delete => ("⌦", "Delete"),
            Self::Up => ("↑", "Up"),
            Self::Down => ("↓", "Down"),
            Self::Left => ("←", "Left"),
            Self::Right => ("→", "Right"),
            Self::Home => ("↖", "Home"),
            Self::End => ("↘", "End"),
            Self::PageUp => ("⇞", "Page Up"),
            Self::PageDown => ("⇟", "Page Down"),
            _ => return None,
        };
        Some(match platform {
            Platform::MacOs => mac,
            Platform::Linux => linux,
        })
    }

    /// The printed character of a punctuation key, which is both its name and its glyph.
    fn symbol(self) -> char {
        match self {
            Self::BracketLeft => '[',
            Self::BracketRight => ']',
            Self::Minus => '-',
            Self::Equal => '=',
            Self::Comma => ',',
            Self::Period => '.',
            Self::Slash => '/',
            Self::Backslash => '\\',
            Self::Semicolon => ';',
            Self::Quote => '\'',
            _ => '`',
        }
    }
}

/// A letter key, kept lowercase: the same key whether or not Shift is down.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Letter(char);

impl Letter {
    /// The key for an ASCII letter in either case; `None` for anything else.
    #[must_use]
    pub const fn new(letter: char) -> Option<Self> {
        if letter.is_ascii_alphabetic() {
            Some(Self(letter.to_ascii_lowercase()))
        } else {
            None
        }
    }

    /// The letter, lowercase.
    #[must_use]
    pub const fn as_char(self) -> char {
        self.0
    }
}

/// A digit key of the main row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Digit(char);

impl Digit {
    /// The key for an ASCII digit; `None` for anything else.
    #[must_use]
    pub const fn new(digit: char) -> Option<Self> {
        if digit.is_ascii_digit() {
            Some(Self(digit))
        } else {
            None
        }
    }

    /// The digit as printed on the key.
    #[must_use]
    pub const fn as_char(self) -> char {
        self.0
    }
}

/// A function key; Mac keyboards have twelve.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FunctionKey(u8);

impl FunctionKey {
    /// The highest function key number.
    pub const MAX: u8 = 12;

    /// The key `F<number>`, for 1 to [`Self::MAX`]; `None` otherwise.
    #[must_use]
    pub const fn new(number: u8) -> Option<Self> {
        if matches!(number, 1..=Self::MAX) {
            Some(Self(number))
        } else {
            None
        }
    }

    /// The number printed on the key.
    #[must_use]
    pub const fn number(self) -> u8 {
        self.0
    }
}
