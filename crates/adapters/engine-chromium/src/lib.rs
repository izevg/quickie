//! Engine adapter on Chromium: [`ChromiumEngine`] is one whole
//! [`Engine`](quickie_application::Engine) of the application over the cxx bridge to `//quickie`
//! (`engine/chromium/quickie/bridge`). What the bridge does not carry yet (page content, the
//! page surface beyond host views, downloads, the browser's own requests, the content blocker)
//! fails as such, without calling Chromium.
//!
//! GN builds this crate into the product with the `chromium` feature
//! (`engine/chromium/quickie/BUILD.gn`).
//! Cargo builds it without the feature: the C++ side exists only inside a Chromium checkout, so
//! the calls into it are compiled out and every engine call fails as unavailable. The bookkeeping
//! on top of the bridge is tested against a fake bridge.
//!
//! Page events flow back without re-entering the use cases: the C++ side reports them, the
//! engine records them as the application's `EngineEvent`s in one queue and posts a wake to
//! Chromium's UI thread, and the shell's waker drains them
//! ([`Engine::drain`](quickie_application::Engine::drain)), the same shape as the `WebKit`
//! engine's.
#![expect(
    unsafe_code,
    reason = "FFI: the cxx bridge to Chromium and the NSView pointer of a shell window, which \
              the engine keeps retained"
)]

mod bridge;
mod engine;
mod ffi;

pub use bridge::KeyEquivalent;
pub use engine::{ChromiumContentBlocker, ChromiumEngine, ChromiumHttp, PageHandle};
