//! WebKitGTK glue (Linux), same API as `wk.rs`: shared ephemeral context, cookie export/import,
//! content filters (same JSON rule format as Safari), downloads, window chrome and shortcuts.

use crate::vault::CookieRec;
use gtk::{
    gdk, gio,
    glib::{self, translate::FromGlibPtrContainer},
    prelude::*,
};
use std::{
    cell::{Cell, RefCell},
    ffi::{CString, c_char, c_void},
    path::{Path, PathBuf},
    ptr,
    rc::Rc,
};
use tao::{
    platform::unix::WindowExtUnix,
    window::{Theme, Window},
};
use webkit2gtk::{
    CacheModel, CookieManagerExt, DownloadExt, PolicyDecisionExt, PolicyDecisionType,
    ResponsePolicyDecisionExt, URIRequestExt, URIResponseExt, UserContentManagerExt, WebContext,
    WebContextExt, WebViewExt, WebsiteDataManagerExt, WebsiteDataTypes,
};
use wry::{
    NewWindowFeatures, NewWindowResponse, Rect, WebView, WebViewBuilder, WebViewBuilderExtUnix,
    WebViewExtUnix,
};

/// The shared ephemeral WebKitWebContext: nothing written to disk by WebKit.
pub type Store = WebContext;
/// Tabs are created related to an existing view, which puts them in the shared context.
pub struct Config(webkit2gtk::WebView);

#[allow(non_camel_case_types)]
type GAsyncReadyCallback =
    Option<unsafe extern "C" fn(*mut glib::gobject_ffi::GObject, *mut gio::ffi::GAsyncResult, *mut c_void)>;

// Not covered by the webkit2gtk crate (UserContentFilter is "ignored", get_all_cookies is 2.42+,
// WebsiteDataManager::clear is not generated).
#[link(name = "webkit2gtk-4.1")]
unsafe extern "C" {
    fn webkit_user_content_filter_store_new(path: *const c_char) -> *mut c_void;
    fn webkit_user_content_filter_store_save(
        store: *mut c_void,
        id: *const c_char,
        source: *mut glib::ffi::GBytes,
        cancellable: *mut gio::ffi::GCancellable,
        callback: GAsyncReadyCallback,
        data: *mut c_void,
    );
    fn webkit_user_content_filter_store_save_finish(
        store: *mut c_void,
        res: *mut gio::ffi::GAsyncResult,
        err: *mut *mut glib::ffi::GError,
    ) -> *mut c_void;
    fn webkit_user_content_filter_unref(filter: *mut c_void);
    fn webkit_user_content_manager_add_filter(manager: *mut c_void, filter: *mut c_void);
    fn webkit_cookie_manager_get_all_cookies(
        manager: *mut c_void,
        cancellable: *mut gio::ffi::GCancellable,
        callback: GAsyncReadyCallback,
        data: *mut c_void,
    );
    fn webkit_cookie_manager_get_all_cookies_finish(
        manager: *mut c_void,
        res: *mut gio::ffi::GAsyncResult,
        err: *mut *mut glib::ffi::GError,
    ) -> *mut glib::ffi::GList;
    fn webkit_website_data_manager_clear(
        manager: *mut c_void,
        types: u32,
        timespan: i64,
        cancellable: *mut gio::ffi::GCancellable,
        callback: GAsyncReadyCallback,
        data: *mut c_void,
    );
}

type Started = dyn Fn(String, PathBuf);

struct Filter(*mut c_void);

impl Drop for Filter {
    fn drop(&mut self) {
        unsafe { webkit_user_content_filter_unref(self.0) }
    }
}

