//! Profiles: each one is `data_dir()/profiles/<id>/` with its own encrypted `vault.bin` (own salt,
//! own Argon2id key). Only the display name (the lock screen lists it before any password is
//! known) and the failed-attempt counter are plain.

use crate::{
    settings::data_dir,
    vault::{Vault, VaultData, now_ms},
};
use chacha20poly1305::aead::{OsRng, rand_core::RngCore};
use std::{
    fs,
    path::{Path, PathBuf},
};

const NAME_FILE: &str = "name";
const ATTEMPTS_FILE: &str = "attempts";
/// Fixed id of the vault of an older single-profile install: a retried migration stays one profile.
const LEGACY_ID: &str = "0000000000000000";
const MAX_NAME: usize = 40;
/// Wrong passwords answered without delay (typing mistakes).
const FREE_ATTEMPTS: u32 = 3;
const MAX_WAIT_SECS: u64 = 900;

pub struct Profile {
    pub id: String,
    pub name: String,
    pub dir: PathBuf,
}

/// Why an unlock did not happen.
#[derive(Debug, PartialEq, Eq)]
pub enum Denied {
    /// Seconds until the next attempt is accepted.
    Wait(u64),
    /// `locales.json` key.
    Failed(&'static str),
}

/// Seconds to refuse attempts after `failures` wrong passwords: doubles, capped.
fn delay_after(failures: u32) -> u64 {
    if failures < FREE_ATTEMPTS {
        return 0;
    }
    (1u64 << (failures - FREE_ATTEMPTS + 1).min(20)).min(MAX_WAIT_SECS)
}

fn now_secs() -> u64 {
    now_ms() / 1000
}

fn root() -> PathBuf {
    data_dir().join("profiles")
}

fn valid_id(id: &str) -> bool {
    id.len() == 16 && id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn clean_name(name: &str) -> Option<String> {
    let name = name.trim();
    (!name.is_empty() && name.chars().count() <= MAX_NAME && !name.chars().any(char::is_control)).then(|| name.to_string())
}

fn make_dir(dir: &Path) -> std::io::Result<()> {
    let mut b = fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
    b.create(dir)
}

fn read(root: &Path, id: &str) -> Option<Profile> {
    let dir = root.join(id);
    if !valid_id(id) || !Vault::exists(&dir) {
        return None;
    }
    let stored = fs::read_to_string(dir.join(NAME_FILE)).unwrap_or_default();
    let name = clean_name(&stored).unwrap_or_else(|| id[..6].to_string());
    Some(Profile { id: id.to_string(), name, dir })
}

fn list_in(root: &Path) -> Vec<Profile> {
    let mut all: Vec<_> = fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| read(root, &e.file_name().to_string_lossy()))
        .collect();
    all.sort_by_key(|p| p.name.to_lowercase());
    all
}

fn create_in(root: &Path, name: &str) -> Result<Profile, &'static str> {
    let name = clean_name(name).ok_or("profile_name_invalid")?;
    if list_in(root).iter().any(|p| p.name.to_lowercase() == name.to_lowercase()) {
        return Err("profile_name_taken");
    }
    let mut raw = [0u8; 8];
    OsRng.fill_bytes(&mut raw);
    let id: String = raw.iter().map(|b| format!("{b:02x}")).collect();
    let dir = root.join(&id);
    make_dir(&dir).and_then(|()| fs::write(dir.join(NAME_FILE), &name)).map_err(|_| "vault_save_failed")?;
    Ok(Profile { id, name, dir })
}

/// The vault of a single-profile install becomes a profile. The rename is atomic: it is either
/// still the old file or already the profile's, and a retry after a crash continues.
fn migrate_in(base: &Path, root: &Path, name: &str) -> std::io::Result<()> {
    let old = base.join("vault.bin");
    if !old.is_file() {
        return Ok(());
    }
    let dir = root.join(LEGACY_ID);
    make_dir(&dir)?;
    fs::write(dir.join(NAME_FILE), name)?;
    fs::rename(old, Vault::file(&dir))
}

