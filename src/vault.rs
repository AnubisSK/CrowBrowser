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
    os::unix::fs::OpenOptionsExt,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};
use zeroize::Zeroizing;

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

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
pub struct VaultData {
    pub history: Vec<HistoryEntry>,
    pub bookmarks: Vec<Bookmark>,
    pub downloads: Vec<Download>,
    pub cookies: Vec<CookieRec>,
    pub session: Vec<String>,
    pub active: usize,
    pub settings: Option<Settings>,
}

pub struct Vault {
    salt: [u8; 16],
    key: Zeroizing<[u8; 32]>,
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
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
        let bytes = fs::read(path()).map_err(|_| "Trezor sa nedá načítať")?;
        if bytes.len() < HEADER + NONCE + 16 || &bytes[..6] != MAGIC {
            return Err("Trezor je poškodený");
        }
        let salt: [u8; 16] = bytes[6..HEADER].try_into().unwrap();
        let key = derive(password, &salt);
        let plain = XChaCha20Poly1305::new(Key::from_slice(&*key))
            .decrypt(
                XNonce::from_slice(&bytes[HEADER..HEADER + NONCE]),
                Payload { msg: &bytes[HEADER + NONCE..], aad: &bytes[..HEADER] },
            )
            .map_err(|_| "Nesprávne heslo")?;
        let plain = Zeroizing::new(plain);
        let data = serde_json::from_slice(&plain).map_err(|_| "Trezor je poškodený")?;
        Ok((Self { salt, key }, data))
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
            let mut f = fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&tmp)?;
            f.write_all(&out)?;
            f.sync_all()?;
            fs::rename(&tmp, path())?;
            // Persist the rename itself.
            fs::File::open(&dir)?.sync_all()
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
