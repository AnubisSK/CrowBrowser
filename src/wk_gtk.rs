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
    CacheModel, CookieManagerExt, TLSErrorsPolicy, DownloadExt, PolicyDecisionExt, PolicyDecisionType,
    ResponsePolicyDecisionExt, SettingsExt, URIRequestExt, URIResponseExt, UserContentManagerExt, WebContext,
    WebContextExt, WebProcessTerminationReason, WebViewExt, WebsiteDataManagerExt, WebsiteDataTypes,
};
use wry::{
    NewWindowFeatures, NewWindowResponse, Rect, WebView, WebViewBuilder, WebViewBuilderExtUnix,
    WebViewExtUnix,
};

/// The shared ephemeral WebKitWebContext: nothing written to disk by WebKit.
pub type Store = WebContext;
/// Certificate failure callback: tab, refused URL, reasons.
type TlsHandler = dyn Fn(u32, String, Vec<&'static str>);
/// Tabs are created related to an existing view, which puts them in the shared context.
pub struct Config(webkit2gtk::WebView);

/// Engine-confirmed connection security of a committed page.
#[derive(Clone, Copy, PartialEq)]
pub enum Tls {
    /// Certificate validated by WebKit, no insecure subresources.
    Secure,
    /// Valid certificate, but HTTP content on the page (not detected on Linux).
    #[allow(dead_code)]
    Mixed,
    /// No validated certificate (certificate errors, or not HTTPS).
    Unverified,
}

// String results of `eval_isolated` (javascriptcore-rs is not a direct dependency).
#[link(name = "javascriptcoregtk-4.1")]
unsafe extern "C" {
    fn jsc_value_is_string(value: *mut c_void) -> glib::ffi::gboolean;
    fn jsc_value_to_string(value: *mut c_void) -> *mut c_char;
}

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
    fn webkit_user_content_filter_store_load(
        store: *mut c_void,
        id: *const c_char,
        cancellable: *mut gio::ffi::GCancellable,
        callback: GAsyncReadyCallback,
        data: *mut c_void,
    );
    fn webkit_user_content_filter_store_load_finish(
        store: *mut c_void,
        res: *mut gio::ffi::GAsyncResult,
        err: *mut *mut glib::ffi::GError,
    ) -> *mut c_void;
    fn webkit_user_content_filter_store_remove(
        store: *mut c_void,
        id: *const c_char,
        cancellable: *mut gio::ffi::GCancellable,
        callback: GAsyncReadyCallback,
        data: *mut c_void,
    );
    fn webkit_user_content_filter_store_remove_finish(
        store: *mut c_void,
        res: *mut gio::ffi::GAsyncResult,
        err: *mut *mut glib::ffi::GError,
    ) -> glib::ffi::gboolean;
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
    /// Compiled content filters by id (see `adblock::LISTS`).
    static RULES: RefCell<Vec<(&'static str, Filter)>> = const { RefCell::new(Vec::new()) };
    static ROOT: RefCell<Option<webkit2gtk::WebView>> = const { RefCell::new(None) };
    static UI_ROOT: RefCell<Option<webkit2gtk::WebView>> = const { RefCell::new(None) };
    static LAYOUT: RefCell<Option<gtk::Layout>> = const { RefCell::new(None) };
    static THEME: RefCell<Option<(Option<String>, bool)>> = const { RefCell::new(None) };
    static DOWNLOADS: Cell<bool> = const { Cell::new(false) };
    /// Downloads waiting for the user's answer (decide-destination is answered asynchronously).
    static ASKING: RefCell<Vec<(u32, webkit2gtk::Download, String)>> = const { RefCell::new(Vec::new()) };
    static STARTED: RefCell<Option<Rc<Started>>> = const { RefCell::new(None) };
    static ON_TLS: RefCell<Option<Rc<TlsHandler>>> = const { RefCell::new(None) };
    static ON_FULLSCREEN: RefCell<Option<Rc<dyn Fn(u32, bool)>>> = const { RefCell::new(None) };
    static ON_CRASH: RefCell<Option<Rc<dyn Fn(u32)>>> = const { RefCell::new(None) };
    static ON_NET_ERROR: RefCell<Option<Rc<dyn Fn(u32, String, &'static str)>>> = const { RefCell::new(None) };
    /// Hidden root view of each container's own ephemeral context (see `new_container_store`).
    static CONTAINER_ROOTS: RefCell<Vec<webkit2gtk::WebView>> = const { RefCell::new(Vec::new()) };
    /// Download prompt hookup, also attached to container contexts.
    static DOWNLOAD_HOOK: RefCell<Option<Rc<dyn Fn(&WebContext)>>> = const { RefCell::new(None) };
}

/// Ephemeral context: nothing on disk, certificate errors refuse the load.
fn ephemeral_context() -> WebContext {
    let ctx = WebContext::new_ephemeral();
    ctx.set_cache_model(CacheModel::DocumentViewer);
    // Explicit: certificate errors refuse the load, never an "ignore" fallback.
    if let Some(m) = ctx.website_data_manager() {
        m.set_tls_errors_policy(TLSErrorsPolicy::Fail);
    }
    ctx
}

/// Hidden, never-loaded view owning an ephemeral context; views created related to it share
/// that context. Web content uses `ROOT`, the browser UI its own `UI_ROOT` (no shared state).
fn root_of(key: &'static std::thread::LocalKey<RefCell<Option<webkit2gtk::WebView>>>) -> webkit2gtk::WebView {
    key.with(|r| {
        r.borrow_mut()
            .get_or_insert_with(|| webkit2gtk::WebView::with_context(&ephemeral_context()))
            .clone()
    })
}

fn root() -> webkit2gtk::WebView {
    root_of(&ROOT)
}

pub fn new_store() -> Store {
    root().context().expect("webkit context")
}

/// Separate ephemeral context of a container (own cookies, storage, cache, network process),
/// with the same download prompt as the shared one.
pub fn new_container_store() -> Store {
    let ctx = ephemeral_context();
    if let Some(hook) = DOWNLOAD_HOOK.with(|h| h.borrow().clone()) {
        hook(&ctx);
    }
    CONTAINER_ROOTS.with(|r| r.borrow_mut().push(webkit2gtk::WebView::with_context(&ctx)));
    ctx
}

/// Forgets the container's root view; the context goes with its last tab.
pub fn drop_container_store(store: &Store) {
    CONTAINER_ROOTS.with(|r| r.borrow_mut().retain(|v| v.context().as_ref() != Some(store)));
}

/// No picture-in-picture in WebKitGTK.
pub const PIP: bool = false;

/// Not reported by WebKitGTK (see `BLOCK_STATS`).
pub fn take_blocked_hosts() -> Vec<(String, u32)> {
    Vec::new()
}

/// WebKitGTK 2.40 has no gamepad switch (`WebKitFeature` is 2.42+): with `gamepad` off the
/// page-side shim removes the API.
pub fn config(store: &Store, _adblock: bool, _gamepad: bool) -> Config {
    let container = CONTAINER_ROOTS.with(|r| r.borrow().iter().find(|v| v.context().as_ref() == Some(store)).cloned());
    Config(container.unwrap_or_else(root))
}

/// window.open target: must be related to the opener so `window.opener` keeps working.
pub fn popup_config(features: NewWindowFeatures) -> Config {
    Config(features.opener.webview)
}

/// WebKitGTK's default UA makes Google/Gmail show "unsupported browser". Native Safari identity
/// only: a Chrome UA makes Google serve Blink-only layouts that break in WebKit.
/// WebKitSettings UA covers the request header and `navigator.userAgent` alike.
#[cfg(target_os = "linux")]
const USER_AGENT: &str =
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.3 Safari/605.1.15";

/// Tab views (including adopted popups); the browser UI uses `ui_builder`.
/// `incognito` only stops wry from creating a throwaway *persistent* context (which would write
/// HSTS/storage files to disk); the related view decides the context actually used.
/// `page_js`: the page-world shim (`adblock::defuser`), injected into every frame.
pub fn builder<'a>(config: Config, page_js: String) -> WebViewBuilder<'a> {
    WebViewBuilder::new()
        .with_incognito(true)
        .with_related_view(config.0)
        .with_user_agent(USER_AGENT)
        .with_initialization_script_for_main_only(page_js, false)
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

/// MIME sniffing defense for HTTP(S) documents: a response is never rendered under a type the
/// server did not declare. No `Content-Type` at all (WebKit would sniff, e.g. an uploaded file
/// into HTML) or, with `X-Content-Type-Options: nosniff`, a declared type different from the
/// one WebKit would use: the response goes to the download prompt instead of being rendered.
fn sniffed(uri: &str, declared: Option<String>, options: Option<String>, res: Option<&webkit2gtk::URIResponse>) -> bool {
    if !(uri.starts_with("http:") || uri.starts_with("https:")) {
        return false;
    }
    let declared = declared.map(|v| v.split(';').next().unwrap_or("").trim().to_string()).filter(|v| !v.is_empty());
    let Some(declared) = declared else { return true };
    let nosniff = options.is_some_and(|v| v.split(',').any(|t| t.trim() == "nosniff"));
    let effective = res.and_then(|r| r.mime_type()).map(|m| m.to_ascii_lowercase());
    nosniff && effective.is_some_and(|e| e != declared)
}

/// WebKitGTK does not emit `WebContext::download-started` for responses it turns into
/// downloads on its own (unsupported MIME type, `Content-Disposition: attachment`), so those
/// would vanish. Start them explicitly; the shared handler (`watch_downloads`) then asks the user.
fn start_downloads(w: &webkit2gtk::WebView, d: &webkit2gtk::PolicyDecision, t: PolicyDecisionType) -> bool {
    if t != PolicyDecisionType::Response {
        return false;
    }
    let Some(r) = d.downcast_ref::<webkit2gtk::ResponsePolicyDecision>() else { return false };
    let Some(uri) = r.request().and_then(|q| q.uri()) else { return false };
    let response = r.response();
    let headers = response.as_ref().and_then(|res| res.http_headers());
    let header = |name: &str| headers.as_ref().and_then(|h| h.one(name)).map(|v| v.trim().to_ascii_lowercase());
    let attachment = header("Content-Disposition").is_some_and(|v| v.starts_with("attachment"));
    let sniffed = sniffed(&uri, header("Content-Type"), header("X-Content-Type-Options"), response.as_ref());
    if r.is_mime_type_supported() && !attachment && !sniffed {
        return false;
    }
    d.ignore();
    // A mistyped subframe is just not rendered (no download prompts from embedded content).
    if sniffed && !attachment && !r.is_main_frame_main_resource() {
        return true;
    }
    w.download_uri(&uri);
    true
}

/// Built-in page `html` under the document address `base` (see `app::INTERNAL_BASE`).
pub fn load_internal(v: &WebView, html: &str, base: &str) {
    v.webview().load_html(html, Some(base));
}

/// Not available on Linux (WebKitGTK reads pads through libmanette itself).
pub fn system_gamepads() -> Option<Vec<String>> {
    None
}

/// Runs `js` at document start in every frame of `v`, in Crow's isolated JS world (see
/// `eval_isolated`). Called before the view's first load.
pub fn add_isolated_script(v: &WebView, js: &str) {
    let Some(manager) = v.webview().user_content_manager() else { return };
    let script = webkit2gtk::UserScript::for_world(
        js,
        webkit2gtk::UserContentInjectedFrames::AllFrames,
        webkit2gtk::UserScriptInjectionTime::Start,
        "crow",
        &[],
        &[],
    );
    manager.add_script(&script);
}

/// Runs `js` in the main frame in Crow's own JS world: same DOM, but globals set there are
/// invisible to the page's scripts. `done` gets the result if it is a string, else "".
pub fn eval_isolated(v: &WebView, js: &str, done: impl FnOnce(String) + 'static) {
    v.webview().evaluate_javascript(js, Some("crow"), None, None::<&gio::Cancellable>, move |res| {
        let s = res
            .ok()
            .and_then(|val| unsafe {
                let p = val.as_ptr().cast::<c_void>();
                (jsc_value_is_string(p) != 0).then(|| {
                    let g: glib::GString = glib::translate::from_glib_full(jsc_value_to_string(p));
                    g.to_string()
                })
            })
            .unwrap_or_default();
        done(s);
    });
}

/// GTK's move drag checks the pointer itself; nothing to guard here.
pub fn left_button_down() -> bool {
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
            for (_, f) in &*r.borrow() {
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

fn filter_store() -> Option<*mut c_void> {
    let dir = cache_dir().join("filters");
    let _ = std::fs::create_dir_all(&dir);
    let path = CString::new(dir.to_string_lossy().as_bytes()).ok()?;
    Some(unsafe { webkit_user_content_filter_store_new(path.as_ptr()) })
}

fn keep_rules(id: &'static str, f: *mut c_void) {
    RULES.with(|r| {
        let mut r = r.borrow_mut();
        r.retain(|(i, _)| *i != id);
        r.push((id, Filter(f)));
    });
}

/// Compiles rule list `id` (stored on disk by WebKit, reused via `load_rules`).
pub fn compile_rules(id: &'static str, json: String, done: impl Fn() + 'static) {
    let (Some(store), Ok(cid)) = (filter_store(), CString::new(id)) else { return };
    let bytes = glib::Bytes::from_owned(json.into_bytes());
    unsafe {
        let data = finish_data(move |src, res| {
            let mut err = ptr::null_mut();
            let f = webkit_user_content_filter_store_save_finish(src as *mut c_void, res, &mut err);
            if !f.is_null() {
                keep_rules(id, f);
                done();
            } else if !err.is_null() {
                let e: glib::Error = glib::translate::from_glib_full(err);
                eprintln!("adblock: {e}");
            }
            glib::gobject_ffi::g_object_unref(src);
        });
        webkit_user_content_filter_store_save(
            store,
            cid.as_ptr(),
            glib::translate::ToGlibPtr::to_glib_none(&bytes).0,
            ptr::null_mut(),
            Some(finish_trampoline),
            data,
        );
    }
}

/// Loads the previously compiled list `id`; `fail` if the store no longer has it.
pub fn load_rules(id: &'static str, ok: impl Fn() + 'static, fail: impl Fn() + 'static) {
    let (Some(store), Ok(cid)) = (filter_store(), CString::new(id)) else { return fail() };
    unsafe {
        let data = finish_data(move |src, res| {
            let mut err = ptr::null_mut();
            let f = webkit_user_content_filter_store_load_finish(src as *mut c_void, res, &mut err);
            if !err.is_null() {
                glib::ffi::g_error_free(err);
            }
            glib::gobject_ffi::g_object_unref(src);
            if f.is_null() {
                return fail();
            }
            keep_rules(id, f);
            ok();
        });
        webkit_user_content_filter_store_load(store, cid.as_ptr(), ptr::null_mut(), Some(finish_trampoline), data);
    }
}

/// Disabled list: forgotten here and removed from the store.
pub fn drop_rules(id: &'static str) {
    RULES.with(|r| r.borrow_mut().retain(|(i, _)| *i != id));
    let (Some(store), Ok(cid)) = (filter_store(), CString::new(id)) else { return };
    unsafe {
        let data = finish_data(|src, res| {
            webkit_user_content_filter_store_remove_finish(src as *mut c_void, res, ptr::null_mut());
            glib::gobject_ffi::g_object_unref(src);
        });
        webkit_user_content_filter_store_remove(store, cid.as_ptr(), ptr::null_mut(), Some(finish_trampoline), data);
    }
}

fn same_site_str(p: soup::SameSitePolicy) -> Option<String> {
    match p {
        soup::SameSitePolicy::Lax => Some("lax".into()),
        soup::SameSitePolicy::Strict => Some("strict".into()),
        _ => None,
    }
}

/// WebKitGTK has no API reporting content-filter actions: blocked loads are not counted.
pub const BLOCK_STATS: bool = false;

pub fn on_blocked(_f: impl Fn() + 'static) {}

/// Called when a navigation is refused because of the server certificate (tab, URL, reasons).
pub fn on_tls_error(f: impl Fn(u32, String, Vec<&'static str>) + 'static) {
    ON_TLS.with(|o| *o.borrow_mut() = Some(Rc::new(f)));
}

fn tls_reasons(flags: gio::TlsCertificateFlags) -> Vec<&'static str> {
    use gio::TlsCertificateFlags as F;
    let all = [
        (F::UNKNOWN_CA, "cert_err_unknown_ca"),
        (F::BAD_IDENTITY, "cert_err_bad_identity"),
        (F::NOT_ACTIVATED, "cert_err_not_activated"),
        (F::EXPIRED, "cert_err_expired"),
        (F::REVOKED, "cert_err_revoked"),
        (F::INSECURE, "cert_err_insecure"),
        (F::GENERIC_ERROR, "cert_err_generic"),
    ];
    let r: Vec<_> = all.iter().filter(|(f, _)| flags.contains(*f)).map(|(_, m)| *m).collect();
    if r.is_empty() { vec!["cert_err_unverified"] } else { r }
}

/// The main page of a tab could not be loaded (network): `f` gets tab, URL and reason.
pub fn on_net_error(f: impl Fn(u32, String, &'static str) + 'static) {
    ON_NET_ERROR.with(|o| *o.borrow_mut() = Some(Rc::new(f)));
}

/// Reason (`locales.json` key) of a failed page load, or None when it is not an error to show:
/// cancelled loads and policy decisions (downloads, MIME types WebKit hands elsewhere).
fn net_reason(e: &glib::Error) -> Option<&'static str> {
    use gio::IOErrorEnum as Io;
    if e.kind::<webkit2gtk::PolicyError>().is_some() || e.matches(webkit2gtk::NetworkError::Cancelled) || e.matches(Io::Cancelled) {
        return None;
    }
    Some(if e.kind::<gio::ResolverError>().is_some() {
        "net_dns"
    } else {
        match e.kind::<Io>() {
            Some(Io::TimedOut) => "net_timeout",
            Some(Io::ConnectionRefused) => "net_refused",
            Some(Io::NetworkUnreachable | Io::HostUnreachable) => "net_offline",
            Some(Io::ConnectionClosed | Io::BrokenPipe) => "net_lost",
            _ => "net_generic",
        }
    })
}

/// Web content process of a tab ended (crash, out of memory, WebGL hang): `f` gets the tab.
pub fn on_crash(f: impl Fn(u32) + 'static) {
    ON_CRASH.with(|o| *o.borrow_mut() = Some(Rc::new(f)));
}

/// Certificate failures of `v` are reported as tab `tab`. The context's policy is FAIL (see
/// `root_of`), so the load is already refused; returning true suppresses WebKit's own error page
/// in favor of Crow's interstitial. Web process crashes are reported too (not Crow's own
/// `terminate_web_process`, which it never calls).
pub fn watch(v: &WebView, tab: u32) {
    watch_fullscreen(v, tab);
    v.webview().connect_web_process_terminated(move |_, reason| {
        let why = match reason {
            WebProcessTerminationReason::Crashed => "crashed",
            WebProcessTerminationReason::ExceededMemoryLimit => "exceeded memory limit",
            WebProcessTerminationReason::TerminatedByApi => return,
            _ => "terminated",
        };
        eprintln!("web process of tab {tab} {why}");
        if let Some(f) = ON_CRASH.with(|o| o.borrow().clone()) {
            f(tab);
        }
    });
    // Network failures get Crow's page (with the offline game) instead of WebKit's own.
    v.webview().connect_load_failed(move |_, _, uri, error| {
        let Some(reason) = net_reason(error) else { return false };
        if let Some(f) = ON_NET_ERROR.with(|o| o.borrow().clone()) {
            f(tab, uri.to_string(), reason);
        }
        true
    });
    v.webview().connect_load_failed_with_tls_errors(move |_, uri, _cert, flags| {
        if let Some(f) = ON_TLS.with(|o| o.borrow().clone()) {
            f(tab, uri.to_string(), tls_reasons(flags));
        }
        true
    });
}

/// Element fullscreen (video players): the app is told so it can give the tab view the whole
/// window (it otherwise stays under the toolbar inside the layout).
pub fn on_fullscreen(f: impl Fn(u32, bool) + 'static) {
    ON_FULLSCREEN.with(|o| *o.borrow_mut() = Some(Rc::new(f)));
}

/// Enables the Fullscreen API on the view; returning false keeps WebKit's default handler,
/// which fullscreens the toplevel GTK (tao) window. Web storage (in memory, the context is
/// ephemeral) is enabled explicitly too, for sites that hydrate from it. A `file:` page may read
/// neither other files nor other origins, and page console output never reaches stdout / the
/// journal (WebKit's defaults, set explicitly).
fn watch_fullscreen(v: &WebView, tab: u32) {
    let wv = v.webview();
    if let Some(s) = WebViewExt::settings(&wv) {
        s.set_enable_fullscreen(true);
        s.set_enable_html5_local_storage(true);
        s.set_enable_html5_database(true);
        s.set_allow_file_access_from_file_urls(false);
        s.set_allow_universal_access_from_file_urls(false);
        s.set_enable_write_console_messages_to_stdout(false);
    }
    let notify = move |on: bool| {
        if let Some(f) = ON_FULLSCREEN.with(|o| o.borrow().clone()) {
            f(tab, on);
        }
        false
    };
    wv.connect_enter_fullscreen(move |_| notify(true));
    wv.connect_leave_fullscreen(move |_| notify(false));
}

/// Tab is closing/unloading: detach the view from the layout before wry destroys it, so no
/// container reference keeps the WebKitWebView (and its web process) alive.
pub fn forget(v: &WebView) {
    let wv = v.webview();
    if let Some(layout) = LAYOUT.with(|l| l.borrow().clone())
        && wv.parent().is_some()
    {
        layout.remove(&wv);
    }
    // Tear down now (web process, DOM, JS context) even if something else still holds a ref;
    // wry's own destroy on drop is then a no-op on an already destroyed widget.
    unsafe { wv.destroy() };
}

/// TLS state of the committed page as verified by WebKit (never inferred from the URL).
/// Mixed content is not reported by WebKitGTK.
pub fn tls(v: &WebView) -> Tls {
    match v.webview().tls_info() {
        Some((_, flags)) if flags.is_empty() => Tls::Secure,
        _ => Tls::Unverified,
    }
}

pub fn take_blocked() -> Vec<(u32, u32)> {
    Vec::new()
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
        (F::UNKNOWN_CA, "cert_err_unknown_ca"),
        (F::BAD_IDENTITY, "cert_err_bad_identity"),
        (F::NOT_ACTIVATED, "cert_err_not_activated"),
        (F::EXPIRED, "cert_err_expired"),
        (F::REVOKED, "cert_err_revoked"),
        (F::INSECURE, "cert_err_insecure"),
        (F::GENERIC_ERROR, "cert_err_generic"),
    ];
    let errors = errors.iter().filter(|(f, _)| flags.contains(*f)).map(|(_, m)| *m).collect();
    Some((chain, errors))
}

/// Wipes cookies, cache, storage and credentials from the in-memory store; `done` runs after.
pub fn clear_store(store: &Store, done: impl FnOnce() + 'static) {
    let Some(m) = store.website_data_manager() else { return done() };
    let all = WebsiteDataTypes::ALL.bits();
    let data = finish_data(move |_, _| done());
    unsafe {
        webkit_website_data_manager_clear(
            m.as_ptr() as *mut c_void,
            all,
            0,
            ptr::null_mut(),
            Some(finish_trampoline),
            data,
        );
    };
}

pub fn download_dir() -> PathBuf {
    glib::user_special_dir(glib::UserDirectory::Downloads)
        .unwrap_or_else(|| glib::home_dir().join("Downloads"))
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
    let hook: Rc<dyn Fn(&WebContext)> = Rc::new(move |ctx: &WebContext| {
        let (ask, finished, next) = (ask.clone(), finished.clone(), next.clone());
        hook_downloads(ctx, ask, finished, next);
    });
    hook(store);
    DOWNLOAD_HOOK.with(|h| *h.borrow_mut() = Some(hook));
}

type AskFn = dyn Fn(u32, String, String);
type FinishedFn = dyn Fn(String, bool);

fn hook_downloads(store: &WebContext, ask: Rc<AskFn>, finished: Rc<FinishedFn>, next: Rc<Cell<u32>>) {
    store.connect_download_started(move |_, d| {
        let (ask, next) = (ask.clone(), next.clone());
        d.connect_decide_destination(move |d, name| {
            let Some(uri) = d.request().and_then(|r| r.uri()) else { return false };
            let key = next.get();
            next.set(key + 1);
            // Sanitized before the prompt shows it and before any path is built from it.
            let name = crate::download::sanitize(name);
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
            ("x", _) => "panic",
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
        ("ukáže vo Finderi", "ukáže v správcovi súborov"),
        ("Zobrazit ve Finderu", "Zobrazit ve složce"),
        ("zobrazí ve Finderu", "zobrazí ve správci souborů"),
        ("Show in Finder", "Show in folder"),
        ("shown in Finder", "shown in the file manager"),
        ("Im Finder zeigen", "Im Ordner zeigen"),
        ("im Finder angezeigt", "im Dateimanager angezeigt"),
        ("⌥klik", "Alt+klik"),
        ("⌥click", "Alt+click"),
        ("⌥Klick", "Alt+Klick"),
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
    let path = crate::download::unique_path(&dir, &name);
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