pub fn list() -> Vec<Profile> {
    list_in(&root())
}

pub fn find(id: &str) -> Option<Profile> {
    read(&root(), id)
}

/// Empty profile folder; the caller writes its first vault.
pub fn create(name: &str) -> Result<Profile, &'static str> {
    create_in(&root(), name)
}

pub fn migrate_legacy(name: &str) {
    if let Err(e) = migrate_in(&data_dir(), &root(), name) {
        eprintln!("profile migration failed: {e}");
    }
}

impl Profile {
    pub fn is_legacy(&self) -> bool {
        self.id == LEGACY_ID
    }

    pub fn delete(&self) {
        let _ = fs::remove_dir_all(&self.dir);
    }

    fn attempts(&self) -> (u32, u64) {
        let text = fs::read_to_string(self.dir.join(ATTEMPTS_FILE)).unwrap_or_default();
        let mut parts = text.split_whitespace().map(str::parse::<u64>);
        match (parts.next(), parts.next()) {
            (Some(Ok(n)), Some(Ok(until))) => (u32::try_from(n).unwrap_or(u32::MAX), until),
            _ => (0, 0),
        }
    }

    /// Seconds before another password is accepted (capped: a changed clock cannot extend it).
    fn wait_secs(&self) -> u64 {
        self.attempts().1.saturating_sub(now_secs()).min(MAX_WAIT_SECS)
    }

    fn record_failure(&self) -> u64 {
        let failures = self.attempts().0.saturating_add(1);
        let wait = delay_after(failures);
        let _ = fs::write(self.dir.join(ATTEMPTS_FILE), format!("{failures} {}", now_secs() + wait));
        wait
    }

