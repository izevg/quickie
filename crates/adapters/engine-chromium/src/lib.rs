//! Engine adapter on Chromium: [`ChromiumEngine`] implements the application's
//! [`EnginePort`](quickie_application::EnginePort) over the cxx bridge to `//quickie`
//! (`engine/quickie/bridge`).
//!
//! GN builds this crate into the product with the `chromium` feature (`engine/quickie/BUILD.gn`).
//! Cargo builds it without the feature: the C++ side exists only inside a Chromium checkout, so
//! the calls into it are compiled out and every engine call fails as unavailable. The bookkeeping
//! on top of the bridge is tested against a fake bridge.
//!
//! Page events flow back without re-entering the use cases: the C++ side reports them, the
//! engine records them and posts a wake to Chromium's UI thread, and the shell's waker drains
//! [`ChromiumEngine::take_navigations`] into `Browsing::navigated`, the same shape as the
//! `WebKit` engine's.
#![expect(
    unsafe_code,
    reason = "FFI: the cxx bridge to Chromium and the NSView pointer of a shell window"
)]

mod bridge;
mod engine;
mod ffi;

pub use bridge::KeyEquivalent;
pub use engine::{ChromiumEngine, OpenedPage, PageHandle};
