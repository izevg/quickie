//! FSD `screens` layer of Quickie's UI: whole windows of their own, composed from the layers
//! below. For now one slice, [`settings`]: the Settings window behind `⌘,`.

#![expect(
    unsafe_code,
    reason = "AppKit through objc2: the Settings window, its panes' flipped views, and its \
              buttons' click target"
)]

pub mod settings;

pub use settings::{
    GeneralState, KeysState, ListState, Pane, SettingsAction, SettingsScreen, ShieldsState,
};
