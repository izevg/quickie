//! System services and extension package verification.
//!
//! - [`KeychainSecrets`], the application's `SecretStore`, where LLM API keys live: never in
//!   the store, never in a file.
//! - [`SystemEntropy`], the application's `Entropy`: the system's cryptographic generator.
//! - [`ExtensionFiles`], the application's `ExtensionPackages`: web extension packages whose
//!   signatures the system checked, kept in the profile.
//!
//! The clipboard, the global hotkey, and the keyboard layout join here as their use cases
//! arrive (`docs/architecture.md` §4.1).

#[cfg(target_os = "macos")]
mod entropy;
#[cfg(target_os = "macos")]
mod keychain;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod packages;

#[cfg(target_os = "macos")]
pub use entropy::SystemEntropy;
#[cfg(target_os = "macos")]
pub use keychain::KeychainSecrets;
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub use packages::ExtensionFiles;