thread_local! {
    static RULES: RefCell<Option<Filter>> = const { RefCell::new(None) };
    static ROOT: RefCell<Option<webkit2gtk::WebView>> = const { RefCell::new(None) };
    static UI_ROOT: RefCell<Option<webkit2gtk::WebView>> = const { RefCell::new(None) };
    static LAYOUT: RefCell<Option<gtk::Layout>> = const { RefCell::new(None) };
    static THEME: RefCell<Option<(Option<String>, bool)>> = const { RefCell::new(None) };
    static DOWNLOADS: Cell<bool> = const { Cell::new(false) };
    /// Downloads waiting for the user's answer (decide-destination is answered asynchronously).
    static ASKING: RefCell<Vec<(u32, webkit2gtk::Download, String)>> = const { RefCell::new(Vec::new()) };
    static STARTED: RefCell<Option<Rc<Started>>> = const { RefCell::new(None) };
}

/// Hidden, never-loaded view owning an ephemeral context; views created related to it share
/// that context. Web content uses `ROOT`, the browser UI its own `UI_ROOT` (no shared state).
fn root_of(key: &'static std::thread::LocalKey<RefCell<Option<webkit2gtk::WebView>>>) -> webkit2gtk::WebView {
    key.with(|r| {
        r.borrow_mut()
            .get_or_insert_with(|| {
                let ctx = WebContext::new_ephemeral();
                ctx.set_cache_model(CacheModel::DocumentViewer);
                webkit2gtk::WebView::with_context(&ctx)
            })
            .clone()
    })
}

fn root() -> webkit2gtk::WebView {
    root_of(&ROOT)
}

pub fn new_store() -> Store {
    root().context().expect("webkit context")
}

pub fn config(_store: &Store, _adblock: bool) -> Config {
    Config(root())
}

/// window.open target: must be related to the opener so `window.opener` keeps working.
pub fn popup_config(features: NewWindowFeatures) -> Config {
    Config(features.opener.webview)
}

/// `incognito` only stops wry from creating a throwaway *persistent* context (which would write
/// HSTS/storage files to disk); the related view decides the context actually used.
pub fn builder<'a>(config: Config) -> WebViewBuilder<'a> {
    WebViewBuilder::new().with_incognito(true).with_related_view(config.0)
}

pub fn ui_builder<'a>() -> WebViewBuilder<'a> {
    WebViewBuilder::new().with_incognito(true).with_related_view(root_of(&UI_ROOT))
}

/// Views live in a GtkLayout (requests no size, so the window stays freely resizable);
/// position and size are managed by `set_bounds`.
pub fn attach(b: WebViewBuilder, rect: Rect, _window: &Window) -> wry::Result<WebView> {
    let layout = LAYOUT.with(|l| l.borrow().clone()).expect("init_window not called");
    let v = b.build_gtk(&layout)?;
    let w = v.webview();
    if let Some(ctx) = w.context() {
        ctx.set_cache_model(CacheModel::DocumentViewer);
    }
    w.connect_decide_policy(start_downloads);
    set_bounds(&v, rect);
    Ok(v)
}

/// WebKitGTK does not emit `WebContext::download-started` for responses it turns into
/// downloads on its own (unsupported MIME type, `Content-Disposition: attachment`), so those
/// would vanish. Start them explicitly; the shared handler (`watch_downloads`) then asks the user.
fn start_downloads(w: &webkit2gtk::WebView, d: &webkit2gtk::PolicyDecision, t: PolicyDecisionType) -> bool {
    if t != PolicyDecisionType::Response {
        return false;
    }
    let Some(r) = d.downcast_ref::<webkit2gtk::ResponsePolicyDecision>() else { return false };
    let attachment = r
        .response()
        .and_then(|res| res.http_headers())
        .and_then(|h| h.one("Content-Disposition"))
        .is_some_and(|v| v.trim_start().to_ascii_lowercase().starts_with("attachment"));
    if r.is_mime_type_supported() && !attachment {
        return false;
    }
    let Some(uri) = r.request().and_then(|q| q.uri()) else { return false };
    d.ignore();
    w.download_uri(&uri);
    true
}

