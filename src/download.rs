//! Downloaded files: the name is sanitized before anything is written (no hidden or decoy
//! extensions), executable types are flagged in the prompt, and on macOS the finished file gets
//! the quarantine attribute so Gatekeeper inspects it when it is opened.

use std::path::{Path, PathBuf};

/// Types that run code when opened (Windows, macOS, Linux, scripts, installers).
const EXECUTABLE: &[&str] = &[
    "exe", "msi", "bat", "cmd", "com", "scr", "pif", "cpl", "vbs", "vbe", "js", "jse", "wsf", "wsh", "ps1",
    "hta", "lnk", "reg", "jar", "app", "command", "tool", "terminal", "workflow", "scpt", "pkg", "mpkg",
    "dmg", "sh", "bash", "zsh", "csh", "ksh", "fish", "run", "bin", "out", "appimage", "desktop", "deb",
    "rpm", "apk", "flatpakref", "snap", "py", "pl", "rb", "php",
];

/// Invisible / direction-changing characters (U+202E can make "gpj.exe" look like "exe.jpg").
fn is_hidden(c: char) -> bool {
    matches!(c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{061C}' | '\u{FEFF}')
}

fn ext_of(name: &str) -> Option<(&str, String)> {
    let (stem, ext) = name.rsplit_once('.')?;
    (!stem.is_empty()).then(|| (stem, ext.to_ascii_lowercase()))
}

fn executable(ext: &str) -> bool {
    EXECUTABLE.contains(&ext)
}

/// Safe file name: no path parts, control or hidden characters, leading dots or padding, and
/// before an executable extension every dot becomes "_" ("invoice.pdf.tar.exe" ->
/// "invoice_pdf_tar.exe", "a.pdf..exe" -> "a_pdf__.exe"), so the only visible type is the real one.
pub fn sanitize(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("");
    let clean: String = base.chars().filter(|c| !c.is_control() && !is_hidden(*c)).collect();
    let clean = clean.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut s = clean.trim_start_matches(['.', ' ']).trim_end_matches(['.', ' ']).to_string();
    if let Some((stem, ext)) = ext_of(&s)
        && executable(&ext)
    {
        let safe_stem = stem.replace('.', "_");
        s = format!("{safe_stem}.{}", &s[stem.len() + 1..]);
    }
    let s: String = s.chars().take(200).collect();
    if s.is_empty() { "download".into() } else { s }
}

/// Extension of an executable/script type (lowercase), for the warning in the download prompt
/// (worded by the UI in its language).
pub fn risk(name: &str) -> Option<String> {
    let (_, ext) = ext_of(name)?;
    executable(&ext).then_some(ext)
}

/// Free path for `name` (sanitized) in `dir`: "file (1).ext", "file (2).ext", …
pub fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let name = sanitize(name);
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.clone(), String::new()),
    };
    let mut p = dir.join(&name);
    let mut n = 1;
    while p.exists() {
        p = dir.join(format!("{stem} ({n}){ext}"));
        n += 1;
    }
    p
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn setxattr(
        path: *const std::ffi::c_char,
        name: *const std::ffi::c_char,
        value: *const std::ffi::c_void,
        size: usize,
        position: u32,
        options: std::ffi::c_int,
    ) -> std::ffi::c_int;
}

/// Marks a finished download as coming from the internet (`com.apple.quarantine`, same value
/// `xattr -w com.apple.quarantine "0081;<time>;Crow;"` writes): Gatekeeper checks it on open.
#[cfg(target_os = "macos")]
pub fn quarantine(path: &Path) {
    use std::os::unix::ffi::OsStrExt;
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let value = format!("0081;{secs:08x};Crow;");
    let Ok(p) = std::ffi::CString::new(path.as_os_str().as_bytes()) else { return };
    let rc = unsafe { setxattr(p.as_ptr(), c"com.apple.quarantine".as_ptr(), value.as_ptr().cast(), value.len(), 0, 0) };
    if rc != 0 {
        eprintln!("quarantine failed: {}", path.display());
    }
}

/// Linux has no quarantine attribute; downloads are written without the executable bit.
#[cfg(target_os = "linux")]
pub fn quarantine(_path: &Path) {}
