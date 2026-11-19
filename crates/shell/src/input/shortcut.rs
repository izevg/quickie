//! Browser shortcuts, matched on hardware key codes so they work on every layout (a Russian
//! layout types `д` on the `L` key; ⌘L must still open the address overlay).

use quickie_domain_browsing::{Direction, PinnedSlot};

/// Key codes of the ANSI keyboard (`kVK_*` in Carbon's `Events.h`).
pub mod key_code {
    pub const R: u16 = 0x0F;
    pub const T: u16 = 0x11;
    pub const W: u16 = 0x0D;
    pub const Q: u16 = 0x0C;
    pub const RIGHT_BRACKET: u16 = 0x1E;
    pub const LEFT_BRACKET: u16 = 0x21;
    pub const RETURN: u16 = 0x24;
    pub const L: u16 = 0x25;
    pub const TAB: u16 = 0x30;
    pub const ESCAPE: u16 = 0x35;
    pub const KEYPAD_ENTER: u16 = 0x4C;
    /// The digit keys `1` … `9` of the top row, in order. They are not contiguous.
    pub const DIGITS: [u16; 9] = [0x12, 0x13, 0x14, 0x15, 0x17, 0x16, 0x1A, 0x1C, 0x19];
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

    /// True when every key of `other` is held.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

/// What the browser is in the middle of, which changes what some keys mean.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Context {
    /// The address overlay is open: ⏎ and ⎋ are its.
    pub address_open: bool,
    /// A `⌃⇥` cycle is in progress: ⎋ cancels it.
    pub cycling: bool,
}

/// What a shortcut asks the browser to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    OpenAddress,
    Back,
    Forward,
    Reload,
    Quit,
    /// ⌘T: a new tab, with the address overlay open on it.
    NewTab,
    /// ⌘W: close the active tab into the trash.
    CloseTab,
    /// ⌘⇧T: reopen the most recently closed tab.
    ReopenClosed,
    /// ⌘1 … ⌘9.
    GoToSlot(PinnedSlot),
    /// ⌥⌘1 … ⌥⌘9: pin the active tab, until the palette offers it.
    PinToSlot(PinnedSlot),
    /// ⌃⇥ / ⌃⇧⇥: begin a cycle, or step the one in progress.
    Cycle(Direction),
    /// ⎋ during a cycle.
    CancelCycle,
    /// ⏎ in the address overlay.
    Submit,
    /// ⎋ in the address overlay.
    Dismiss,
}

/// The action for a key press, or `None` to let the page or the focused field have it.
#[must_use]
pub fn resolve(key: u16, modifiers: Modifiers, context: Context) -> Option<Action> {
    if context.cycling && key == key_code::ESCAPE {
        return Some(Action::CancelCycle);
    }
    if key == key_code::TAB && modifiers.contains(Modifiers::CONTROL) {
        return cycle(modifiers);
    }
    let slot = slot(key);
    let option_command = Modifiers::COMMAND.with(Modifiers::OPTION, true);
    let shift_command = Modifiers::COMMAND.with(Modifiers::SHIFT, true);
    if modifiers == option_command {
        return slot.map(Action::PinToSlot);
    }
    if modifiers == shift_command {
        return (key == key_code::T).then_some(Action::ReopenClosed);
    }
    if modifiers == Modifiers::COMMAND {
        return slot.map(Action::GoToSlot).or_else(|| command(key));
    }
    if !context.address_open || modifiers != Modifiers::NONE {
        return None;
    }
    match key {
        key_code::RETURN | key_code::KEYPAD_ENTER => Some(Action::Submit),
        key_code::ESCAPE => Some(Action::Dismiss),
        _ => None,
    }
}

/// `⌃⇥` forward, `⌃⇧⇥` backward; ⌘ or ⌥ with them is someone else's chord.
fn cycle(modifiers: Modifiers) -> Option<Action> {
    let backward = Modifiers::CONTROL.with(Modifiers::SHIFT, true);
    if modifiers == Modifiers::CONTROL {
        Some(Action::Cycle(Direction::Forward))
    } else if modifiers == backward {
        Some(Action::Cycle(Direction::Backward))
    } else {
        None
    }
}

fn command(key: u16) -> Option<Action> {
    match key {
        key_code::L => Some(Action::OpenAddress),
        key_code::LEFT_BRACKET => Some(Action::Back),
        key_code::RIGHT_BRACKET => Some(Action::Forward),
        key_code::R => Some(Action::Reload),
        key_code::Q => Some(Action::Quit),
        key_code::T => Some(Action::NewTab),
        key_code::W => Some(Action::CloseTab),
        _ => None,
    }
}

/// The pinned slot of a digit key.
fn slot(key: u16) -> Option<PinnedSlot> {
    let index = key_code::DIGITS.iter().position(|&digit| digit == key)?;
    PinnedSlot::from_index(index)
}

#[cfg(test)]
mod tests {
    use quickie_domain_browsing::{Direction, PinnedSlot};

    use super::{Action, Context, Modifiers, key_code, resolve};

    const IDLE: Context = Context {
        address_open: false,
        cycling: false,
    };
    const ADDRESS: Context = Context {
        address_open: true,
        cycling: false,
    };
    const CYCLING: Context = Context {
        address_open: false,
        cycling: true,
    };

    fn slot(n: u8) -> PinnedSlot {
        PinnedSlot::try_from(n).unwrap()
    }

    fn mods(list: &[Modifiers]) -> Modifiers {
        list.iter()
            .fold(Modifiers::NONE, |all, &one| all.with(one, true))
    }

