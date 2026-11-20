//! System adapter of Quickie: what the application needs from macOS itself.
//!
//! - [`KeychainSecrets`], the application's `SecretStore`, where LLM API keys live: never in
//!   the store, never in a file.
//! - [`SystemEntropy`], the application's `Entropy`: the system's cryptographic generator.
//! - [`ExtensionFiles`], the application's `ExtensionPackages`: web extension packages whose
//!   signatures the system checked, kept in the profile.
//!
//! The clipboard, the global hotkey, and the keyboard layout join here as their use cases
//! arrive (`docs/architecture.md` §4.1).

#![expect(
    unsafe_code,
    reason = "CommonCrypto's SHA-256 and Security.framework's `SecKeyCreateWithData`, which \
              security-framework does not wrap, check web extension packages"
)]

mod entropy;
mod keychain;
mod packages;

pub use entropy::SystemEntropy;
pub use keychain::KeychainSecrets;
pub use packages::ExtensionFiles;
