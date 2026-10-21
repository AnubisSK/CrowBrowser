use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

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
    /// Use the downloaded StevenBlack hosts list in addition to the built-in one.
    pub adblock_hosts: bool,
}

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
            adblock_hosts: true,
        }
    }
}

#[cfg(target_os = "macos")]
pub fn data_dir() -> PathBuf {
    let home = std::env::var_os("HOME").unwrap_or_default();
    PathBuf::from(home).join("Library/Application Support/Crow")
}

/// `$XDG_DATA_HOME/crow`, default `~/.local/share/crow`.
#[cfg(target_os = "linux")]
pub fn data_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share"))
        .join("crow")
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
            "adblock_hosts" if matches!(v, "on" | "off") => self.adblock_hosts = v == "on",
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
