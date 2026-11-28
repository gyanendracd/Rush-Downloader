//! Secrets too large for the system's credential store (Windows keeps at
//! most 2.5 KB per entry, and a site's cookies are often more): sign-in
//! passwords, tokens, cookies and private headers of downloads and request
//! profiles. They are kept in `secrets.vault`, encrypted with AES-256-GCM
//! under a random key that lives in the credential store itself, so the
//! file alone (in a backup, or copied off the computer) gives nothing away.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM, NONCE_LEN};
use ring::rand::{SecureRandom, SystemRandom};

use crate::secrets::SecretStore;

const KEY_NAME: &str = "vault-key";

pub(crate) struct Vault {
    file: PathBuf,
    store: Arc<dyn SecretStore>,
    /// What the file holds, once read.
    open: Mutex<Option<HashMap<String, String>>>,
}

impl Vault {
    pub(crate) fn new(file: PathBuf, store: Arc<dyn SecretStore>) -> Vault {
        Vault { file, store, open: Mutex::new(None) }
    }

    /// The key, made the first time a secret is kept.
    fn key(&self, create: bool) -> Result<Option<LessSafeKey>, String> {
        let b64 = base64::engine::general_purpose::STANDARD;
        let raw = match self.store.get(KEY_NAME)? {
            Some(text) => b64.decode(text).map_err(|_| "the vault's key in the credential store is damaged".to_string())?,
            None if create => {
                let mut fresh = [0u8; 32];
                SystemRandom::new().fill(&mut fresh).map_err(|_| "no random numbers for a new key".to_string())?;
                self.store.set(KEY_NAME, &b64.encode(fresh))?;
                fresh.to_vec()
            }
            None => return Ok(None),
        };
        let key = UnboundKey::new(&AES_256_GCM, &raw).map_err(|_| "the vault's key has the wrong length".to_string())?;
        Ok(Some(LessSafeKey::new(key)))
    }

    fn load(&self) -> Result<HashMap<String, String>, String> {
        let Ok(sealed) = std::fs::read(&self.file) else {
            return Ok(HashMap::new());
        };
        if sealed.len() < NONCE_LEN {
            return Err("the vault file is damaged".into());
        }
        let key = self.key(false)?.ok_or("the vault's key is missing from the credential store; its secrets cannot be read")?;
        let (nonce, body) = sealed.split_at(NONCE_LEN);
        let mut body = body.to_vec();
        let nonce = Nonce::try_assume_unique_for_key(nonce).map_err(|_| "the vault file is damaged".to_string())?;
        let plain = key
            .open_in_place(nonce, Aad::from(b"rushdownload-vault-1"), &mut body)
            .map_err(|_| "the vault could not be opened (damaged, or another computer's)".to_string())?;
        serde_json::from_slice(plain).map_err(|_| "the vault file is damaged".to_string())
    }

    fn save(&self, map: &HashMap<String, String>) -> Result<(), String> {
        let key = self.key(true)?.expect("made when missing");
        let mut nonce = [0u8; NONCE_LEN];
        SystemRandom::new().fill(&mut nonce).map_err(|_| "no random numbers".to_string())?;
        let mut body = serde_json::to_vec(map).map_err(|e| e.to_string())?;
        key.seal_in_place_append_tag(Nonce::assume_unique_for_key(nonce), Aad::from(b"rushdownload-vault-1"), &mut body)
            .map_err(|_| "could not encrypt the vault".to_string())?;
        let mut sealed = nonce.to_vec();
        sealed.extend_from_slice(&body);
        let tmp = self.file.with_extension("vault.tmp");
        std::fs::write(&tmp, sealed).map_err(|e| format!("could not save secrets: {e}"))?;
        std::fs::rename(&tmp, &self.file).map_err(|e| format!("could not save secrets: {e}"))
    }

    fn with<T>(&self, f: impl FnOnce(&mut HashMap<String, String>) -> T) -> Result<(T, bool), String> {
        let mut open = self.open.lock().unwrap();
        if open.is_none() {
            *open = Some(self.load()?);
        }
        let map = open.as_mut().expect("loaded above");
        let before = map.clone();
        let out = f(map);
        Ok((out, *map != before))
    }

    pub(crate) fn get(&self, name: &str) -> Result<Option<String>, String> {
        Ok(self.with(|m| m.get(name).cloned())?.0)
    }

    pub(crate) fn set(&self, name: &str, value: &str) -> Result<(), String> {
        let (_, changed) = self.with(|m| {
            m.insert(name.to_string(), value.to_string());
        })?;
        if changed {
            self.save(self.open.lock().unwrap().as_ref().expect("loaded"))?;
        }
        Ok(())
    }

    pub(crate) fn delete(&self, name: &str) -> Result<(), String> {
        let (_, changed) = self.with(|m| {
            m.remove(name);
        })?;
        if changed {
            self.save(self.open.lock().unwrap().as_ref().expect("loaded"))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::MemoryStore;

    #[test]
    fn secrets_are_kept_encrypted_and_come_back() {
        let dir = std::env::temp_dir().join(format!("rd-vault-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store: Arc<dyn SecretStore> = Arc::new(MemoryStore::default());
        let file = dir.join("secrets.vault");
        let vault = Vault::new(file.clone(), store.clone());
        assert_eq!(vault.get("request:1").unwrap(), None, "nothing yet, and no key made just to read");
        assert!(store.get(KEY_NAME).unwrap().is_none());

        let cookies = "session=".to_string() + &"x".repeat(8000);
        vault.set("request:1", &cookies).unwrap();
        let on_disk = std::fs::read(&file).unwrap();
        assert!(!String::from_utf8_lossy(&on_disk).contains("session="), "the file is not readable as it is");

        // a new vault on the same file and key reads it back
        let again = Vault::new(file.clone(), store.clone());
        assert_eq!(again.get("request:1").unwrap().as_deref(), Some(cookies.as_str()));
        again.delete("request:1").unwrap();
        assert_eq!(Vault::new(file.clone(), store.clone()).get("request:1").unwrap(), None);

        // another computer's key (or a damaged file) is told, not read as empty
        again.set("x", "y").unwrap();
        let stranger = Vault::new(file.clone(), Arc::new(MemoryStore::default()));
        assert!(stranger.get("x").unwrap_err().contains("missing"));
        let other: Arc<dyn SecretStore> = Arc::new(MemoryStore::default());
        other.set(KEY_NAME, &base64::engine::general_purpose::STANDARD.encode([7u8; 32])).unwrap();
        assert!(Vault::new(file.clone(), other).get("x").unwrap_err().contains("could not be opened"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
