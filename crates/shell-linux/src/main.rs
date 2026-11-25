//! The native GTK4 entry point for the Linux preview.

use std::io::{self, Write};
use std::process::ExitCode;

#[cfg(target_os = "linux")]
mod gtk;
mod launch;
mod mcp;
#[cfg(target_os = "linux")]
mod updates;

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() == Some(std::ffi::OsStr::new("--version")) && args.next().is_none() {
        let version = mcp::product_version();
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

fn run() -> Result<(), String> {
    #[cfg(target_os = "linux")]
    let mut args = std::env::args_os().skip(1).peekable();
    #[cfg(not(target_os = "linux"))]
    let args = std::env::args_os().skip(1);
    #[cfg(target_os = "linux")]
    if args
        .next_if(|argument| argument == updates::install::INSTALL_PACKAGE_ARGUMENT)
        .is_some()
    {
        return updates::install::install_privileged(mcp::product_version(), args)
            .map_err(|error| error.to_string());
    }
    let options = launch::Options::parse(args)?;
    if let launch::Options::Mcp { attach_only } = options {
        return mcp::run(attach_only);
    }
    run_gui(options)
}

#[cfg(target_os = "linux")]
fn run_gui(options: launch::Options) -> Result<(), String> {
    gtk::run(options)
}

#[cfg(not(target_os = "linux"))]
fn run_gui(_options: launch::Options) -> Result<(), String> {
    Err("the GTK preview runs on Linux".into())
}
