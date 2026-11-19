//! API keys in the Keychain, as generic passwords: the service names Quickie's LLM keys, the
//! account is the key's [`KeyRef`], the origin of the endpoint it belongs to.

#[cfg(test)]
mod tests;

use quickie_application::{SecretError, SecretStore};
use quickie_domain_agents::{ApiKey, KeyRef};
use security_framework::base::Error;
use security_framework::os::macos::keychain::SecKeychain;
use security_framework::os::macos::keychain_item::SecKeychainItem;
use security_framework::os::macos::passwords::{SecKeychainItemPassword, find_generic_password};

/// The Keychain service every LLM key is kept under, as Keychain Access shows it.
const SERVICE: &str = "Quickie LLM API key";

/// The Security framework's `errSecItemNotFound`: no item matches.
const NOT_FOUND: i32 = -25_300;

/// The application's `SecretStore` on a Keychain: the user's default (login) Keychain in the
/// product, or one of its own.
pub struct KeychainSecrets {
    /// The Keychain to use; `None` for the user's default one.
    keychain: Option<SecKeychain>,
}

impl std::fmt::Debug for KeychainSecrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeychainSecrets")
            .field("default", &self.keychain.is_none())
            .finish()
    }
}

impl Default for KeychainSecrets {
    fn default() -> Self {
        Self::login()
    }
}

impl KeychainSecrets {
    /// Keys in the user's default Keychain, the login Keychain.
    #[must_use]
    pub fn login() -> Self {
        Self { keychain: None }
    }

    /// Keys in `keychain`, such as one made for a test.
    #[must_use]
    pub fn in_keychain(keychain: SecKeychain) -> Self {
        Self {
            keychain: Some(keychain),
        }
    }

    /// The item kept under `key`, if there is one.
    fn find(
        &self,
        key: &KeyRef,
    ) -> Result<Option<(SecKeychainItemPassword, SecKeychainItem)>, SecretError> {
        let keychains = self.keychain.as_ref().map(std::slice::from_ref);
        match find_generic_password(keychains, SERVICE, key.as_str()) {
            Ok(found) => Ok(Some(found)),
            Err(error) if error.code() == NOT_FOUND => Ok(None),
            Err(error) => Err(failed(error)),
        }
    }
}

impl SecretStore for KeychainSecrets {
    fn read(&self, key: &KeyRef) -> Result<Option<ApiKey>, SecretError> {
        let Some((password, _item)) = self.find(key)? else {
            return Ok(None);
        };
        let text = std::str::from_utf8(&password)
            .map_err(|_| SecretError(format!("the key for {key} is not text")))?;
        ApiKey::try_from(text)
            .map(Some)
            .map_err(|error| SecretError(format!("the key for {key}: {error}")))
    }

    fn write(&self, key: &KeyRef, value: &ApiKey) -> Result<(), SecretError> {
        let keychain = match &self.keychain {
            Some(keychain) => keychain.clone(),
            None => SecKeychain::default().map_err(failed)?,
        };
        keychain
            .set_generic_password(SERVICE, key.as_str(), value.reveal().as_bytes())
            .map_err(failed)
    }

    fn delete(&self, key: &KeyRef) -> Result<(), SecretError> {
        if let Some((_password, item)) = self.find(key)? {
            item.delete();
        }
        Ok(())
    }
}

fn failed(error: Error) -> SecretError {
    SecretError(error.to_string())
}
