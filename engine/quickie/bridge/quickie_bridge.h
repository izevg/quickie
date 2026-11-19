// The C++ side of the cxx bridge between Chromium and the Rust shell. The Rust side is
// crates/adapters/engine-chromium/src/ffi.rs; both change together. cxx generates
// "quickie_crates/adapters/engine-chromium/src/ffi.rs.h" from it, which includes this header first
// and defines the shared types forward-declared here.
//
// Every function runs on the browser UI thread (the macOS main thread). Chromium builds without
// exceptions, so failures are reported as `false`, never thrown; details go to the log.

#ifndef QUICKIE_BRIDGE_QUICKIE_BRIDGE_H_
#define QUICKIE_BRIDGE_QUICKIE_BRIDGE_H_

#include <cstdint>

#include "third_party/rust/cxx/v1/cxx.h"

namespace quickie {

// Shared with Rust; defined by the generated ffi.rs.h. The underlying type must match the
// `#[repr(u8)]` there.
enum class NavigationKind : ::std::uint8_t;

// The NSView of a shell window. Opaque: only //quickie/browser_window casts it back.
struct HostView;

// Creates the page for `tab` and starts loading `url`: in the main profile, or in throwaway jar
// `jar` (an off-the-record profile per jar, see tab_registry.h) when `throwaway` is set.
bool CreateTab(std::uint64_t tab, rust::Str url, bool throwaway, std::uint64_t jar);

// Shows `tab` in shell window `window`, inside `host_view`. The first call for a window binds
// it to `host_view`.
bool ShowTab(std::uint64_t window, HostView* host_view, std::uint64_t tab);

// Navigates `tab`; `url` is used by NavigationKind::Load only.
bool NavigateTab(std::uint64_t tab, NavigationKind kind, rust::Str url);

// Destroys `tab`'s page.
bool CloseTab(std::uint64_t tab);

// Gives a page Chromium opened (reported as EngineEventKind::PageOpenedTab) its tab id.
bool AdoptTab(std::uint64_t page, std::uint64_t tab);

// Destroys throwaway jar `jar` and its storage. Fails while a tab still uses it.
bool DestroyJar(std::uint64_t jar);

}  // namespace quickie

#endif  // QUICKIE_BRIDGE_QUICKIE_BRIDGE_H_