    #[test]
    fn command_shortcuts_match_key_codes() {
        let cases = [
            (key_code::L, Action::OpenAddress),
            (key_code::LEFT_BRACKET, Action::Back),
            (key_code::RIGHT_BRACKET, Action::Forward),
            (key_code::R, Action::Reload),
            (key_code::Q, Action::Quit),
            (key_code::T, Action::NewTab),
            (key_code::W, Action::CloseTab),
        ];
        for (key, action) in cases {
            assert_eq!(resolve(key, Modifiers::COMMAND, IDLE), Some(action));
            assert_eq!(resolve(key, Modifiers::COMMAND, ADDRESS), Some(action));
        }
    }

    #[test]
    fn other_command_keys_reach_the_page() {
        // ⌘C (0x08) stays with the page or the field.
        assert_eq!(resolve(0x08, Modifiers::COMMAND, IDLE), None);
        assert_eq!(resolve(0x08, Modifiers::COMMAND, ADDRESS), None);
    }

    #[test]
    fn extra_modifiers_make_a_different_shortcut() {
        for extra in [Modifiers::SHIFT, Modifiers::OPTION, Modifiers::CONTROL] {
            let modifiers = Modifiers::COMMAND.with(extra, true);
            assert_eq!(resolve(key_code::L, modifiers, IDLE), None);
        }
        assert_eq!(
            resolve(
                key_code::L,
                Modifiers::COMMAND.with(Modifiers::SHIFT, false),
                IDLE
            ),
            Some(Action::OpenAddress)
        );
        assert_eq!(resolve(key_code::L, Modifiers::NONE, IDLE), None);
    }

    #[test]
    fn shift_command_t_reopens_and_nothing_else_does() {
        let shift_command = mods(&[Modifiers::COMMAND, Modifiers::SHIFT]);
        assert_eq!(
            resolve(key_code::T, shift_command, IDLE),
            Some(Action::ReopenClosed)
        );
        assert_eq!(resolve(key_code::W, shift_command, IDLE), None);
        assert_eq!(resolve(key_code::DIGITS[0], shift_command, IDLE), None);
    }

    #[test]
    fn digits_go_to_and_pin_slots_in_order() {
        let option_command = mods(&[Modifiers::COMMAND, Modifiers::OPTION]);
        for (index, &key) in key_code::DIGITS.iter().enumerate() {
            let n = u8::try_from(index + 1).unwrap();
            assert_eq!(
                resolve(key, Modifiers::COMMAND, IDLE),
                Some(Action::GoToSlot(slot(n)))
            );
            assert_eq!(
                resolve(key, option_command, IDLE),
                Some(Action::PinToSlot(slot(n)))
            );
        }
        assert_eq!(resolve(key_code::T, option_command, IDLE), None);
        assert_eq!(resolve(key_code::DIGITS[0], Modifiers::NONE, IDLE), None);
    }

    #[test]
    fn control_tab_cycles_both_ways() {
        let backward = mods(&[Modifiers::CONTROL, Modifiers::SHIFT]);
        for context in [IDLE, CYCLING, ADDRESS] {
            assert_eq!(
                resolve(key_code::TAB, Modifiers::CONTROL, context),
                Some(Action::Cycle(Direction::Forward))
            );
            assert_eq!(
                resolve(key_code::TAB, backward, context),
                Some(Action::Cycle(Direction::Backward))
            );
        }
        let with_command = mods(&[Modifiers::CONTROL, Modifiers::COMMAND]);
        assert_eq!(resolve(key_code::TAB, with_command, IDLE), None);
        assert_eq!(resolve(key_code::TAB, Modifiers::NONE, IDLE), None);
        assert_eq!(resolve(key_code::TAB, Modifiers::SHIFT, IDLE), None);
    }

    #[test]
    fn escape_cancels_a_cycle_whatever_is_held() {
        assert_eq!(
            resolve(key_code::ESCAPE, Modifiers::CONTROL, CYCLING),
            Some(Action::CancelCycle)
        );
        assert_eq!(
            resolve(key_code::ESCAPE, Modifiers::NONE, CYCLING),
            Some(Action::CancelCycle)
        );
        assert_eq!(resolve(key_code::ESCAPE, Modifiers::CONTROL, IDLE), None);
    }

    #[test]
    fn modifier_sets_report_what_they_hold() {
        let held = mods(&[Modifiers::CONTROL, Modifiers::SHIFT]);
        assert!(held.contains(Modifiers::CONTROL));
        assert!(held.contains(Modifiers::SHIFT));
        assert!(!held.contains(Modifiers::COMMAND));
        assert!(held.contains(Modifiers::NONE));
    }

    #[test]
    fn return_and_escape_belong_to_the_open_overlay() {
        assert_eq!(
            resolve(key_code::RETURN, Modifiers::NONE, ADDRESS),
            Some(Action::Submit)
        );
        assert_eq!(
            resolve(key_code::KEYPAD_ENTER, Modifiers::NONE, ADDRESS),
            Some(Action::Submit)
        );
        assert_eq!(
            resolve(key_code::ESCAPE, Modifiers::NONE, ADDRESS),
            Some(Action::Dismiss)
        );
        assert_eq!(resolve(key_code::RETURN, Modifiers::NONE, IDLE), None);
        assert_eq!(resolve(key_code::ESCAPE, Modifiers::NONE, IDLE), None);
        assert_eq!(resolve(key_code::RETURN, Modifiers::SHIFT, ADDRESS), None);
        assert_eq!(resolve(key_code::L, Modifiers::NONE, ADDRESS), None);
    }
}
