//! The widgets' stories (`docs/quality.md`, UI tests). `cargo test` checks every one against
//! its reference and runs its plays; `cargo xtask ui render <widget>[/<state>]` draws them to
//! `target/ui/<widget>/<state>.<theme>.png`.
//!
//! Each story builds its widget from the fixtures and, where the widget reacts to keys or
//! clicks, plays them. Keys the browser routes before the window sees them (`↓` in the palette,
//! `⏎` in the find bar) go through a route that does what the shell does with them.

use std::process::ExitCode;

use quickie_ui_shared::stories::{self, Story};

mod agent;
mod fixtures;
mod hints;
mod navigation;
mod surfaces;

fn main() -> ExitCode {
    let all: Vec<Story> = navigation::GROUPS
        .into_iter()
        .chain(surfaces::GROUPS)
        .chain([hints::STORIES, agent::STORIES])
        .flatten()
        .copied()
        .collect();
    if stories::run(&all, env!("CARGO_MANIFEST_DIR")) {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
