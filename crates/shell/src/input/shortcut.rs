//! Browser shortcuts, matched on hardware key codes so they work on every layout (a Russian
//! layout types `д` on the `L` key; ⌘L must still open the address overlay).

/// Key codes of the ANSI keyboard (`kVK_*` in Carbon's `Events.h`).
pub mod key_code {
    pub const Q: u16 = 0x0C;
    pub const R: u16 = 0x0F;
    pub const RIGHT_BRACKET: u16 = 0x1E;
    pub const LEFT_BRACKET: u16 = 0x21;
    pub const RETURN: u16 = 0x24;
    pub const L: u16 = 0x25;
    pub const ESCAPE: u16 = 0x35;
    pub const KEYPAD_ENTER: u16 = 0x4C;
}

/// The modifier keys held with a key, as a set; Caps Lock and Fn do not count.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers(u8);

impl Modifiers {
    pub const NONE: Self = Self(0);
    pub const COMMAND: Self = Self(1);
    pub const SHIFT: Self = Self(1 << 1);
    pub const OPTION: Self = Self(1 << 2);
    pub const CONTROL: Self = Self(1 << 3);

    /// This set with `other` added when `held`.
    #[must_use]
    pub const fn with(self, other: Self, held: bool) -> Self {
        if held { Self(self.0 | other.0) } else { self }
    }
}

/// What a shortcut asks the browser to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    OpenAddress,
    Back,
    Forward,
    Reload,
    Quit,
    /// ⏎ in the address overlay.
    Submit,
    /// ⎋ in the address overlay.
    Dismiss,
}

/// The action for a key press, or `None` to let the page or the focused field have it.
#[must_use]
pub fn resolve(key: u16, modifiers: Modifiers, address_open: bool) -> Option<Action> {
    if modifiers == Modifiers::COMMAND {
        return match key {
            key_code::L => Some(Action::OpenAddress),
            key_code::LEFT_BRACKET => Some(Action::Back),
            key_code::RIGHT_BRACKET => Some(Action::Forward),
            key_code::R => Some(Action::Reload),
            key_code::Q => Some(Action::Quit),
            _ => None,
        };
    }
    if !address_open || modifiers != Modifiers::NONE {
        return None;
    }
    match key {
        key_code::RETURN | key_code::KEYPAD_ENTER => Some(Action::Submit),
        key_code::ESCAPE => Some(Action::Dismiss),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{Action, Modifiers, key_code, resolve};

    #[test]
    fn command_shortcuts_match_key_codes() {
        let cases = [
            (key_code::L, Action::OpenAddress),
            (key_code::LEFT_BRACKET, Action::Back),
            (key_code::RIGHT_BRACKET, Action::Forward),
            (key_code::R, Action::Reload),
            (key_code::Q, Action::Quit),
        ];
        for (key, action) in cases {
            assert_eq!(resolve(key, Modifiers::COMMAND, false), Some(action));
            assert_eq!(resolve(key, Modifiers::COMMAND, true), Some(action));
        }
    }

    #[test]
    fn other_command_keys_reach_the_page() {
        // ⌘C (0x08) stays with the page or the field.
        assert_eq!(resolve(0x08, Modifiers::COMMAND, false), None);
        assert_eq!(resolve(0x08, Modifiers::COMMAND, true), None);
    }

    #[test]
    fn extra_modifiers_make_a_different_shortcut() {
        for extra in [Modifiers::SHIFT, Modifiers::OPTION, Modifiers::CONTROL] {
            let modifiers = Modifiers::COMMAND.with(extra, true);
            assert_eq!(resolve(key_code::L, modifiers, false), None);
        }
        assert_eq!(
            resolve(
                key_code::L,
                Modifiers::COMMAND.with(Modifiers::SHIFT, false),
                false
            ),
            Some(Action::OpenAddress)
        );
        assert_eq!(resolve(key_code::L, Modifiers::NONE, false), None);
    }

    #[test]
    fn return_and_escape_belong_to_the_open_overlay() {
        assert_eq!(
            resolve(key_code::RETURN, Modifiers::NONE, true),
            Some(Action::Submit)
        );
        assert_eq!(
            resolve(key_code::KEYPAD_ENTER, Modifiers::NONE, true),
            Some(Action::Submit)
        );
        assert_eq!(
            resolve(key_code::ESCAPE, Modifiers::NONE, true),
            Some(Action::Dismiss)
        );
        assert_eq!(resolve(key_code::RETURN, Modifiers::NONE, false), None);
        assert_eq!(resolve(key_code::ESCAPE, Modifiers::NONE, false), None);
        assert_eq!(resolve(key_code::RETURN, Modifiers::SHIFT, true), None);
        assert_eq!(resolve(key_code::L, Modifiers::NONE, true), None);
    }
}
