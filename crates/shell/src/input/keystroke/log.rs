//! The last key presses and what became of them, for "Show Key Debug Overlay": the key code,
//! the characters, the modifiers (`fn` included), and the outcome, so a flaky shortcut can be
//! reported precisely. Kept in memory only while the overlay shows, and never written anywhere.

use std::collections::VecDeque;

use quickie_domain_commands::{CommandId, KeyStroke, Modifiers, Platform};

/// How many presses the overlay lists.
pub const LENGTH: usize = 10;

/// What became of a key press.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The keymap ran this command; `reserved` when the page could never have had the key.
    Command { id: CommandId, reserved: bool },
    /// The first stroke of a chord: the next one completes it.
    Pending,
    /// The second stroke of a chord that nothing binds; it went nowhere.
    ChordUnbound,
    /// An open overlay's or sheet's own key (`⎋`, `⏎`, arrows, a hint label).
    Overlay,
    /// The key of an extension's command, which ran; the keymap binds nothing to it.
    Extension,
    /// `⌘Q`.
    Quit,
    /// Nothing of ours: the page, the focused field, or the menu bar has it.
    Passed,
    /// The same press again: the page did not take it, and `WebKit` handed it back to `AppKit`,
    /// which runs the monitor a second time. It goes on untouched.
    Returned,
    /// Held with `fn`/Globe: the system's, passed on untouched.
    System,
    /// A key the keymap cannot name, passed on.
    Unnamed,
    /// A key in a window that is not a browser window (Settings, a system panel).
    Elsewhere,
}

impl Outcome {
    /// True when the browser took the key and nobody else sees it.
    #[must_use]
    pub fn handled(&self) -> bool {
        matches!(
            self,
            Self::Command { .. }
                | Self::Pending
                | Self::ChordUnbound
                | Self::Overlay
                | Self::Extension
                | Self::Quit
        )
    }

    fn describe(&self) -> String {
        match self {
            Self::Command { id, reserved: true } => format!("{id} (reserved)"),
            Self::Command { id, .. } => id.to_string(),
            Self::Pending => "pending: waits for the second stroke".to_owned(),
            Self::ChordUnbound => "chord not bound: dropped".to_owned(),
            Self::Overlay => "overlay key".to_owned(),
            Self::Extension => "an extension's command".to_owned(),
            Self::Quit => "quit".to_owned(),
            Self::Passed => "passed through".to_owned(),
            Self::Returned => "back from the page, which did not take it: passed on".to_owned(),
            Self::System => "passed through: fn/Globe is the system's".to_owned(),
            Self::Unnamed => "passed through: no name in the keymap".to_owned(),
            Self::Elsewhere => "another window's".to_owned(),
        }
    }
}

/// One key press as `AppKit` reported it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Press {
    /// The hardware key code.
    pub code: u16,
    /// The characters it typed, ignoring modifiers.
    pub chars: String,
    /// The modifiers the keymap knows.
    pub modifiers: Modifiers,
    /// Whether `fn`/Globe was down.
    pub function: bool,
    /// The stroke the keymap read, if it read one.
    pub stroke: Option<KeyStroke>,
}

/// The last [`LENGTH`] presses, newest last.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeyLog(VecDeque<(Press, Outcome)>);

impl KeyLog {
    /// Adds a press; the oldest goes once there are [`LENGTH`].
    pub fn push(&mut self, press: Press, outcome: Outcome) {
        if self.0.len() == LENGTH {
            self.0.pop_front();
        }
        self.0.push_back((press, outcome));
    }

    /// One line per press, oldest first, keycaps as `platform` prints them:
    /// `0x25 "l" ⌘ → ⌘L → quickie.address.open`.
    #[must_use]
    pub fn lines(&self, platform: Platform) -> Vec<String> {
        self.0
            .iter()
            .map(|(press, outcome)| line(press, outcome, platform))
            .collect()
    }
}

fn line(press: &Press, outcome: &Outcome, platform: Platform) -> String {
    let mut held: Vec<&str> = press.modifiers.glyphs(platform).collect();
    if press.function {
        held.insert(0, "fn");
    }
    let held = if held.is_empty() {
        "–".to_owned()
    } else {
        held.join("")
    };
    let read = press
        .stroke
        .map_or_else(|| "?".to_owned(), |stroke| stroke.glyphs(platform));
    format!(
        "{:#04x} {} {held} → {read} → {}",
        press.code,
        printable(&press.chars),
        outcome.describe()
    )
}