pub fn set_bounds(v: &WebView, rect: Rect) {
    let w = v.webview();
    let scale = w.scale_factor() as f64;
    let (x, y): (i32, i32) = rect.position.to_logical::<i32>(scale).into();
    let (width, height): (i32, i32) = rect.size.to_logical::<i32>(scale).into();
    if let Some(layout) = w.parent().and_then(|p| p.downcast::<gtk::Layout>().ok()) {
        layout.move_(&w, x, y);
    }
    w.set_size_request(width.max(1), height.max(1));
}

pub fn popup_response(v: &WebView) -> NewWindowResponse {
    NewWindowResponse::Create { webview: v.webview() }
}

pub fn set_rules(v: &WebView, on: bool) {
    let Some(manager) = v.webview().user_content_manager() else { return };
    manager.remove_all_filters();
    if on {
        RULES.with(|r| {
            if let Some(f) = &*r.borrow() {
                unsafe { webkit_user_content_manager_add_filter(manager.as_ptr() as *mut c_void, f.0) };
            }
        });
    }
}

fn cache_dir() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".cache"))
        .join("crow")
}

/// Boxed Rust callback carried through a GAsyncReadyCallback.
type Finish = Box<dyn FnOnce(*mut glib::gobject_ffi::GObject, *mut gio::ffi::GAsyncResult)>;

unsafe extern "C" fn finish_trampoline(
    src: *mut glib::gobject_ffi::GObject,
    res: *mut gio::ffi::GAsyncResult,
    data: *mut c_void,
) {
    let f: Box<Finish> = unsafe { Box::from_raw(data as *mut Finish) };
    f(src, res);
}

fn finish_data(f: impl FnOnce(*mut glib::gobject_ffi::GObject, *mut gio::ffi::GAsyncResult) + 'static) -> *mut c_void {
    let f: Finish = Box::new(f);
    Box::into_raw(Box::new(f)) as *mut c_void
}

pub fn compile_rules(json: String, done: impl Fn() + 'static) {
    let dir = cache_dir().join("filters");
    let _ = std::fs::create_dir_all(&dir);
    let Ok(path) = CString::new(dir.to_string_lossy().as_bytes()) else { return };
    let bytes = glib::Bytes::from_owned(json.into_bytes());
    unsafe {
        let store = webkit_user_content_filter_store_new(path.as_ptr());
        let data = finish_data(move |src, res| {
            let mut err = ptr::null_mut();
            let f = webkit_user_content_filter_store_save_finish(src as *mut c_void, res, &mut err);
            if !f.is_null() {
                RULES.with(|r| *r.borrow_mut() = Some(Filter(f)));
                done();
            } else if !err.is_null() {
                let e: glib::Error = glib::translate::from_glib_full(err);
                eprintln!("adblock: {e}");
            }
            glib::gobject_ffi::g_object_unref(src);
        });
        webkit_user_content_filter_store_save(
            store,
            c"crow-adblock".as_ptr(),
            glib::translate::ToGlibPtr::to_glib_none(&bytes).0,
            ptr::null_mut(),
            Some(finish_trampoline),
            data,
        );
    }
}

fn same_site_str(p: soup::SameSitePolicy) -> Option<String> {
    match p {
        soup::SameSitePolicy::Lax => Some("lax".into()),
        soup::SameSitePolicy::Strict => Some("strict".into()),
        _ => None,
    }
}

