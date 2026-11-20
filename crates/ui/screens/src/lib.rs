//! FSD `screens` layer of Quickie's UI: whole windows of their own, composed from the layers
//! below. For now one slice, [`settings`]: the Settings window behind `⌘,`.

#![expect(
    unsafe_code,
    reason = "AppKit through objc2: the Settings window, its flipped views, its tables' data \
              sources and key handling, its rows' drawing, and its buttons' click target"
)]

pub mod settings;

pub use settings::{
    ExtensionState, ExtensionsState, GeneralState, KeyPress, KeysState, ListState, Pane, Recorded,
    SettingsAction, SettingsScreen, ShieldsState,
};
