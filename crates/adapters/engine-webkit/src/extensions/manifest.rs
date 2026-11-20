//! What `WebKit` read of an extension's manifest, as the application's values: its name,
//! version, what it asks for, its action, and its keyboard commands with their keys written as
//! `keybindings.json` writes them.

use std::collections::BTreeSet;

use objc2::rc::Retained;
use objc2_app_kit::NSEventModifierFlags;
use objc2_foundation::{NSSet, NSString};
use objc2_web_kit::{WKWebExtension, WKWebExtensionContext, WKWebExtensionMatchPattern};
use quickie_application::{ExtensionCommand, ExtensionInfo};
use quickie_domain_settings::{ExtensionVersion, Grants};

/// The manifest keys of a toolbar action, MV3's and MV2's.
const ACTION_KEYS: [&str; 3] = ["action", "browser_action", "page_action"];

/// What the action is called when the manifest gives it no title.
const DEFAULT_ACTION: &str = "Open";

/// The extension's manifest as the application sees it; `context` gives its commands.
pub(super) fn info(
    extension: &WKWebExtension,
    context: &WKWebExtensionContext,
) -> Result<ExtensionInfo, String> {
    // SAFETY: getters with no preconditions, here and below.
    let name = unsafe { extension.displayName() };
    // SAFETY: as above.
    let short = unsafe { extension.displayShortName() };
    let name = name.or(short).map_or_default(|name| name.to_string());
    // SAFETY: as above.
    let version = unsafe { extension.version() }.map_or_default(|version| version.to_string());
    let version =
        ExtensionVersion::try_from(version.as_str()).map_err(|error| error.to_string())?;
    // SAFETY: as above.
    let description =
        unsafe { extension.displayDescription() }.map_or_default(|text| text.to_string());
    // SAFETY: as above.
    let permissions = strings(&*unsafe { extension.requestedPermissions() });
    // SAFETY: as above; content scripts' matches count, since they need host access to run.
    let patterns = patterns(&*unsafe { extension.allRequestedMatchPatterns() });
    // SAFETY: as above.
    let manifest = unsafe { extension.manifest() };
    let has_action = ACTION_KEYS
        .iter()
        .any(|key| manifest.objectForKey(&NSString::from_str(key)).is_some());
    let action = has_action.then(|| {
        // SAFETY: a getter with no preconditions.
        unsafe { extension.displayActionLabel() }
            .map(|label| label.to_string())
            .filter(|label| !label.is_empty())
            .unwrap_or_else(|| DEFAULT_ACTION.to_owned())
    });
    // SAFETY: a getter; a context lists its commands before it is loaded.
    let commands = unsafe { context.commands() }
        .iter()
        .map(|command| {
            // SAFETY: getters with no preconditions, here and below.
            let name = unsafe { command.identifier() }.to_string();
            // SAFETY: as above.
            let title = unsafe { command.title() }.to_string();
            // SAFETY: as above.
            let key = unsafe { command.activationKey() };
            // SAFETY: as above.
            let flags = unsafe { command.modifierFlags() };
            ExtensionCommand {
                name,
                title,
                shortcut: key.and_then(|key| shortcut(&key.to_string(), flags)),
            }
        })
        .collect();
    Ok(ExtensionInfo {
        name,
        version,
        description,
        requested: Grants {
            permissions,
            patterns,
        },
        action,
        commands,
    })
}

fn strings(set: &NSSet<NSString>) -> BTreeSet<String> {
    set.iter().map(|name| name.to_string()).collect()
}

pub(super) fn patterns(set: &NSSet<WKWebExtensionMatchPattern>) -> BTreeSet<String> {
    set.iter()
        // SAFETY: a getter with no preconditions.
        .map(|pattern| unsafe { pattern.string() }.to_string())
        .collect()
}

/// A command's key as `keybindings.json` writes it (`cmd+shift+y`); `None` for a key Quickie's
/// keymap cannot name.
pub(super) fn shortcut(key: &str, flags: NSEventModifierFlags) -> Option<String> {
    let mut chars = key.chars();
    let (Some(first), None) = (chars.next(), chars.next()) else {
        return None;
    };
    let name = match first {
        '\u{f700}' => "up".to_owned(),
        '\u{f701}' => "down".to_owned(),
        '\u{f702}' => "left".to_owned(),
        '\u{f703}' => "right".to_owned(),
        ' ' => "space".to_owned(),
        key if key.is_ascii_alphanumeric() || key.is_ascii_punctuation() => {
            key.to_ascii_lowercase().to_string()
        }
        _ => return None,
    };
    let mut text = String::new();
    for (flag, word) in [
        (NSEventModifierFlags::Control, "ctrl+"),
        (NSEventModifierFlags::Option, "alt+"),
        (NSEventModifierFlags::Shift, "shift+"),
        (NSEventModifierFlags::Command, "cmd+"),
    ] {
        if flags.contains(flag) {
            text.push_str(word);
        }
    }
    text.push_str(&name);
    Some(text)
}

/// A match pattern for `text`, `None` when `WebKit` does not take it.
pub(super) fn pattern(
    text: &str,
    mtm: objc2::MainThreadMarker,
) -> Option<Retained<WKWebExtensionMatchPattern>> {
    // SAFETY: a plain constructor; it answers nil for a pattern it cannot read.
    unsafe { WKWebExtensionMatchPattern::matchPatternWithString(&NSString::from_str(text), mtm) }
}

#[cfg(test)]
mod tests {
    use objc2_app_kit::NSEventModifierFlags;

    use super::shortcut;

    #[test]
    fn a_command_s_key_is_written_as_the_keymap_writes_it() {
        let flags = NSEventModifierFlags::Command | NSEventModifierFlags::Shift;

        assert_eq!(shortcut("Y", flags), Some("shift+cmd+y".to_owned()));
        assert_eq!(
            shortcut("\u{f700}", NSEventModifierFlags::Option),
            Some("alt+up".to_owned())
        );
        assert_eq!(
            shortcut(",", NSEventModifierFlags::Control),
            Some("ctrl+,".to_owned())
        );
        assert_eq!(shortcut("", flags), None);
        assert_eq!(shortcut("ab", flags), None);
        assert_eq!(shortcut("é", flags), None);
    }
}
