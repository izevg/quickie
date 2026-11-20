// Key equivalents for the Rust shell (docs/architecture.md, section 4.4). Chromium offers every
// key-down to the BrowserWindow twice: PreHandleKeyboardEvent before the renderer sees it, and
// HandleKeyboardEvent when the page did not handle it. Both go to Rust through the bridge
// (HandleKeyEquivalent): the shell takes its reserved shortcuts (⌘T, ⌘W, ⌃Tab, the palette) in the
// first pass, so a page cannot swallow them, and its other bindings in the second.

#ifndef QUICKIE_KEYBOARD_KEY_EQUIVALENTS_H_
#define QUICKIE_KEYBOARD_KEY_EQUIVALENTS_H_

namespace input {
struct NativeWebKeyboardEvent;
}

namespace quickie::keyboard {

// Before the renderer: true if the shell consumed the key (a reserved shortcut).
bool OfferBeforePage(const input::NativeWebKeyboardEvent& event);

// After the renderer declined it: true if the shell handled the key.
bool OfferAfterPage(const input::NativeWebKeyboardEvent& event);

}  // namespace quickie::keyboard

#endif  // QUICKIE_KEYBOARD_KEY_EQUIVALENTS_H_
