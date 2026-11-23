//! The native GTK4 entry point for the Linux preview.

use std::io::{self, Write};
use std::process::ExitCode;

#[cfg(target_os = "linux")]
mod gtk;
#[cfg(any(target_os = "linux", test))]
mod launch;
#[cfg(target_os = "linux")]
mod updates;

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() == Some(std::ffi::OsStr::new("--version")) && args.next().is_none() {
        let version = option_env!("QUICKIE_RELEASE_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"));
        return if writeln!(io::stdout().lock(), "{version}").is_ok() {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        };
    }
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
    let mut args = std::env::args_os().skip(1).peekable();
    if args
        .next_if(|argument| argument == updates::install::INSTALL_PACKAGE_ARGUMENT)
        .is_some()
    {
        let current = option_env!("QUICKIE_RELEASE_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"));
        return updates::install::install_privileged(current, args)
            .map_err(|error| error.to_string());
    }
    gtk::run(launch::Options::parse(args)?)
}

#[cfg(not(target_os = "linux"))]
fn run() -> Result<(), String> {
    Err("the GTK preview runs on Linux".into())
}
