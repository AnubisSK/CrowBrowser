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

/// Names Windows maps to devices whatever the extension ("CON.txt" opens the console).
const DOS_DEVICES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9",
    "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Characters no Windows file name may contain.
fn is_illegal(c: char) -> bool {
    matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
}

/// Windows ignores spaces before the extension, so "CON .txt" is the device as well.
fn is_device(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or("").trim_end().to_ascii_uppercase();
    DOS_DEVICES.contains(&stem.as_str())
}

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

/// Safe file name: no path parts, drive prefixes or NTFS streams (anything before the last `/`,
/// `\` or `:`), no illegal, control or hidden characters, leading dots or padding, no bare device
/// name ("CON.txt" -> "_CON.txt"), and before an executable extension every dot becomes "_"
/// ("invoice.pdf.tar.exe" -> "invoice_pdf_tar.exe", "a.pdf..exe" -> "a_pdf_.exe"), so the only
/// visible type is the real one.
pub fn sanitize(name: &str) -> String {
    let base = name.rsplit(['/', '\\', ':']).next().unwrap_or("");
    let clean: String = base.chars().filter(|c| !c.is_control() && !is_hidden(*c) && !is_illegal(*c)).collect();
    let clean = clean.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut s = clean.trim_start_matches(['.', ' ']).trim_end_matches(['.', ' ']).to_string();
    if let Some((stem, ext)) = ext_of(&s)
        && executable(&ext)
    {
        let safe_stem = stem.replace('.', "_");
        s = format!("{safe_stem}.{}", &s[stem.len() + 1..]);
    }
    if is_device(&s) {
        s.insert(0, '_');
    }
    let s: String = s.chars().take(200).collect();
    // The cut may have left a trailing dot or space, which Windows silently drops.
    let s = s.trim_end_matches(['.', ' ']);
    if s.is_empty() { "download".into() } else { s.to_string() }
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

/// Mark of the Web (`Zone.Identifier` stream, zone 3 = Internet): SmartScreen and Office check it
/// on open. WebView2 writes it for its own downloads; set explicitly for Crow's (update installer).
#[cfg(target_os = "windows")]
pub fn quarantine(path: &Path) {
    let mut ads = path.as_os_str().to_os_string();
    ads.push(":Zone.Identifier");
    if std::fs::write(&ads, "[ZoneTransfer]\r\nZoneId=3\r\n").is_err() {
        eprintln!("quarantine failed: {}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multi_dot_executable_decoy_is_neutralized() {
        assert_eq!(sanitize("faktura.pdf.tar.exe"), "faktura_pdf_tar.exe");
        assert_eq!(sanitize("a.pdf..exe"), "a_pdf_.exe");
        assert_eq!(risk("faktura_pdf_tar.exe").as_deref(), Some("exe"));
    }

    #[test]
    fn non_executable_keeps_its_dots() {
        assert_eq!(sanitize("archive.tar.gz"), "archive.tar.gz");
        assert_eq!(risk("archive.tar.gz"), None);
    }

    #[test]
    fn hidden_and_rtl_characters_are_removed() {
        assert_eq!(sanitize("invoice\u{202E}gpj.exe"), "invoicegpj.exe");
        assert_eq!(sanitize("a\u{200B}b\u{FEFF}.txt"), "ab.txt");
        assert_eq!(sanitize("bad\u{0}\nname.txt"), "badname.txt");
    }

    #[test]
    fn path_traversal_is_stripped() {
        assert_eq!(sanitize("../../etc/passwd"), "passwd");
        assert_eq!(sanitize("..\\..\\Windows\\evil.bat"), "evil.bat");
        assert_eq!(sanitize("...hidden"), "hidden");
    }

    #[test]
    fn device_names_are_prefixed() {
        assert_eq!(sanitize("CON.txt"), "_CON.txt");
        assert_eq!(sanitize("con.tar.gz"), "_con.tar.gz");
        assert_eq!(sanitize("LPT9"), "_LPT9");
        assert_eq!(sanitize("AUX .txt"), "_AUX .txt");
        assert_eq!(sanitize("CONSOLE.txt"), "CONSOLE.txt");
        assert_eq!(sanitize("COM10.txt"), "COM10.txt");
    }

    #[test]
    fn drive_prefixes_and_streams_are_stripped() {
        assert_eq!(sanitize("C:invoice.pdf"), "invoice.pdf");
        assert_eq!(sanitize("payload.pdf:stream.exe"), "stream.exe");
        assert_eq!(sanitize("C:\\Users\\x\\a.txt"), "a.txt");
    }

    #[test]
    fn illegal_characters_are_removed() {
        assert_eq!(sanitize("a<b>c\"d|e?f*.txt"), "abcdef.txt");
        assert_eq!(sanitize("CON?.txt"), "_CON.txt");
        assert_eq!(sanitize("???"), "download");
    }

    #[test]
    fn empty_name_falls_back_to_download() {
        assert_eq!(sanitize(""), "download");
        assert_eq!(sanitize("../"), "download");
        assert_eq!(sanitize(" . . "), "download");
        assert_eq!(sanitize("\u{202E}"), "download");
    }

    #[test]
    fn long_name_is_truncated() {
        assert_eq!(sanitize(&"a".repeat(500)).chars().count(), 200);
    }
}