pub fn export_cookies(store: &Store, done: impl Fn(Vec<CookieRec>) + 'static) {
    let Some(cm) = store.website_data_manager().and_then(|m| m.cookie_manager()) else {
        return done(Vec::new());
    };
    unsafe {
        let data = finish_data(move |src, res| {
            let mut err = ptr::null_mut();
            let list = webkit_cookie_manager_get_all_cookies_finish(src as *mut c_void, res, &mut err);
            if !err.is_null() {
                let e: glib::Error = glib::translate::from_glib_full(err);
                eprintln!("cookies: {e}");
            }
            let cookies: Vec<soup::Cookie> = FromGlibPtrContainer::from_glib_full(list);
            let recs = cookies
                .into_iter()
                .map(|mut c| CookieRec {
                    name: c.name().map(|s| s.to_string()).unwrap_or_default(),
                    value: c.value().map(|s| s.to_string()).unwrap_or_default(),
                    domain: c.domain().map(|s| s.to_string()).unwrap_or_default(),
                    path: c.path().map(|s| s.to_string()).unwrap_or_default(),
                    secure: c.is_secure(),
                    http_only: c.is_http_only(),
                    same_site: same_site_str(c.same_site_policy()),
                    expires: c.expires().map(|d| d.to_unix() as f64),
                })
                .collect();
            done(recs);
        });
        webkit_cookie_manager_get_all_cookies(
            cm.as_ptr() as *mut c_void,
            ptr::null_mut(),
            Some(finish_trampoline),
            data,
        );
    }
}

pub fn import_cookies(store: &Store, cookies: &[CookieRec], done: impl Fn() + 'static) {
    let now = crate::vault::now_ms() as f64 / 1000.0;
    let list: Vec<_> = cookies
        .iter()
        .filter(|c| c.expires.is_none_or(|e| e > now))
        .map(to_soup_cookie)
        .collect();
    let cm = store.website_data_manager().and_then(|m| m.cookie_manager());
    let (Some(cm), false) = (cm, list.is_empty()) else {
        done();
        return;
    };
    let left = Rc::new(Cell::new(list.len()));
    let done = Rc::new(done);
    for mut c in list {
        let (left, done) = (left.clone(), done.clone());
        cm.add_cookie(&mut c, None::<&gio::Cancellable>, move |_| {
            left.set(left.get() - 1);
            if left.get() == 0 {
                done();
            }
        });
    }
}

fn to_soup_cookie(c: &CookieRec) -> soup::Cookie {
    let mut k = soup::Cookie::new(&c.name, &c.value, &c.domain, &c.path, -1);
    if let Some(d) = c.expires.and_then(|e| glib::DateTime::from_unix_utc(e as i64).ok()) {
        k.set_expires(&d);
    }
    k.set_secure(c.secure);
    k.set_http_only(c.http_only);
    k.set_same_site_policy(match c.same_site.as_deref().map(str::to_ascii_lowercase).as_deref() {
        Some("lax") => soup::SameSitePolicy::Lax,
        Some("strict") => soup::SameSitePolicy::Strict,
        _ => soup::SameSitePolicy::None,
    });
    k
}

/// Removes the given cookies from WebKit (site-info "clear cookies").
pub fn delete_cookies(store: &Store, cookies: &[CookieRec], done: impl Fn() + 'static) {
    let cm = store.website_data_manager().and_then(|m| m.cookie_manager());
    let (Some(cm), false) = (cm, cookies.is_empty()) else {
        done();
        return;
    };
    let left = Rc::new(Cell::new(cookies.len()));
    let done = Rc::new(done);
    for c in cookies {
        let (left, done) = (left.clone(), done.clone());
        cm.delete_cookie(&mut to_soup_cookie(c), None::<&gio::Cancellable>, move |_| {
            left.set(left.get() - 1);
            if left.get() == 0 {
                done();
            }
        });
    }
}

/// Server certificate chain (DER, leaf first) and TLS problems of the committed page.
pub fn cert_chain(v: &WebView) -> Option<(Vec<Vec<u8>>, Vec<&'static str>)> {
    let (cert, flags) = v.webview().tls_info()?;
    let mut chain = Vec::new();
    let mut next = Some(cert);
    while let Some(c) = next.take().filter(|_| chain.len() < 8) {
        if let Some(der) = c.certificate() {
            chain.push(der.to_vec());
        }
        next = c.issuer();
    }
    use gio::TlsCertificateFlags as F;
    let errors = [
        (F::UNKNOWN_CA, "Certifikát nevydala dôveryhodná autorita"),
        (F::BAD_IDENTITY, "Certifikát nepatrí tejto doméne"),
        (F::NOT_ACTIVATED, "Certifikát ešte nie je platný"),
        (F::EXPIRED, "Platnosť certifikátu vypršala"),
        (F::REVOKED, "Certifikát bol zrušený"),
        (F::INSECURE, "Certifikát používa nezabezpečený algoritmus"),
        (F::GENERIC_ERROR, "Chyba pri overení certifikátu"),
    ];
    let errors = errors.iter().filter(|(f, _)| flags.contains(*f)).map(|(_, m)| *m).collect();
    Some((chain, errors))
}

