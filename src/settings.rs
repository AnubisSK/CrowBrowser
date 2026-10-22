use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf, sync::OnceLock};

pub const ENGINES: &[(&str, &str, &str)] = &[
    ("ddg", "DuckDuckGo", "https://duckduckgo.com/?q="),
    ("google", "Google", "https://www.google.com/search?q="),
    ("bing", "Bing", "https://www.bing.com/search?q="),
    ("brave", "Brave Search", "https://search.brave.com/search?q="),
    ("startpage", "Startpage", "https://www.startpage.com/do/search?q="),
    ("ecosia", "Ecosia", "https://www.ecosia.org/search?q="),
];

/// Stored encrypted inside the vault (see `VaultData::settings`).
#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Settings {
    pub search: String,
    pub homepage: String,
    pub startup: String, // "home" | "last"
    pub theme: String,   // "dark" | "light" | "system"
    pub zoom: f64,
    pub adblock: bool,
    pub unload: u32, // minutes before hidden tabs are unloaded, 0 = never
    pub updates: bool, // check GitHub Releases after unlocking
    /// Sites (and their subdomains) where the ad blocker is off. Lives in the encrypted vault;
    /// applied per tab, so it never ends up in WebKit's on-disk rule store.
    pub adblock_allow: Vec<String>,
    /// Download and use EasyList + EasyPrivacy.
    pub adblock_easylist: bool,
    /// Download and use the StevenBlack hosts list.
    pub adblock_hosts: bool,
    /// UI language (`LANGUAGES`).
    pub language: String,
}

/// UI languages with dictionaries in `locales.json`.
pub const LANGUAGES: &[&str] = &["sk", "cs", "en", "de"];

impl Default for Settings {
    fn default() -> Self {
        Self {
            search: "ddg".into(),
            homepage: "https://duckduckgo.com".into(),
            startup: "last".into(),
            theme: "dark".into(),
            zoom: 1.0,
            adblock: true,
            unload: 5,
            updates: true,
            adblock_allow: Vec::new(),
            adblock_easylist: true,
            adblock_hosts: true,
            language: "sk".into(),
        }
    }
}

#[cfg(all(target_os = "macos", not(test)))]
pub fn data_dir() -> PathBuf {
    let home = std::env::var_os("HOME").unwrap_or_default();
    PathBuf::from(home).join("Library/Application Support/Crow")
}

/// `$XDG_DATA_HOME/crow`, default `~/.local/share/crow`.
#[cfg(all(target_os = "linux", not(test)))]
pub fn data_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share"))
        .join("crow")
}

/// Tests never touch the real profile: a per-process temp dir instead.
#[cfg(test)]
pub fn data_dir() -> PathBuf {
    std::env::temp_dir().join(format!("crow-test-{}", std::process::id()))
}

/// The lock screen is shown before the vault (and its settings) is decrypted, so the chosen
/// language is also kept in plain `data_dir()/language`: just the code, nothing else.
pub fn stored_language() -> String {
    let v = fs::read_to_string(data_dir().join("language")).unwrap_or_default();
    let v = v.trim();
    if LANGUAGES.contains(&v) { v.into() } else { "sk".into() }
}

/// UI dictionaries shared with `i18n.js`, parsed once on first use.
fn locales() -> &'static serde_json::Value {
    static L: OnceLock<serde_json::Value> = OnceLock::new();
    L.get_or_init(|| serde_json::from_str(include_str!("locales.json")).unwrap_or_default())
}

/// Backend text in `lang`, like `t()` in i18n.js: English fallback, then the key itself;
/// `{0}`, `{1}` … are replaced by `args`.
pub fn tr(lang: &str, key: &str, args: &[&str]) -> String {
    let l = locales();
    let s = [lang, "en"].iter().find_map(|c| l[*c][key].as_str()).unwrap_or(key);
    args.iter()
        .enumerate()
        .fold(s.to_string(), |s, (i, a)| s.replace(&format!("{{{i}}}"), a))
}