/// The characters quoted, a control character as its code point.
fn printable(chars: &str) -> String {
    let shown: String = chars
        .chars()
        .map(|c| {
            if c.is_control() || ('\u{F700}'..='\u{F8FF}').contains(&c) {
                format!("U+{:04X}", u32::from(c))
            } else {
                c.to_string()
            }
        })
        .collect();
    format!("\"{shown}\"")
}

#[cfg(test)]
mod tests {
    use quickie_domain_commands::{CommandId, KeyStroke, Modifiers, Platform};

    use super::{KeyLog, LENGTH, Outcome, Press};

    fn press(code: u16, chars: &str, stroke: Option<&str>, function: bool) -> Press {
        let stroke = stroke.map(|text| KeyStroke::try_from(text).unwrap());
        Press {
            code,
            chars: chars.to_owned(),
            modifiers: stroke.map_or(Modifiers::NONE, |stroke| stroke.modifiers),
            function,
            stroke,
        }
    }

    #[test]
    fn a_line_says_the_key_what_was_held_and_what_happened() {
        let mut log = KeyLog::default();
        let id = CommandId::try_from("quickie.address.open").unwrap();
        log.push(
            press(0x25, "l", Some("cmd+l"), false),
            Outcome::Command { id, reserved: true },
        );
        log.push(press(0x03, "f", None, true), Outcome::System);
        log.push(press(0x7E, "\u{F700}", Some("up"), true), Outcome::Passed);
        log.push(press(0x0A, "\t", None, false), Outcome::Unnamed);

        assert_eq!(
            log.lines(Platform::MacOs),
            [
                "0x25 \"l\" ⌘ → ⌘L → quickie.address.open (reserved)",
                "0x03 \"f\" fn → ? → passed through: fn/Globe is the system's",
                "0x7e \"U+F700\" fn → ↑ → passed through",
                "0x0a \"U+0009\" – → ? → passed through: no name in the keymap",
            ]
        );
    }

    #[test]
    fn it_keeps_the_last_presses_only() {
        let mut log = KeyLog::default();
        for code in 0..=u16::try_from(LENGTH).unwrap() {
            log.push(press(code, "a", Some("a"), false), Outcome::Passed);
        }

        let lines = log.lines(Platform::MacOs);
        assert_eq!(lines.len(), LENGTH);
        assert!(lines[0].starts_with("0x01 "), "{lines:?}");
        assert!(lines[LENGTH - 1].starts_with("0x0a "), "{lines:?}");
    }

    #[test]
    fn only_what_the_browser_took_counts_as_handled() {
        let id = CommandId::try_from("quickie.tab.new").unwrap();
        let taken = [
            Outcome::Command {
                id,
                reserved: false,
            },
            Outcome::Pending,
            Outcome::ChordUnbound,
            Outcome::Overlay,
            Outcome::Extension,
            Outcome::Quit,
        ];
        let passed = [
            Outcome::Passed,
            Outcome::Returned,
            Outcome::System,
            Outcome::Unnamed,
            Outcome::Elsewhere,
        ];

        assert!(taken.iter().all(Outcome::handled));
        assert!(!passed.iter().any(Outcome::handled));
    }

    #[test]
    fn every_outcome_reads_as_words() {
        let outcomes = [
            Outcome::Pending,
            Outcome::ChordUnbound,
            Outcome::Overlay,
            Outcome::Quit,
            Outcome::Elsewhere,
            Outcome::Returned,
        ];
        let mut log = KeyLog::default();
        for outcome in outcomes {
            log.push(press(0x00, "a", Some("a"), false), outcome);
        }

        let lines = log.lines(Platform::MacOs);
        assert!(lines[0].ends_with("pending: waits for the second stroke"));
        assert!(lines[1].ends_with("chord not bound: dropped"));
        assert!(lines[2].ends_with("overlay key"));
        assert!(lines[3].ends_with("quit"));
        assert!(lines[4].ends_with("another window's"));
        assert!(lines[5].ends_with("back from the page, which did not take it: passed on"));
    }
}