/// Wipes cookies, cache and storage from the in-memory store.
pub fn clear_store(store: &Store) {
    if let Some(m) = store.website_data_manager() {
        let all = WebsiteDataTypes::ALL.bits();
        unsafe {
            webkit_website_data_manager_clear(m.as_ptr() as *mut c_void, all, 0, ptr::null_mut(), None, ptr::null_mut())
        };
    }
}

fn download_dir() -> PathBuf {
    glib::user_special_dir(glib::UserDirectory::Downloads)
        .unwrap_or_else(|| glib::home_dir().join("Downloads"))
}

fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let name = Path::new(name).file_name().and_then(|n| n.to_str()).filter(|n| !n.is_empty());
    let name = name.unwrap_or("download");
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s, format!(".{e}")),
        _ => (name, String::new()),
    };
    let mut p = dir.join(name);
    let mut n = 1;
    while p.exists() {
        p = dir.join(format!("{stem} ({n}){ext}"));
        n += 1;
    }
    p
}

/// Registered once on the shared context (wry would add one handler per webview).
/// Nothing is written until the user approves the prompt (`ask` → `decide_download`).
pub fn watch_downloads(
    store: &Store,
    ask: impl Fn(u32, String, String) + 'static,
    started: impl Fn(String, PathBuf) + 'static,
    finished: impl Fn(String, bool) + 'static,
) {
    if DOWNLOADS.with(|d| d.replace(true)) {
        return;
    }
    STARTED.with(|s| *s.borrow_mut() = Some(Rc::new(started)));
    let (ask, finished) = (Rc::new(ask), Rc::new(finished));
    let next = Rc::new(Cell::new(1u32));
    store.connect_download_started(move |_, d| {
        let (ask, next) = (ask.clone(), next.clone());
        d.connect_decide_destination(move |d, name| {
            let Some(uri) = d.request().and_then(|r| r.uri()) else { return false };
            let key = next.get();
            next.set(key + 1);
            let name = Path::new(name).file_name().map(|n| n.to_string_lossy().into_owned());
            let name = name.filter(|n| !n.is_empty()).unwrap_or_else(|| "download".into());
            ASKING.with(|a| a.borrow_mut().push((key, d.clone(), name.clone())));
            ask(key, uri.to_string(), name);
            true // destination is set later, in decide_download
        });
        let failed = Rc::new(Cell::new(false));
        let f = failed.clone();
        d.connect_failed(move |_, _| f.set(true));
        let finished = finished.clone();
        d.connect_finished(move |d| {
            if let Some(uri) = d.request().and_then(|r| r.uri()) {
                finished(uri.to_string(), !failed.get());
            }
        });
    });
}

/// Content area for all webviews, plus window-level keyboard shortcuts
/// (GTK delivers key presses to the toplevel before the focused webview).
pub fn init_window(window: &Window, on_cmd: impl Fn(String) + 'static) {
    let layout = gtk::Layout::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
    if let Some(vbox) = window.default_vbox() {
        vbox.pack_start(&layout, true, true, 0);
    }
    layout.show_all();
    LAYOUT.with(|l| *l.borrow_mut() = Some(layout));
    window.gtk_window().connect_key_press_event(move |_, ev| match shortcut(ev) {
        Some(cmd) => {
            on_cmd(cmd);
            glib::Propagation::Stop
        }
        None => glib::Propagation::Proceed,
    });
}

