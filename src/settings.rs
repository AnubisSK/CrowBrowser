use serde::Serialize;
use std::{fs, path::PathBuf};

pub const ENGINES: &[(&str, &str, &str)] = &[
    ("ddg", "DuckDuckGo", "https://duckduckgo.com/?q="),
    ("google", "Google", "https://www.google.com/search?q="),
    ("bing", "Bing", "https://www.bing.com/search?q="),
    ("brave", "Brave Search", "https://search.brave.com/search?q="),
    ("startpage", "Startpage", "https://www.startpage.com/do/search?q="),
    ("ecosia", "Ecosia", "https://www.ecosia.org/search?q="),
];

#[derive(Serialize)]
pub struct Settings {
    pub search: String,
    pub homepage: String,
    pub startup: String, // "home" | "last"
    pub theme: String,   // "dark" | "light" | "system"
    pub zoom: f64,
    pub adblock: bool,
    pub unload: u32, // minutes before hidden tabs are unloaded, 0 = never
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
        }
    }
}

pub fn data_dir() -> PathBuf {
    let home = std::env::var_os("HOME").unwrap_or_default();
    PathBuf::from(home).join("Library/Application Support/Crow")
}

fn path() -> PathBuf {
    data_dir().join("settings.conf")
}

impl Settings {
    pub fn load() -> Self {
        let mut s = Self::default();
        if let Ok(text) = fs::read_to_string(path()) {
            for line in text.lines() {
                if let Some((k, v)) = line.split_once('=') {
                    s.set(k, v);
                }
            }
        }
        s
    }

    pub fn save(&self) {
        let _ = fs::create_dir_all(data_dir());
        let _ = fs::write(
            path(),
            format!(
                "search={}\nhomepage={}\nstartup={}\ntheme={}\nzoom={}\nadblock={}\nunload={}\n",
                self.search,
                self.homepage,
                self.startup,
                self.theme,
                self.zoom,
                if self.adblock { "on" } else { "off" },
                self.unload
            ),
        );
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

    pub fn search_prefix(&self) -> &'static str {
        ENGINES
            .iter()
            .find(|e| e.0 == self.search)
            .unwrap_or(&ENGINES[0])
            .2
    }
}
