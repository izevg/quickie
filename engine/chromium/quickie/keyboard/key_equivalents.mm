// See key_equivalents.h.

#include "quickie/keyboard/key_equivalents.h"

#import <AppKit/AppKit.h>

#include <cstdint>

#include "components/input/native_web_keyboard_event.h"
#include "quickie_crates/adapters/engine-chromium/src/ffi.rs.h"
#include "third_party/blink/public/common/input/web_input_event.h"

namespace quickie::keyboard {

namespace {

bool Offer(const input::NativeWebKeyboardEvent& event, bool page_declined) {
  // Only the key-down counts; the matching char and key-up events follow it to the page or not
  // (content/public/browser/keyboard_event_processing_result.h).
  if (event.GetType() != blink::WebInputEvent::Type::kRawKeyDown) {
    return false;
  }
  // components/input/native_web_keyboard_event.h: on macOS `os_event` is a
  // base::apple::OwnedNSEvent (base/apple/owned_objc.h, Get()). It is empty for synthetic events
  // (IME composition, tests); those belong to the page.
  if (!event.os_event) {
    return false;
  }
  NSEvent* ns_event = event.os_event.Get();
  if (ns_event.type != NSEventTypeKeyDown) {
    return false;
  }
  // The Rust shell maps keyCode to the ASCII-capable layout itself (KeyStroke, architecture
  // section 5), so ⌘⇧C works on a Russian layout; the character is a hint for display.
  NSString* characters = ns_event.charactersIgnoringModifiers;
  std::uint32_t character = characters.length > 0 ? [characters characterAtIndex:0] : 0;
  return HandleKeyEquivalent(ns_event.keyCode, ns_event.modifierFlags, character,
                             page_declined);
}

}  // namespace

bool OfferBeforePage(const input::NativeWebKeyboardEvent& event) {
  return Offer(event, /*page_declined=*/false);
}

bool OfferAfterPage(const input::NativeWebKeyboardEvent& event) {
  // `skip_if_unhandled` marks key-downs made while an IME composes; they never reach the shell.
  return !event.skip_if_unhandled && Offer(event, /*page_declined=*/true);
}

}  // namespace quickie::keyboard
