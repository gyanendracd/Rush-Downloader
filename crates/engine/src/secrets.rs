//! Passwords and other secrets. They are kept in the system's own credential
//! store (Windows Credential Manager, the macOS Keychain, the Secret Service
//! on Linux), never in the app's JSON files, so they stay out of backups and
//! of anything copied off the state folder.
//!
//! The store is reached through [`SecretStore`] so tests can use one that
//! lives in memory.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

/// Where secrets are kept, by key ("proxy:3").
pub trait SecretStore: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<String>, String>;
    fn set(&self, key: &str, value: &str) -> Result<(), String>;
    /// Forgets a secret; one that was never kept is not an error.
    fn delete(&self, key: &str) -> Result<(), String>;
}

/// The system's credential store.
pub struct Keyring {
    service: String,
    /// Put before every key, so a test copy of the app started with a
    /// state folder of its own never reads or overwrites the real one's.
    prefix: String,
}

impl Keyring {
    /// Secrets of the app whose state lives in `state_dir`.
    pub fn for_state_dir(state_dir: &Path) -> Keyring {
        let digest = ring::digest::digest(&ring::digest::SHA256, state_dir.to_string_lossy().as_bytes());
        let prefix: String = digest.as_ref()[..6].iter().map(|b| format!("{b:02x}")).collect();
        Keyring { service: "RushDownload".into(), prefix }
    }

    fn entry(&self, key: &str) -> Result<keyring::Entry, String> {
        keyring::Entry::new(&self.service, &format!("{}/{key}", self.prefix))
            .map_err(|e| format!("the system's credential store: {e}"))
    }
}

impl SecretStore for Keyring {
    fn get(&self, key: &str) -> Result<Option<String>, String> {
        match self.entry(key)?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(format!("could not read from the system's credential store: {e}")),
        }
    }

    fn set(&self, key: &str, value: &str) -> Result<(), String> {
        self.entry(key)?
            .set_password(value)
            .map_err(|e| format!("could not save to the system's credential store: {e}"))
    }

    fn delete(&self, key: &str) -> Result<(), String> {
        match self.entry(key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(format!("could not remove from the system's credential store: {e}")),
        }
    }
}

/// Secrets kept only while the program runs, for tests.
#[derive(Default)]
pub struct MemoryStore(Mutex<HashMap<String, String>>);

impl SecretStore for MemoryStore {
    fn get(&self, key: &str) -> Result<Option<String>, String> {
        Ok(self.0.lock().unwrap().get(key).cloned())
    }

    fn set(&self, key: &str, value: &str) -> Result<(), String> {
        self.0.lock().unwrap().insert(key.to_string(), value.to_string());
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<(), String> {
        self.0.lock().unwrap().remove(key);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_test_copy_of_the_app_keeps_its_secrets_apart() {
        let real = Keyring::for_state_dir(Path::new("C:/Users/a/AppData/Roaming/com.rushdownload.app"));
        let test = Keyring::for_state_dir(Path::new("D:/tmp/rush-test"));
        assert_ne!(real.prefix, test.prefix);
        assert_eq!(real.prefix, Keyring::for_state_dir(Path::new("C:/Users/a/AppData/Roaming/com.rushdownload.app")).prefix);
        assert_eq!(real.prefix.len(), 12);
    }

    /// Uses this computer's real credential store (it writes an entry and
    /// removes it again), so it only runs when asked for:
    /// `cargo test -p rushdownload-engine -- --ignored system_credential_store`
    #[test]
    #[ignore]
    fn system_credential_store_keeps_and_forgets() {
        let store = Keyring::for_state_dir(Path::new("rushdownload-secret-store-test"));
        store.set("test:1", "pa:ss/wörd").unwrap();
        assert_eq!(store.get("test:1").unwrap().as_deref(), Some("pa:ss/wörd"));
        store.delete("test:1").unwrap();
        assert_eq!(store.get("test:1").unwrap(), None);
        store.delete("test:1").unwrap();
    }

    #[test]
    fn memory_store_forgets_on_delete() {
        let store = MemoryStore::default();
        assert_eq!(store.get("proxy:1").unwrap(), None);
        store.set("proxy:1", "hunter2").unwrap();
        assert_eq!(store.get("proxy:1").unwrap().as_deref(), Some("hunter2"));
        store.delete("proxy:1").unwrap();
        store.delete("proxy:1").unwrap();
        assert_eq!(store.get("proxy:1").unwrap(), None);
    }
}
