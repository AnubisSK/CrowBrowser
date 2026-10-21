//! Update check against GitHub Releases. Network goes through the system `curl` (no HTTP stack
//! in the binary). The installer is only downloaded after the user clicks, verified against the
//! release's SHA-256 digest and revealed in the file manager, never executed.

use crate::app::UserEvent;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::Command,
};
use tao::event_loop::EventLoopProxy;

const REPO: &str = "AnubisSK/CrowBrowser";
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");
const MAX_NOTES: usize = 30;

#[derive(Serialize, Clone)]
pub struct Info {
    pub version: String,
    pub page: String,
    pub notes: Vec<Note>,
    pub asset: Option<Asset>,
}

#[derive(Serialize, Clone)]
pub struct Note {
    pub sha: String,
    pub msg: String,
    pub url: String,
}

#[derive(Serialize, Clone)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
    #[serde(skip)]
    pub sha256: Option<String>,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<RelAsset>,
}

#[derive(Deserialize)]
struct RelAsset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    digest: Option<String>,
}

#[derive(Deserialize)]
struct Commits {
    #[serde(default)]
    commits: Vec<CommitItem>,
}

#[derive(Deserialize)]
struct CommitItem {
    sha: String,
    html_url: String,
    commit: CommitBody,
}

#[derive(Deserialize)]
struct CommitBody {
    message: String,
}

/// curl that ignores `~/.curlrc` (`-q` must be first), refuses anything but HTTPS (also on
/// redirects) and bypasses proxy settings from the environment.
pub fn curl() -> Command {
    let mut c = Command::new("curl");
    c.arg("-q");
    c.args(["-fsSL", "--proto", "=https", "--proto-redir", "=https", "--noproxy", "*", "--connect-timeout", "10"]);
    c.arg("-A").arg(format!("Crow/{CURRENT}"));
    c
}

fn api<T: for<'de> Deserialize<'de>>(path: &str) -> Result<T, String> {
    let out = curl()
        .args(["--max-time", "20", "-H", "Accept: application/vnd.github+json"])
        .arg(format!("https://api.github.com/repos/{REPO}/{path}"))
        .output()
        .map_err(|_| "curl nie je dostupný".to_string())?;
    if !out.status.success() {
        return Err("na GitHube nie je vydanie alebo neodpovedá".into());
    }
    serde_json::from_slice(&out.stdout).map_err(|_| "Neplatná odpoveď GitHubu".into())
}

/// "v1.2.3-beta" → (1, 2, 3)
fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let v = v.trim().trim_start_matches(['v', 'V']);
    let v = v.split(['-', '+']).next()?;
    let mut it = v.split('.').map(|p| p.parse::<u64>().ok());
    Some((it.next()??, it.next().flatten().unwrap_or(0), it.next().flatten().unwrap_or(0)))
}

/// Installer for this OS/arch, only from this repo's release downloads.
fn pick_asset(assets: Vec<RelAsset>) -> Option<Asset> {
    let prefix = format!("https://github.com/{REPO}/releases/download/");
    #[cfg(target_os = "macos")]
    let exts: Vec<&str> = vec![".dmg"];
    #[cfg(target_os = "linux")]
    let exts: Vec<&str> = {
        let has = |p: &str| Path::new(p).exists();
        let mut e = Vec::new();
        if has("/usr/bin/pacman") {
            e.push(".pkg.tar.zst");
        }
        if has("/usr/bin/dpkg") {
            e.push(".deb");
        }
        if has("/usr/bin/rpm") {
            e.push(".rpm");
        }
        e.extend([".AppImage", ".tar.gz"]);
        e
    };
    let arch = std::env::consts::ARCH;
    let other_arch = |n: &str| {
        let n = n.to_ascii_lowercase();
        let arm = n.contains("arm64") || n.contains("aarch64");
        let x86 = n.contains("x86_64") || n.contains("amd64") || n.contains("intel");
        (arm && arch != "aarch64") || (x86 && arch != "x86_64")
    };
    let ok = |a: &RelAsset| {
        a.browser_download_url.starts_with(&prefix)
            && !a.name.starts_with('.')
            && Path::new(&a.name).file_name().is_some_and(|f| f == a.name.as_str())
            && !other_arch(&a.name)
    };
    let a = exts.iter().find_map(|ext| assets.iter().find(|a| ok(a) && a.name.ends_with(ext)))?;
    let sha256 = a
        .digest
        .as_deref()
        .and_then(|d| d.strip_prefix("sha256:"))
        .filter(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()))
        .map(|h| h.to_ascii_lowercase());
    Some(Asset { name: a.name.clone(), url: a.browser_download_url.clone(), size: a.size, sha256 })
}