/// macOS menu shortcuts mapped to Linux conventions (⌘ → Ctrl, plus common browser aliases).
/// Digits and punctuation match by hardware key so they work on any layout (e.g. Slovak QWERTZ).
fn shortcut(ev: &gdk::EventKey) -> Option<String> {
    use gdk::ModifierType as M;
    let m = ev.state() & (M::CONTROL_MASK | M::SHIFT_MASK | M::MOD1_MASK);
    let (ctrl, shift, alt) = (m.contains(M::CONTROL_MASK), m.contains(M::SHIFT_MASK), m.contains(M::MOD1_MASK));
    let key = ev.keyval().to_lower().name().map(|n| n.to_ascii_lowercase()).unwrap_or_default();
    let code = ev.hardware_keycode();
    let cmd = match (ctrl, shift, alt) {
        (true, false, false) => match (key.as_str(), code) {
            (_, 10..=18) => return Some(format!("tab:{}", code - 9)),
            (_, 19) | ("kp_0", _) => "zoom-reset",
            (_, 20) | ("kp_subtract", _) => "zoom-out",
            (_, 21) | ("plus" | "kp_add", _) => "zoom-in",
            (_, 34) => "back",
            (_, 35) => "forward",
            (_, 59) => "panel:settings",
            ("t", _) => "new-tab",
            ("w" | "f4", _) => "close-tab",
            ("l", _) => "location",
            ("f", _) => "find",
            ("g", _) => "find-next",
            ("r", _) => "reload",
            ("d", _) => "bookmark",
            ("h" | "y", _) => "panel:history",
            ("q", _) => "quit",
            ("tab" | "page_down", _) => "next-tab",
            ("page_up", _) => "prev-tab",
            _ => return None,
        },
        (true, true, false) => match (key.as_str(), code) {
            (_, 21) => "zoom-in",
            (_, 34) | ("iso_left_tab" | "tab", _) => "prev-tab",
            (_, 35) => "next-tab",
            ("t", _) => "reopen-tab",
            ("g", _) => "find-prev",
            ("h", _) => "home",
            ("b" | "o", _) => "panel:bookmarks",
            ("y", _) => "panel:downloads",
            ("l", _) => "lock",
            _ => return None,
        },
        (false, false, true) => match key.as_str() {
            "home" => "home",
            "left" => "back",
            "right" => "forward",
            "d" => "location",
            _ => return None,
        },
        (false, false, false) => match key.as_str() {
            "f3" => "find-next",
            "f5" => "reload",
            "f6" => "location",
            "f11" => "fullscreen",
            _ => return None,
        },
        (false, true, false) if key == "f3" => "find-prev",
        _ => return None,
    };
    Some(cmd.to_string())
}

fn is_dark_name(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.ends_with("-dark") || n.ends_with(":dark")
}

/// WebKitGTK derives `prefers-color-scheme` (pages and browser UI) from these GTK settings.
pub fn set_theme(window: &Window, theme: Option<Theme>) {
    let Some(s) = gtk::Settings::default() else { return };
    let (name, dark) = THEME.with(|t| {
        t.borrow_mut()
            .get_or_insert_with(|| {
                (s.gtk_theme_name().map(|n| n.to_string()), s.is_gtk_application_prefer_dark_theme())
            })
            .clone()
    });
    let light_name = name.as_deref().map(|n| {
        let base = &n[..n.len() - if is_dark_name(n) { 5 } else { 0 }];
        if base.is_empty() { "Adwaita".to_string() } else { base.to_string() }
    });
    match theme {
        Some(Theme::Dark) => {
            s.set_gtk_theme_name(name.as_deref());
            s.set_gtk_application_prefer_dark_theme(true);
        }
        Some(Theme::Light) => {
            s.set_gtk_theme_name(light_name.as_deref());
            s.set_gtk_application_prefer_dark_theme(false);
        }
        _ => {
            s.set_gtk_theme_name(name.as_deref());
            // GTK theme says light, but the desktop portal may prefer dark.
            let portal_dark = window.theme() == Theme::Dark;
            s.set_gtk_application_prefer_dark_theme(dark || portal_dark);
        }
    }
}

