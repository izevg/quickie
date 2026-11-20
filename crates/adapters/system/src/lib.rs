//! System adapter of Quickie: what the application needs from macOS itself.
//!
//! - [`KeychainSecrets`], the application's `SecretStore`, where LLM API keys live: never in
//!   the store, never in a file.
//! - [`SystemEntropy`], the application's `Entropy`: the system's cryptographic generator.
//!
//! The clipboard, the global hotkey, and the keyboard layout join here as their use cases
//! arrive (`docs/architecture.md` §4.1).

mod entropy;
mod keychain;

pub use entropy::SystemEntropy;
pub use keychain::KeychainSecrets;