    /// The master password gate. While the backoff runs nothing is derived or decrypted, right
    /// password or not. The counter is on disk so that restarting the browser does not reset it.
    pub fn unlock(&self, password: &str) -> Result<(Vault, VaultData), Denied> {
        let wait = self.wait_secs();
        if wait > 0 {
            return Err(Denied::Wait(wait));
        }
        match Vault::unlock(&self.dir, password) {
            Ok(opened) => {
                let _ = fs::remove_file(self.dir.join(ATTEMPTS_FILE));
                Ok(opened)
            }
            Err("wrong_password") => match self.record_failure() {
                0 => Err(Denied::Failed("wrong_password")),
                wait => Err(Denied::Wait(wait)),
            },
            Err(e) => Err(Denied::Failed(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::Bookmark;

    fn root_for(test: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("crow-profile-test-{}-{test}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        root
    }

    const SECRET_URL: &str = "https://secret-bookmark.example";

    fn sealed(root: &Path, name: &str, password: &str) -> Profile {
        let p = create_in(root, name).unwrap();
        let mut data = VaultData::default();
        data.bookmarks.push(Bookmark { url: SECRET_URL.into(), title: "Hidden title".into() });
        data.session.push("https://secret-tab.example".into());
        Vault::create(&p.dir, password).save(&data).unwrap();
        p
    }

    fn files(dir: &Path) -> Vec<Vec<u8>> {
        fs::read_dir(dir).unwrap().flatten().filter(|e| e.path().is_file()).map(|e| fs::read(e.path()).unwrap()).collect()
    }

    fn holds(haystack: &[u8], needle: &str) -> bool {
        haystack.windows(needle.len()).any(|w| w == needle.as_bytes())
    }

    #[test]
    fn wrong_password_cannot_decrypt() {
        let root = root_for("wrong");
        let p = sealed(&root, "Osobný", "correct horse");
        assert_eq!(p.unlock("incorrect").err(), Some(Denied::Failed("wrong_password")));
        assert!(p.unlock("").is_err());
        let (_, data) = p.unlock("correct horse").map_err(|_| "denied").unwrap();
        assert_eq!(data.bookmarks[0].url, SECRET_URL);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn nothing_readable_on_disk_without_the_password() {
        let root = root_for("plain");
        let p = sealed(&root, "Práca", "correct horse");
        for bytes in files(&p.dir) {
            for needle in ["secret-bookmark", "Hidden title", "secret-tab", "correct horse"] {
                assert!(!holds(&bytes, needle), "plaintext leaked: {needle}");
            }
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn one_profiles_password_does_not_open_another() {
        let root = root_for("cross");
        let a = sealed(&root, "A", "password of a");
        let b = sealed(&root, "B", "password of b");
        assert!(a.unlock("password of b").is_err());
        assert!(b.unlock("password of a").is_err());
        // A vault copied into another profile still needs its own password.
        fs::copy(Vault::file(&a.dir), Vault::file(&b.dir)).unwrap();
        assert!(b.unlock("password of b").is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn same_password_gives_independent_vaults() {
        let root = root_for("salt");
        let (a, b) = (sealed(&root, "A", "same password"), sealed(&root, "B", "same password"));
        let (va, vb) = (fs::read(Vault::file(&a.dir)).unwrap(), fs::read(Vault::file(&b.dir)).unwrap());
        assert_ne!(va[6..22], vb[6..22], "salts must differ");
        assert_ne!(va[22..], vb[22..]);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn tampered_vault_is_refused() {
        let root = root_for("tamper");
        let p = sealed(&root, "A", "correct horse");
        let path = Vault::file(&p.dir);
        let mut bytes = fs::read(&path).unwrap();
        *bytes.last_mut().unwrap() ^= 1;
        fs::write(&path, bytes).unwrap();
        assert!(p.unlock("correct horse").is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn backoff_refuses_even_the_right_password() {
        let root = root_for("backoff");
        let p = sealed(&root, "A", "correct horse");
        for _ in 0..FREE_ATTEMPTS - 1 {
            assert_eq!(p.unlock("nope").err(), Some(Denied::Failed("wrong_password")));
        }
        assert!(matches!(p.unlock("nope"), Err(Denied::Wait(w)) if w > 0));
        assert!(matches!(p.unlock("correct horse"), Err(Denied::Wait(_))));
        // A restart reads the same counter.
        let again = read(&root, &p.id).unwrap();
        assert!(matches!(again.unlock("correct horse"), Err(Denied::Wait(_))));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn backoff_grows_and_is_capped() {
        assert_eq!(delay_after(FREE_ATTEMPTS - 1), 0);
        assert_eq!(delay_after(FREE_ATTEMPTS), 2);
        assert_eq!(delay_after(FREE_ATTEMPTS + 1), 4);
        assert_eq!(delay_after(u32::MAX), MAX_WAIT_SECS);
    }

    #[test]
    fn listing_only_shows_real_profiles() {
        let root = root_for("list");
        sealed(&root, "Osobný", "correct horse");
        fs::create_dir_all(root.join("not-an-id")).unwrap();
        fs::create_dir_all(root.join("aaaaaaaaaaaaaaaa")).unwrap(); // no vault
        let all = list_in(&root);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].name, "Osobný");
        assert!(read(&root, "../../etc").is_none());
        assert_eq!(create_in(&root, "osobný").err(), Some("profile_name_taken"));
        assert_eq!(create_in(&root, "  ").err(), Some("profile_name_invalid"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn legacy_vault_becomes_a_profile() {
        let base = root_for("legacy");
        let root = base.join("profiles");
        let v = Vault::create(&base, "correct horse");
        v.save(&VaultData::default()).unwrap();
        migrate_in(&base, &root, "Osobný").unwrap();
        assert!(!base.join("vault.bin").exists());
        let all = list_in(&root);
        assert_eq!(all.len(), 1);
        assert!(all[0].is_legacy());
        assert!(all[0].unlock("correct horse").is_ok());
        let _ = fs::remove_dir_all(base);
    }
}