pub fn is_light(_window: &Window) -> bool {
    let Some(s) = gtk::Settings::default() else { return false };
    let env_dark = std::env::var("GTK_THEME").is_ok_and(|t| is_dark_name(&t));
    let name_dark = s.gtk_theme_name().is_some_and(|n| is_dark_name(&n));
    !(env_dark || name_dark || s.is_gtk_application_prefer_dark_theme())
}

/// Use native decorations instead of the drawn traffic lights (CROW_DECORATIONS=1).
pub fn native_decorations() -> bool {
    std::env::var_os("CROW_DECORATIONS").is_some_and(|v| v != "0")
}

/// Adapts the macOS-written browser UI: shortcut labels, paths, drawn window controls.
pub fn ui_html(html: String) -> String {
    const MAP: &[(&str, &str)] = &[
        ("⌃⌘L", "Ctrl+Shift+L"),
        ("⌘⌥B", "Ctrl+Shift+B"),
        ("⌘⌥L", "Ctrl+Shift+Y"),
        ("⌘⇧H", "Alt+Home"),
        ("⌘⇧G", "Shift+F3"),
        ("⌘+ / ⌘− / ⌘0", "Ctrl++ / Ctrl+− / Ctrl+0"),
        ("⌘[", "Alt+←"),
        ("⌘]", "Alt+→"),
        ("⌘G", "F3"),
        ("⌘", "Ctrl+"),
        ("Zobraziť vo Finderi", "Zobraziť v priečinku"),
        ("~/Library/Application Support/Crow/", "~/.local/share/crow/"),
    ];
    let mut html = html;
    for (a, b) in MAP {
        html = html.replace(a, b);
    }
    let class = if native_decorations() { "linux" } else { "linux chrome" };
    html.replacen("<body>", &format!("<body class=\"{class}\">"), 1)
}

fn file_uri(path: &str) -> String {
    let mut s = String::from("file://");
    for b in path.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => s.push(b as char),
            _ => s.push_str(&format!("%{b:02X}")),
        }
    }
    s
}

/// Answer to the download prompt: set the destination (starts writing) or cancel.
pub fn decide_download(key: u32, allow: bool, _url: &str, _view: Option<&WebView>) {
    let d = ASKING.with(|a| {
        let mut a = a.borrow_mut();
        let i = a.iter().position(|(k, ..)| *k == key)?;
        let (_, d, name) = a.remove(i);
        Some((d, name))
    });
    let Some((d, name)) = d else { return };
    if !allow {
        d.cancel();
        return;
    }
    let Some(uri) = d.request().and_then(|r| r.uri()) else { return d.cancel() };
    let dir = download_dir();
    let _ = std::fs::create_dir_all(&dir);
    let path = unique_path(&dir, &name);
    d.set_destination(&path.to_string_lossy());
    if let Some(f) = STARTED.with(|s| s.borrow().clone()) {
        f(uri.to_string(), path);
    }
}

/// Never opens/executes a downloaded file: highlights it in the file manager (FileManager1),
/// falls back to opening its parent folder.
pub fn reveal(path: &str) {
    let path = path.to_string();
    std::thread::spawn(move || {
        let ok = std::process::Command::new("dbus-send")
            .args([
                "--session",
                "--print-reply",
                "--dest=org.freedesktop.FileManager1",
                "--type=method_call",
                "/org/freedesktop/FileManager1",
                "org.freedesktop.FileManager1.ShowItems",
                &format!("array:string:{}", file_uri(&path)),
                "string:",
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if !ok && let Some(dir) = Path::new(&path).parent() {
            let _ = std::process::Command::new("xdg-open").arg(dir).status();
        }
    });
}
