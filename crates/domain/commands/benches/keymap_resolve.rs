//! What one key press costs the keymap: the context the shell builds for it, then the lookup.
//!
//! The keymap is the defaults plus a heavy user file: forty more bindings, each behind a `when`
//! clause, and a two-stroke chord. Run with `cargo bench -p quickie-domain-commands`.

#![expect(clippy::unwrap_used, reason = "a broken fixture should stop the bench")]

use std::hint::black_box;

use divan::Bencher;
use quickie_domain_commands::{
    Catalog, ContextFlag, FocusedView, KeyStroke, Keymap, KeymapEntry, Platform, Resolution,
    WhenContext,
};

const PLATFORM: Platform = Platform::MacOs;

fn main() {
    divan::main();
}

/// The defaults and forty user bindings on `ctrl+alt+<key>`, each behind a clause, and the chord
/// `⌘K ⌘S`.
fn keymap() -> Keymap {
    let keys = ('a'..='z').chain('0'..='9').chain(['-', '=', '[', ']']);
    let mut entries: Vec<KeymapEntry> = keys
        .map(|key| {
            KeymapEntry::parse(
                &format!("ctrl+alt+{key}"),
                "quickie.tab.reload",
                Some("focusedView == 'palette' && !inputFocused || hintsOpen"),
                PLATFORM,
            )
            .unwrap()
        })
        .collect();
    entries.push(KeymapEntry::parse("mod+k mod+s", "quickie.page.find", None, PLATFORM).unwrap());
    let keymap = Keymap::defaults(PLATFORM)
        .with_user_entries(&Catalog::builtin(), entries)
        .unwrap();
    let chord = keymap.resolve(stroke("mod+s"), Some(stroke("mod+k")), &page_context());
    assert!(
        matches!(chord, Resolution::Command(_)),
        "the chord is bound"
    );
    keymap
}

fn stroke(text: &str) -> KeyStroke {
    KeyStroke::parse(text, PLATFORM).unwrap()
}

/// What the shell knows while a page has the keyboard and no field is focused.
fn page_context() -> WhenContext {
    WhenContext::new()
        .with(ContextFlag::PageFocused)
        .in_view(FocusedView::Page)
}

/// One press: the context is built per press, as the shell builds it.
fn press(bencher: Bencher, text: &str, pending: Option<&str>) {
    let keymap = keymap();
    let stroke = stroke(text);
    let pending = pending.map(self::stroke);
    bencher.bench(|| {
        let context = page_context();
        black_box(keymap.resolve(black_box(stroke), black_box(pending), &context))
    });
}

/// `⌘⇧P`: the first default, so the last one a newest-first scan reaches.
#[divan::bench]
fn first_default(bencher: Bencher) {
    press(bencher, "mod+shift+p", None);
}

/// `⌘Y`: bound nowhere, so every binding is looked at.
#[divan::bench]
fn unbound(bencher: Bencher) {
    press(bencher, "mod+y", None);
}

/// `⌥F` on a page: a default behind `pageFocused`.
#[divan::bench]
fn behind_a_clause(bencher: Bencher) {
    press(bencher, "alt+f", None);
}

/// `F` typed into a page: no shortcut may start with a key that types, so the shell lets it go
/// on after this check alone, without building a context or asking the keymap.
#[divan::bench]
fn typing(bencher: Bencher) {
    let stroke = stroke("shift+f");
    bencher.bench(|| black_box(black_box(stroke).types_text()));
}

/// `⌘S` after `⌘K`: the second stroke of a chord.
#[divan::bench]
fn second_stroke(bencher: Bencher) {
    press(bencher, "mod+s", Some("mod+k"));
}
