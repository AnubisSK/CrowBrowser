//! Default-browser integration: status, registration with the system, and (Linux) receiving
//! links from other instances. Links themselves arrive as `UserEvent::OpenUrls` (macOS:
//! `Event::Opened`; Linux: command line / the `open-url` action of the running instance).

use crate::app::UserEvent;
use serde::Serialize;
use tao::event_loop::EventLoopProxy;

#[derive(Clone, Copy, Serialize)]
pub struct Status {
    pub is_default: bool,
    /// Registration is possible (macOS: running as the Crow.app bundle; Linux: the .desktop
    /// entry is installed and `xdg-settings` exists).
    pub available: bool,
}

/// Off the main thread (Linux spawns `xdg-*`): optionally registers Crow first, then reports
/// the status and, after a registration, the `locales.json` key of the note to show.
pub fn query(proxy: EventLoopProxy<UserEvent>, set: bool) {
    std::thread::spawn(move || {
        let note = set.then(set_default);
        let _ = proxy.send_event(UserEvent::DefaultBrowser { status: status(), note });
    });
}

#[cfg(target_os = "macos")]
mod mac {
    use std::ffi::{c_char, c_void};

    type CFStringRef = *const c_void;
    const UTF8: u32 = 0x0800_0100;

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringCreateWithBytes(alloc: *const c_void, bytes: *const u8, len: isize, enc: u32, ext: u8) -> CFStringRef;
        fn CFStringGetCString(s: CFStringRef, buf: *mut c_char, size: isize, enc: u32) -> u8;
        fn CFBundleGetMainBundle() -> *const c_void;
        fn CFBundleGetIdentifier(bundle: *const c_void) -> CFStringRef;
        fn CFRelease(obj: *const c_void);
    }

    #[link(name = "CoreServices", kind = "framework")]
    unsafe extern "C" {
        fn LSCopyDefaultHandlerForURLScheme(scheme: CFStringRef) -> CFStringRef;
        fn LSSetDefaultHandlerForURLScheme(scheme: CFStringRef, handler: CFStringRef) -> i32;
    }

    /// Owned CFString, released on drop.
    struct Cf(CFStringRef);

    impl Cf {
        fn new(s: &str) -> Option<Self> {
            let r = unsafe { CFStringCreateWithBytes(std::ptr::null(), s.as_ptr(), s.len() as isize, UTF8, 0) };
            (!r.is_null()).then_some(Self(r))
        }
    }

    impl Drop for Cf {
        fn drop(&mut self) {
            unsafe { CFRelease(self.0) };
        }
    }

    fn to_string(s: CFStringRef) -> Option<String> {
        let mut buf = [0 as c_char; 256];
        let ok = unsafe { CFStringGetCString(s, buf.as_mut_ptr(), buf.len() as isize, UTF8) } != 0;
        ok.then(|| unsafe { std::ffi::CStr::from_ptr(buf.as_ptr()) }.to_string_lossy().into_owned())
    }

    /// Bundle identifier from Info.plist; None when run unbundled (`cargo run`).
    pub fn bundle_id() -> Option<String> {
        let id = unsafe { CFBundleGetIdentifier(CFBundleGetMainBundle()) };
        if id.is_null() { None } else { to_string(id) } // "Get" rule: not owned
    }

    pub fn handler(scheme: &str) -> Option<String> {
        let scheme = Cf::new(scheme)?;
        let h = unsafe { LSCopyDefaultHandlerForURLScheme(scheme.0) };
        if h.is_null() {
            return None;
        }
        let h = Cf(h); // "Copy" rule: owned
        to_string(h.0)
    }

    /// Asks LaunchServices; macOS then shows its own confirmation dialog.
    pub fn set_handler(scheme: &str, id: &str) -> bool {
        let (Some(scheme), Some(id)) = (Cf::new(scheme), Cf::new(id)) else { return false };
        unsafe { LSSetDefaultHandlerForURLScheme(scheme.0, id.0) == 0 }
    }
}

#[cfg(target_os = "macos")]
pub fn status() -> Status {
    let Some(id) = mac::bundle_id() else {
        return Status { is_default: false, available: false };
    };
    // LaunchServices reports bundle IDs lowercased.
    let is_default = ["http", "https"]
        .iter()
        .all(|s| mac::handler(s).is_some_and(|h| h.eq_ignore_ascii_case(&id)));
    Status { is_default, available: true }
}

#[cfg(target_os = "macos")]
fn set_default() -> &'static str {
    let Some(id) = mac::bundle_id() else { return "default_browser_unavailable" };
    if !["http", "https"].iter().all(|s| mac::set_handler(s, &id)) {
        return "default_browser_failed";
    }
    // Takes effect only once the user confirms the system dialog (re-checked on focus).
    if status().is_default { "default_browser_set" } else { "default_browser_confirm" }
}

/// Desktop entry written by scripts/install_linux.sh (also the GTK application ID).
#[cfg(target_os = "linux")]
const DESKTOP: &str = "com.crow.browser.desktop";

#[cfg(target_os = "linux")]
fn installed() -> bool {
    use std::path::PathBuf;
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home.join(".local/share"));
    let dirs = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    std::iter::once(data_home)
        .chain(dirs.split(':').filter(|d| d.starts_with('/')).map(PathBuf::from))
        .any(|d| d.join("applications").join(DESKTOP).is_file())
}

/// Runs an `xdg-*` tool with fixed arguments (no shell); stdout on success.
#[cfg(target_os = "linux")]
fn xdg(cmd: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(cmd)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[cfg(target_os = "linux")]
pub fn status() -> Status {
    if !installed() {
        return Status { is_default: false, available: false };
    }
    match xdg("xdg-settings", &["get", "default-web-browser"]) {
        Some(current) => Status { is_default: current == DESKTOP, available: true },
        None => Status { is_default: false, available: false },
    }
}

#[cfg(target_os = "linux")]
fn set_default() -> &'static str {
    if !installed() {
        return "default_browser_unavailable";
    }
    let set = xdg("xdg-settings", &["set", "default-web-browser", DESKTOP]).is_some();
    let mime = xdg("xdg-mime", &["default", DESKTOP, "x-scheme-handler/http", "x-scheme-handler/https"]).is_some();
    if set && mime && status().is_default { "default_browser_set" } else { "default_browser_failed" }
}

/// Primary instance: links from later launches (`crow-browser <url>`) arrive through the
/// exported GApplication action `open-url`.
#[cfg(target_os = "linux")]
pub fn listen(app: &gtk::Application, proxy: EventLoopProxy<UserEvent>) {
    use gtk::{gio, glib, prelude::*};
    let action = gio::SimpleAction::new("open-url", Some(glib::VariantTy::STRING));
    action.connect_activate(move |_, v| {
        if let Some(url) = v.and_then(|v| v.str()) {
            let _ = proxy.send_event(UserEvent::OpenUrls(vec![url.to_string()]));
        }
    });
    app.add_action(&action);
}

/// Secondary instance (GTK keeps one per app ID): hands `urls` to the running Crow over D-Bus.
/// True = done, this process should exit without opening a window.
#[cfg(target_os = "linux")]
pub fn forward_to_primary(app: &gtk::Application, urls: &[String]) -> bool {
    use gtk::{gio, prelude::*};
    if !app.is_remote() || urls.is_empty() {
        return false;
    }
    for u in urls {
        app.activate_action("open-url", Some(&u.to_variant()));
    }
    if let Some(c) = app.dbus_connection() {
        let _ = c.flush_sync(gio::Cancellable::NONE);
    }
    true
}
