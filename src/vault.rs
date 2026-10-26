//! Encrypted on-disk store: Argon2id (64 MiB) -> XChaCha20-Poly1305.
//! Layout: MAGIC | salt(16) | nonce(24) | ciphertext+tag. Header is authenticated as AAD.

use crate::settings::{Settings, data_dir};
use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::{
    Key, XChaCha20Poly1305, XNonce,
    aead::{Aead, AeadCore, KeyInit, OsRng, Payload, rand_core::RngCore},
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Write},
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};
use zeroize::{Zeroize, Zeroizing};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

const MAGIC: &[u8; 6] = b"CROWV1";
const HEADER: usize = 6 + 16;
const NONCE: usize = 24;
pub const MAX_HISTORY: usize = 20_000;

#[derive(Serialize, Deserialize, Clone)]
pub struct HistoryEntry {
    pub url: String,
    pub title: String,
    pub time: u64,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Bookmark {
    pub url: String,
    pub title: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Download {
    pub url: String,
    pub path: String,
    pub state: String, // "active" | "done" | "failed"
    pub time: u64,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct CookieRec {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
    pub same_site: Option<String>,
    pub expires: Option<f64>,
}

// Decrypted vault contents are wiped from RAM when dropped (lock, replacement, trimming), not
// just released to the allocator: browsing data per entry, session tokens per cookie.

impl Drop for HistoryEntry {
    fn drop(&mut self) {
        self.url.zeroize();
        self.title.zeroize();
    }
}

impl Drop for Bookmark {
    fn drop(&mut self) {
        self.url.zeroize();
        self.title.zeroize();
    }
}

impl Drop for Download {
    fn drop(&mut self) {
        self.url.zeroize();
        self.path.zeroize();
    }
}

/// Open-tab URLs of the saved session; wiped on drop like the rest of the vault contents.
#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(transparent)]
pub struct Urls(Vec<String>);

impl Drop for Urls {
    fn drop(&mut self) {
        self.0.iter_mut().for_each(Zeroize::zeroize);
    }
}

impl std::ops::Deref for Urls {
    type Target = Vec<String>;
    fn deref(&self) -> &Vec<String> {
        &self.0
    }
}

impl std::ops::DerefMut for Urls {
    fn deref_mut(&mut self) -> &mut Vec<String> {
        &mut self.0
    }
}

/// Session tokens live in `name`/`value`: wiped whenever a cookie list is replaced or dropped
/// (after export, import into WebKit, deletion or locking).
impl Drop for CookieRec {
    fn drop(&mut self) {
        self.name.zeroize();
        self.value.zeroize();
        self.domain.zeroize();
    }
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
pub struct VaultData {
    pub history: Vec<HistoryEntry>,
    pub bookmarks: Vec<Bookmark>,
    pub downloads: Vec<Download>,
    pub cookies: Vec<CookieRec>,
    pub session: Urls,
    pub active: usize,
    pub settings: Option<Settings>,
    /// Lifetime count of loads blocked by the ad blocker.
    pub blocked_total: u64,
}

pub struct Vault {
    salt: [u8; 16],
    key: Zeroizing<[u8; 32]>,
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

fn path() -> PathBuf {
    data_dir().join("vault.bin")
}

fn derive(password: &str, salt: &[u8]) -> Zeroizing<[u8; 32]> {
    let params = Params::new(64 * 1024, 3, 1, Some(32)).expect("argon2 params");
    let mut key = Zeroizing::new([0u8; 32]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(password.as_bytes(), salt, key.as_mut())
        .expect("argon2");
    key
}

impl Vault {
    pub fn exists() -> bool {
        path().exists()
    }

    pub fn create(password: &str) -> Self {
        let mut salt = [0u8; 16];
        OsRng.fill_bytes(&mut salt);
        Self { key: derive(password, &salt), salt }
    }

    pub fn unlock(password: &str) -> Result<(Self, VaultData), &'static str> {
        let bytes = fs::read(path()).map_err(|_| "vault_unreadable")?;
        if bytes.len() < HEADER + NONCE + 16 || &bytes[..6] != MAGIC {
            return Err("vault_corrupt");
        }
        let salt: [u8; 16] = bytes[6..HEADER].try_into().unwrap();
        let key = derive(password, &salt);
        let plain = XChaCha20Poly1305::new(Key::from_slice(&*key))
            .decrypt(
                XNonce::from_slice(&bytes[HEADER..HEADER + NONCE]),
                Payload { msg: &bytes[HEADER + NONCE..], aad: &bytes[..HEADER] },
            )
            .map_err(|_| "wrong_password")?;
        let plain = Zeroizing::new(plain);
        let data = serde_json::from_slice(&plain).map_err(|_| "vault_corrupt")?;
        Ok((Self { salt, key }, data))
    }

    /// Password change as one transaction: fresh random salt (`OsRng`) and Argon2id key, `data`
    /// encrypted with it and persisted via `.vault.tmp` + `sync_all` + rename (`save`). Only after
    /// that succeeds does `self` switch to the new key (the old one is zeroized on drop); on any
    /// error the vault on disk and in memory both stay on the old password.
    pub fn rotate_key(&mut self, new_password: &str, data: &VaultData) -> io::Result<()> {
        let next = Self::create(new_password);
        next.save(data)?;
        *self = next;
        Ok(())
    }

    pub fn verify(&self, password: &str) -> bool {
        *derive(password, &self.salt) == *self.key
    }

    pub fn save(&self, data: &VaultData) -> io::Result<()> {
        let plain = Zeroizing::new(serde_json::to_vec(data)?);
        let mut out = Vec::with_capacity(HEADER + NONCE + plain.len() + 16);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&self.salt);
        let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
        let ct = XChaCha20Poly1305::new(Key::from_slice(&*self.key))
            .encrypt(&nonce, Payload { msg: &plain, aad: &out[..HEADER] })
            .map_err(|_| io::Error::other("encrypt"))?;
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ct);

        // Transactional: full write + fsync to .vault.tmp, then atomic rename over vault.bin.
        // On any error the previous vault.bin stays intact.
        let dir = data_dir();
        fs::create_dir_all(&dir)?;
        let tmp = dir.join(".vault.tmp");
        let res = (|| {
            let mut opts = fs::OpenOptions::new();
            opts.write(true).create(true).truncate(true);
            #[cfg(unix)]
            opts.mode(0o600);
            let mut f = opts.open(&tmp)?;
            f.write_all(&out)?;
            f.sync_all()?;
            // Windows: MoveFileExW(MOVEFILE_REPLACE_EXISTING), atomic on the same volume.
            fs::rename(&tmp, path())?;
            // Persist the rename itself (directories cannot be opened for fsync on Windows;
            // NTFS journals the rename).
            #[cfg(unix)]
            fs::File::open(&dir)?.sync_all()?;
            Ok(())
        })();
        if res.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        res
    }

    pub fn destroy() {
        let _ = fs::remove_file(path());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// All tests share the one vault file in the test data dir.
    static LOCK: Mutex<()> = Mutex::new(());

    fn sample() -> VaultData {
        let mut data = VaultData::default();
        data.bookmarks.push(Bookmark { url: "https://example.com".into(), title: "Example".into() });
        data.session.push("https://example.org".into());
        data.blocked_total = 42;
        data
    }

    fn assert_sample(data: &VaultData) {
        assert_eq!(data.bookmarks.len(), 1);
        assert_eq!(data.bookmarks[0].url, "https://example.com");
        assert_eq!(data.bookmarks[0].title, "Example");
        assert_eq!(data.session.as_slice(), ["https://example.org"]);
        assert_eq!(data.blocked_total, 42);
    }

    #[test]
    fn roundtrip_with_correct_password() {
        let _g = LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let v = Vault::create("correct horse");
        v.save(&sample()).unwrap();
        assert!(Vault::exists());
        let bytes = fs::read(path()).unwrap();
        assert_eq!(&bytes[..6], MAGIC);
        assert!(!bytes.windows(11).any(|w| w == b"example.com"), "plaintext leaked");
        let (v2, data) = Vault::unlock("correct horse").unwrap();
        assert_sample(&data);
        assert!(v2.verify("correct horse"));
        Vault::destroy();
    }

    #[test]
    fn wrong_password_fails() {
        let _g = LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let v = Vault::create("right");
        v.save(&sample()).unwrap();
        assert!(!v.verify("wrong"));
        assert_eq!(Vault::unlock("wrong").err(), Some("wrong_password"));
        Vault::destroy();
    }

    #[test]
    fn rotate_key_reencrypts_under_new_password() {
        let _g = LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut v = Vault::create("old");
        v.save(&sample()).unwrap();
        let old_salt = v.salt;
        v.rotate_key("new", &sample()).unwrap();
        assert_ne!(v.salt, old_salt);
        assert!(v.verify("new"));
        assert!(!v.verify("old"));
        assert!(!data_dir().join(".vault.tmp").exists());
        assert_eq!(Vault::unlock("old").err(), Some("wrong_password"));
        let (_, data) = Vault::unlock("new").unwrap();
        assert_sample(&data);
        Vault::destroy();
    }
}
