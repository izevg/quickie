//! System adapter of Quickie: what the application needs from macOS itself. For now the
//! [`KeychainSecrets`], the application's `SecretStore`, where LLM API keys live: never in the
//! store, never in a file.
//!
//! The clipboard, the global hotkey, and the keyboard layout join here as their use cases
//! arrive (`docs/architecture.md` §4.1).

mod keychain;

pub use keychain::KeychainSecrets;
