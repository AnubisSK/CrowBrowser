//! Volatile session storage: browser profiles live on tmpfs and are overwritten and removed when
//! the session ends.

use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

const PREFIX: &str = "crow-rbi-";

#[cfg(target_os = "linux")]
fn volatile(path: &Path) -> bool {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    const TMPFS: i64 = 0x0102_1994;
    const RAMFS: i64 = 0x8584_58f6;
    let Ok(c) = CString::new(path.as_os_str().as_bytes()) else { return false };
    let mut st: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(c.as_ptr(), &raw mut st) } != 0 {
        return false;
    }
    #[allow(clippy::useless_conversion, clippy::unnecessary_cast)]
    let kind = st.f_type as i64;
    kind == TMPFS || kind == RAMFS
}

#[cfg(target_os = "linux")]
fn roots() -> Vec<PathBuf> {
    let mut v = vec![PathBuf::from("/dev/shm")];
    v.extend(std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from));
    v.retain(|p| volatile(p));
    v
}

#[cfg(not(target_os = "linux"))]
fn roots() -> Vec<PathBuf> {
    Vec::new()
}

#[cfg(target_os = "linux")]
fn swap_active() -> bool {
    fs::read_to_string("/proc/swaps").map_or(true, |s| s.lines().count() > 1)
}

#[cfg(not(target_os = "linux"))]
fn swap_active() -> bool {
    true
}

/// Why `--ram-only` cannot be honoured for browser profiles on this machine.
pub fn ram_only_problem() -> Option<&'static str> {
    if roots().is_empty() {
        Some("no tmpfs mount available (/dev/shm, $XDG_RUNTIME_DIR)")
    } else if swap_active() {
        Some("swap is active, tmpfs pages could reach the disk (swapoff -a)")
    } else {
        None
    }
}

/// `--ram-only` process hardening: no core dumps, no ptrace by other users, memory locked when
/// the limit allows it (otherwise locking would make later allocations fail).
#[cfg(unix)]
pub fn harden() {
    unsafe {
        let none = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
        libc::setrlimit(libc::RLIMIT_CORE, &raw const none);
        #[cfg(target_os = "linux")]
        libc::prctl(libc::PR_SET_DUMPABLE, 0);
        let mut lim: libc::rlimit = std::mem::zeroed();
        if libc::getrlimit(libc::RLIMIT_MEMLOCK, &raw mut lim) == 0
            && lim.rlim_cur == libc::RLIM_INFINITY
            && libc::mlockall(libc::MCL_CURRENT | libc::MCL_FUTURE) != 0
        {
            eprintln!("warning: mlockall failed");
        }
    }
}

#[cfg(not(unix))]
pub fn harden() {}

fn unique_name() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    format!("{PREFIX}{:x}-{:x}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed))
}

fn make_private(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new().mode(0o700).create(path)
    }
    #[cfg(not(unix))]
    {
        fs::create_dir(path)
    }
}

fn zero_file(path: &Path) -> io::Result<()> {
    use std::io::Write;
    let len = fs::symlink_metadata(path)?.len();
    let mut f = fs::OpenOptions::new().write(true).open(path)?;
    let zeros = [0u8; 64 * 1024];
    let mut left = len;
    while left > 0 {
        let n = usize::try_from(left.min(zeros.len() as u64)).unwrap_or(zeros.len());
        f.write_all(&zeros[..n])?;
        left -= n as u64;
    }
    f.sync_all()
}

/// Zeros every regular file below `dir`. Links are not followed.
fn scrub(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let Ok(t) = e.file_type() else { continue };
        if t.is_dir() {
            scrub(&e.path());
        } else if t.is_file() {
            let _ = zero_file(&e.path());
        }
    }
}

fn wipe(dir: &Path) {
    // Windows keeps files locked for a moment after the browser's processes are gone.
    for _ in 0..20 {
        scrub(dir);
        if fs::remove_dir_all(dir).is_ok() || !dir.exists() {
            return;
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    eprintln!("warning: a session directory could not be removed");
}

/// Directories left by a run that was killed before it could clean up.
pub fn sweep_stale() {
    let mut places = roots();
    places.push(std::env::temp_dir());
    for root in places {
        let Ok(entries) = fs::read_dir(&root) else { continue };
        for e in entries.flatten() {
            if e.file_name().to_string_lossy().starts_with(PREFIX) && e.file_type().is_ok_and(|t| t.is_dir()) {
                wipe(&e.path());
            }
        }
    }
}

/// A session's directory; wiped on drop.
pub struct RamDir(PathBuf);

impl RamDir {
    /// On tmpfs when there is one. Without it only a run without `--ram-only` falls back to the
    /// temp directory.
    pub fn create(ram_only: bool) -> Result<Self, &'static str> {
        if ram_only && ram_only_problem().is_some() {
            return Err("remote_err_ram");
        }
        let root = roots().into_iter().next().unwrap_or_else(std::env::temp_dir);
        let path = root.join(unique_name());
        make_private(&path).map_err(|_| "remote_err_rbi")?;
        Ok(Self(path))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for RamDir {
    fn drop(&mut self) {
        wipe(&self.0);
    }
}
