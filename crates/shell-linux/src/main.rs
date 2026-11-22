//! The native GTK4 entry point for the Linux preview.

use std::io::{self, Write};
use std::process::ExitCode;

#[cfg(target_os = "linux")]
mod gtk;
#[cfg(any(target_os = "linux", test))]
mod launch;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _ = writeln!(io::stderr(), "Quickie: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(target_os = "linux")]
fn run() -> Result<(), String> {
    let args = std::env::args_os().skip(1);
    gtk::run(launch::Options::parse(args)?)
}

#[cfg(not(target_os = "linux"))]
fn run() -> Result<(), String> {
    Err("the GTK preview runs on Linux".into())
}