/// Commit messages between the installed version's tag and the new release.
fn notes(tag: &str) -> Vec<Note> {
    let base = format!("{}{CURRENT}", if tag.starts_with('v') { "v" } else { "" });
    let commits = api::<Commits>(&format!("compare/{base}...{tag}"))
        .map(|c| c.commits.into_iter().rev().collect())
        .or_else(|_| api::<Vec<CommitItem>>(&format!("commits?sha={tag}&per_page={MAX_NOTES}")))
        .unwrap_or_default();
    commits
        .into_iter()
        .filter_map(|c| {
            let msg = c.commit.message.lines().next().unwrap_or("").trim().to_string();
            (!msg.is_empty() && !msg.starts_with("Merge ")).then(|| Note {
                sha: c.sha.chars().take(7).collect(),
                msg: msg.chars().take(200).collect(),
                url: c.html_url,
            })
        })
        .take(MAX_NOTES)
        .collect()
}

fn latest() -> Result<Option<Info>, String> {
    let r: Release = api("releases/latest")?;
    if r.draft || r.prerelease {
        return Ok(None);
    }
    let (Some(new), Some(cur)) = (parse_version(&r.tag_name), parse_version(CURRENT)) else {
        return Err("Neznámy formát verzie".into());
    };
    if new <= cur {
        return Ok(None);
    }
    Ok(Some(Info {
        version: r.tag_name.trim_start_matches(['v', 'V']).to_string(),
        notes: notes(&r.tag_name),
        page: r.html_url,
        asset: pick_asset(r.assets),
    }))
}

/// Background check; answers with `UserEvent::Update`.
pub fn check(proxy: EventLoopProxy<UserEvent>, manual: bool) {
    std::thread::spawn(move || {
        let _ = proxy.send_event(UserEvent::Update { result: latest(), manual });
    });
}

#[cfg(target_os = "macos")]
fn download_dir() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join("Downloads")
}

#[cfg(target_os = "linux")]
fn download_dir() -> PathBuf {
    crate::wk::download_dir()
}

/// Creates (or truncates) `path` readable only by the user; curl then writes into it and keeps
/// the mode.
pub fn private_file(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)?;
    // An existing file keeps its old mode: enforce it.
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}

fn sha256_of(path: &Path) -> Option<String> {
    let mut f = fs::File::open(path).ok()?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Some(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Downloads the installer (user already confirmed), verifies its digest, then reveals it.
/// Progress goes through the regular download events so it shows in the Downloads panel.
pub fn download(proxy: EventLoopProxy<UserEvent>, asset: Asset) {
    std::thread::spawn(move || {
        let dir = download_dir();
        let _ = fs::create_dir_all(&dir);
        let path = crate::download::unique_path(&dir, &asset.name);
        let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else { return };
        let part = path.with_file_name(format!("{name}.part"));
        let send = |e| {
            let _ = proxy.send_event(e);
        };
        send(UserEvent::DownloadStarted { url: asset.url.clone(), path: path.clone() });
        let fetched = private_file(&part).ok().and_then(|_| {
            curl()
                .args(["--max-time", "1800", "--max-filesize"])
                .arg((asset.size.max(1) + 1024 * 1024).to_string())
                .arg("-o")
                .arg(&part)
                .arg(&asset.url)
                .status()
                .ok()
        });
        let ok = fetched.is_some_and(|s| s.success())
            && asset.sha256.as_deref().is_some_and(|want| sha256_of(&part).as_deref() == Some(want))
            && fs::rename(&part, &path).is_ok();
        if !ok {
            let _ = fs::remove_file(&part);
        }
        send(UserEvent::DownloadDone { url: asset.url.clone(), ok });
        send(UserEvent::UpdateSaved(ok.then_some(path)));
    });
}
