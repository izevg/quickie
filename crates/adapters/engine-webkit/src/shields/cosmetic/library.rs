//! The scriptlet library as `adblock` resources.
//!
//! The library itself is Quickie's own JavaScript (`js/scriptlets.js`, Apache-2.0), shipped in
//! the binary; nothing is fetched for it and nothing GPL is in it (`docs/privacy.md`). `adblock`
//! only needs to know each scriptlet's name, its aliases, and what to write for a rule: a small
//! function that calls into the library under the scriptlet's name. The library sits once in
//! the shields script, and each site's rules only add their calls.

use adblock::resources::{MimeType, PermissionMask, Resource, ResourceType};

/// The library: an expression whose value is `run(name, args)`.
pub(crate) const LIBRARY: &str = include_str!("js/scriptlets.js");

/// The permission the trusted scriptlets need: only lists whose rules may use them
/// (`FilterListId::trusted`) are parsed with it.
pub(crate) const TRUSTED: PermissionMask = PermissionMask::from_bits(0b0000_0001);

/// The name the library function goes by in the shields script.
pub(crate) const RUN: &str = "quickieScriptlet";

/// Every scriptlet: its name as rules write it, its aliases, and whether it needs
/// [`TRUSTED`]. Each is a key of the library in `js/scriptlets.js`.
pub(crate) const SCRIPTLETS: [(&str, &[&str], bool); 28] = [
    ("set-constant", &["set"], false),
    ("trusted-set-constant", &["trusted-set"], true),
    ("json-prune", &[], false),
    ("json-prune-fetch-response", &[], false),
    ("json-prune-xhr-response", &[], false),
    ("trusted-replace-fetch-response", &[], true),
    ("trusted-replace-xhr-response", &[], true),
    ("abort-on-property-read", &["aopr"], false),
    ("abort-on-property-write", &["aopw"], false),
    (
        "abort-current-script",
        &["acs", "abort-current-inline-script", "acis"],
        false,
    ),
    (
        "no-setTimeout-if",
        &["nostif", "prevent-setTimeout", "setTimeout-defuser"],
        false,
    ),
    (
        "no-setInterval-if",
        &["nosiif", "prevent-setInterval", "setInterval-defuser"],
        false,
    ),
    (
        "adjust-setTimeout",
        &["nano-setTimeout-booster", "nano-stb"],
        false,
    ),
    (
        "adjust-setInterval",
        &["nano-setInterval-booster", "nano-sib"],
        false,
    ),
    (
        "addEventListener-defuser",
        &["aeld", "prevent-addEventListener"],
        false,
    ),
    ("no-fetch-if", &["prevent-fetch"], false),
    ("no-xhr-if", &["prevent-xhr"], false),
    ("noeval-if", &["prevent-eval-if"], false),
    ("noeval", &["silent-noeval"], false),
    (
        "no-window-open-if",
        &["nowoif", "prevent-window-open"],
        false,
    ),
    ("remove-attr", &["ra"], false),
    ("remove-class", &["rc"], false),
    ("set-cookie", &[], false),
    ("set-local-storage-item", &[], false),
    ("set-session-storage-item", &[], false),
    ("trusted-prune-inbound-object", &[], true),
    ("trusted-prune-outbound-object", &[], true),
    ("trusted-replace-outbound-text", &[], true),
];

/// The scriptlets as `adblock` resources: one small function per scriptlet, named after it,
/// that hands its arguments to the library.
pub(crate) fn resources() -> Vec<Resource> {
    SCRIPTLETS
        .iter()
        .map(|&(name, aliases, trusted)| Resource {
            name: format!("{name}.js"),
            aliases: aliases.iter().map(|alias| format!("{alias}.js")).collect(),
            kind: ResourceType::Mime(MimeType::ApplicationJavascript),
            content: base64(&stub(name)),
            dependencies: Vec::new(),
            permission: if trusted {
                TRUSTED
            } else {
                PermissionMask::default()
            },
        })
        .collect()
}

/// The function `adblock` writes for a scriptlet, which rules then call by its name.
fn stub(name: &str) -> String {
    format!(
        "function {}(...args) {{ {RUN}({name:?}, args); }}",
        function_name(name)
    )
}

/// `json-prune` → `quickie_json_prune`: a JavaScript identifier unique to the scriptlet.
fn function_name(name: &str) -> String {
    let body: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("quickie_{body}")
}

/// Standard base64, as `Resource::content` holds it.
fn base64(text: &str) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let triple = chunk.iter().enumerate().fold(0u32, |acc, (at, &byte)| {
            acc | u32::from(byte) << (16 - 8 * at)
        });
        for at in 0..4 {
            if at <= chunk.len() {
                let index = (triple >> (18 - 6 * at)) & 0x3f;
                out.push(char::from(ALPHABET[index as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{LIBRARY, SCRIPTLETS, base64, function_name, resources, stub};

    #[test]
    fn base64_matches_the_standard_alphabet_and_padding() {
        assert_eq!(base64(""), "");
        assert_eq!(base64("a"), "YQ==");
        assert_eq!(base64("ab"), "YWI=");
        assert_eq!(base64("abc"), "YWJj");
        assert_eq!(base64("abcd"), "YWJjZA==");
        assert_eq!(base64("\u{ff}~"), "w79+");
    }

    #[test]
    fn a_stub_calls_the_library_under_the_scriptlet_s_name() {
        assert_eq!(
            stub("json-prune"),
            "function quickie_json_prune(...args) { quickieScriptlet(\"json-prune\", args); }"
        );
        assert_eq!(
            function_name("no-setTimeout-if"),
            "quickie_no_setTimeout_if"
        );
    }

    #[test]
    fn every_scriptlet_is_in_the_library() {
        for (name, _, _) in SCRIPTLETS {
            assert!(LIBRARY.contains(&format!("'{name}':")), "{name}");
        }
    }

    #[test]
    fn names_and_aliases_are_unique() {
        let mut all: Vec<String> = resources()
            .into_iter()
            .flat_map(|resource| std::iter::once(resource.name).chain(resource.aliases))
            .collect();
        let count = all.len();
        all.sort();
        all.dedup();

        assert_eq!(all.len(), count);
    }
}