pub fn store_language(code: &str) {
    if LANGUAGES.contains(&code) && stored_language() != code {
        let _ = fs::create_dir_all(data_dir());
        let _ = fs::write(data_dir().join("language"), code);
    }
}

impl Settings {
    /// Migrates the old plaintext settings.conf (if any) and deletes it.
    pub fn take_legacy() -> Option<Self> {
        let path = data_dir().join("settings.conf");
        let text = fs::read_to_string(&path).ok()?;
        let mut s = Self::default();
        for line in text.lines() {
            if let Some((k, v)) = line.split_once('=') {
                s.set(k, v);
            }
        }
        let _ = fs::remove_file(path);
        Some(s)
    }

    /// Returns true if the key was recognized and the value valid.
    pub fn set(&mut self, key: &str, value: &str) -> bool {
        let v = value.trim();
        if v.contains(['\n', '\r']) {
            return false;
        }
        match key {
            "search" if ENGINES.iter().any(|e| e.0 == v) => self.search = v.into(),
            "homepage" if !v.is_empty() => self.homepage = v.into(),
            "startup" if matches!(v, "home" | "last") => self.startup = v.into(),
            "theme" if matches!(v, "dark" | "light" | "system") => self.theme = v.into(),
            "adblock" if matches!(v, "on" | "off") => self.adblock = v == "on",
            "updates" if matches!(v, "on" | "off") => self.updates = v == "on",
            "adblock_easylist" if matches!(v, "on" | "off") => self.adblock_easylist = v == "on",
            "adblock_hosts" if matches!(v, "on" | "off") => self.adblock_hosts = v == "on",
            "language" if LANGUAGES.contains(&v) => self.language = v.into(),
            "zoom" => match v.parse::<f64>() {
                Ok(z) if (0.25..=3.0).contains(&z) => self.zoom = z,
                _ => return false,
            },
            "unload" => match v.parse::<u32>() {
                Ok(m) if m <= 240 => self.unload = m,
                _ => return false,
            },
            _ => return false,
        }
        true
    }

    /// `host` is on the ad-block allowlist (exact or subdomain).
    pub fn allowed(&self, host: &str) -> bool {
        !host.is_empty() && self.adblock_allow.iter().any(|d| host == d || host.ends_with(&format!(".{d}")))
    }

    pub fn search_prefix(&self) -> &'static str {
        ENGINES
            .iter()
            .find(|e| e.0 == self.search)
            .unwrap_or(&ENGINES[0])
            .2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_accepts_valid_values() {
        let mut s = Settings::default();
        assert!(s.set("search", "google"));
        assert_eq!(s.search_prefix(), "https://www.google.com/search?q=");
        assert!(s.set("adblock", "off"));
        assert!(!s.adblock);
        assert!(s.set("zoom", " 1.5 "));
        assert!((s.zoom - 1.5).abs() < f64::EPSILON);
        assert!(s.set("language", "de"));
    }

    #[test]
    fn set_rejects_invalid_values() {
        let mut s = Settings::default();
        assert!(!s.set("search", "evil"));
        assert!(!s.set("zoom", "10"));
        assert!(!s.set("unload", "-1"));
        assert!(!s.set("language", "fr"));
        assert!(!s.set("homepage", "a\nb"));
        assert!(!s.set("unknown", "x"));
        assert_eq!(s.search, "ddg");
        assert_eq!(s.language, "sk");
    }

    #[test]
    fn allowlist_matches_host_and_subdomains_only() {
        let s = Settings { adblock_allow: vec!["example.com".into()], ..Settings::default() };
        assert!(s.allowed("example.com"));
        assert!(s.allowed("www.example.com"));
        assert!(!s.allowed("badexample.com"));
        assert!(!s.allowed(""));
    }

    #[test]
    fn tr_falls_back_to_english_then_key() {
        assert_eq!(tr("en", "up_to_date", &["1.0"]), "CrowBrowser 1.0 is the latest version");
        assert_eq!(tr("xx", "up_to_date", &["1.0"]), "CrowBrowser 1.0 is the latest version");
        assert_eq!(tr("sk", "no_such_key", &[]), "no_such_key");
    }
}
