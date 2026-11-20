//! API keys in the Keychain, as generic passwords: the service names Quickie's LLM keys, the
//! account is the key's [`KeyRef`], the origin of the endpoint it belongs to.
//!
//! Two rules keep another app of the same user from the key:
//!
//! - **Only one Keychain is searched**, the login Keychain (or the one given), never the whole
//!   search list, so an app that adds a Keychain of its own cannot answer with a decoy key.
//! - **A key is never written into an existing item.** The service and account names are
//!   public, so another app can create the item first with an access list that lets it read
//!   it; writing the key into that item would hand the key over. Every write deletes all
//!   matching items, then adds a new one, which gets Quickie's own access list.
//!
//! TODO(signed build): move to the data-protection Keychain (`SecItem` with
//! `kSecUseDataProtectionKeychain` and Quickie's access group), where no other app can create
//! or read Quickie's items at all. It needs a signed app with a Keychain access group; the
//! development binary has neither.

#[cfg(test)]
mod tests;

use quickie_application::{SecretError, SecretStore};
use quickie_domain_agents::{ApiKey, KeyRef};
use security_framework::base::Error;
use security_framework::item::{ItemClass, ItemSearchOptions};
use security_framework::os::macos::keychain::SecKeychain;
use security_framework::os::macos::passwords::find_generic_password;

/// The Keychain service every LLM key is kept under, as Keychain Access shows it.
const SERVICE: &str = "Quickie LLM API key";

/// The Security framework's `errSecItemNotFound`: no item matches.
const NOT_FOUND: i32 = -25_300;

/// How often a delete is tried while items are left: each round deletes every match, so a
/// second round is only for one created meanwhile.
const MAX_DELETE_ROUNDS: usize = 4;

/// The application's `SecretStore` on one Keychain: the user's login Keychain in the product,
/// or one of its own. A clone uses the same Keychain, so a worker thread can read the key the
/// main thread saved.
#[derive(Clone)]
pub struct KeychainSecrets {
    /// The Keychain to use; `None` for the user's login Keychain.
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
    /// Keys in the user's login Keychain, its default one.
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

    /// The one Keychain searched and written.
    fn keychain(&self) -> Result<SecKeychain, SecretError> {
        match &self.keychain {
            Some(keychain) => Ok(keychain.clone()),
            None => SecKeychain::default().map_err(failed),
        }
    }

    /// Deletes every item kept under `key` in `keychain`, and checks that none is left. No item
    /// is no error; anything else that keeps an item there is, such as a locked Keychain or a
    /// denied prompt. A delete alone cannot tell: in a locked Keychain it finds nothing to
    /// delete and reports so, while the key stays.
    fn delete_all(keychain: &SecKeychain, key: &KeyRef) -> Result<(), SecretError> {
        for _ in 0..MAX_DELETE_ROUNDS {
            match find_generic_password(Some(std::slice::from_ref(keychain)), SERVICE, key.as_str())
            {
                Err(error) if error.code() == NOT_FOUND => return Ok(()),
                Err(error) => return Err(failed(error)),
                Ok(_) => ItemSearchOptions::new()
                    .class(ItemClass::generic_password())
                    .service(SERVICE)
                    .account(key.as_str())
                    .keychains(std::slice::from_ref(keychain))
                    .delete()
                    .map_err(failed)?,
            }
        }
        Err(SecretError(format!("the key for {key} cannot be deleted")))
    }
}

impl SecretStore for KeychainSecrets {
    fn read(&self, key: &KeyRef) -> Result<Option<ApiKey>, SecretError> {
        let keychain = self.keychain()?;
        let found =
            find_generic_password(Some(std::slice::from_ref(&keychain)), SERVICE, key.as_str());
        let password = match found {
            Ok((password, _item)) => password,
            Err(error) if error.code() == NOT_FOUND => return Ok(None),
            Err(error) => return Err(failed(error)),
        };
        let text = std::str::from_utf8(&password)
            .map_err(|_| SecretError(format!("the key for {key} is not text")))?;
        ApiKey::try_from(text)
            .map(Some)
            .map_err(|error| SecretError(format!("the key for {key}: {error}")))
    }

    fn write(&self, key: &KeyRef, value: &ApiKey) -> Result<(), SecretError> {
        let keychain = self.keychain()?;
        Self::delete_all(&keychain, key)?;
        keychain
            .add_generic_password(SERVICE, key.as_str(), value.reveal().as_bytes())
            .map_err(failed)
    }

    fn delete(&self, key: &KeyRef) -> Result<(), SecretError> {
        Self::delete_all(&self.keychain()?, key)
    }
}

fn failed(error: Error) -> SecretError {
    SecretError(error.to_string())
}
