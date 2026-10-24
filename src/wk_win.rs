//! WebView2 glue (Windows), same API as `wk.rs` / `wk_gtk.rs`: one shared environment whose
//! profiles are all InPrivate (browsing data in memory only), a hidden root view per store for
//! cookie export/import, network-level blocking, Crow's isolated JS world (DevTools protocol),
//! crash / TLS / network-error reporting, downloads and window-level shortcuts.
#![allow(clippy::wildcard_imports, clippy::borrow_as_ptr, clippy::type_complexity)] // COM out-params, WebView2 API

use crate::vault::CookieRec;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    fmt::Write as _,
    path::{Path, PathBuf},
    rc::Rc,
    sync::mpsc,
    time::{Duration, Instant},
};
use tao::{
    platform::windows::WindowExtWindows,
    window::{Theme, Window},
};
use webview2_com::{Microsoft::Web::WebView2::Win32::*, *};
use windows::{
    Win32::{
        Foundation::{CloseHandle, E_FAIL, E_POINTER, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::{
            Dwm::DwmExtendFrameIntoClientArea,
            Gdi::ScreenToClient,
        },
        UI::{
            Controls::MARGINS,
            HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi},
            Shell::{DefSubclassProc, SetWindowSubclass},
            WindowsAndMessaging::{
                GetClientRect, HTCAPTION, HTCLIENT, HTTOP, HTTOPLEFT, HTTOPRIGHT, IsZoomed, NCCALCSIZE_PARAMS,
                PostMessageW, SM_CXFRAME, SM_CXPADDEDBORDER, SM_CYFRAME, SW_MAXIMIZE, SW_MINIMIZE, SW_RESTORE,
                SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos, ShowWindow,
                WM_ACTIVATE, WM_CLOSE, WM_DWMCOMPOSITIONCHANGED, WM_NCCALCSIZE, WM_NCHITTEST,
            },
        },
        System::{
            Com::{COINIT_APARTMENTTHREADED, CoInitializeEx},
            Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
        },
        UI::{
            Input::KeyboardAndMouse::{
                GetAsyncKeyState, GetKeyState, VIRTUAL_KEY, VK_ADD, VK_CONTROL, VK_F3, VK_F4, VK_F5, VK_F6,
                VK_F11, VK_HOME, VK_LBUTTON, VK_LEFT, VK_MENU, VK_NEXT, VK_NUMPAD0, VK_OEM_4, VK_OEM_6,
                VK_OEM_COMMA, VK_OEM_MINUS, VK_OEM_PLUS, VK_PRIOR, VK_RIGHT, VK_SHIFT, VK_SUBTRACT, VK_TAB,
            },
            Shell::{
                FOLDERID_Downloads, ILCreateFromPathW, ILFree, KF_FLAG_DEFAULT, SHCreateMemStream,
                SHGetKnownFolderPath, SHOpenFolderAndSelectItems,
            },
        },
    },
    core::{HSTRING, Interface, PCWSTR, PWSTR, w},
};
use wry::{
    NewWindowFeatures, NewWindowResponse, Rect, WebView, WebViewBuilder, WebViewBuilderExtWindows,
    WebViewExtWindows,
};

/// Engine-confirmed connection security of a committed page.
#[derive(Clone, Copy, PartialEq)]
pub enum Tls {
    /// Certificate validated by the engine, no insecure subresources.
    Secure,
    /// Valid certificate, but HTTP content on the page.
    Mixed,
    /// No validated certificate (certificate errors, or not HTTPS).
    Unverified,
}

/// In-memory profile of one cookie/storage jar: every store is its own InPrivate profile, held
/// open by a hidden, never-navigated root view (cookies can only be reached through a view).
/// Dropping the last handle closes the root; with its last tab the profile's data is gone.
pub struct StoreInner {
    profile: String,
    root: Option<ICoreWebView2Controller>,
}

pub type Store = Rc<StoreInner>;

impl Drop for StoreInner {
    fn drop(&mut self) {
        PROFILES.with(|p| p.borrow_mut().retain(|(n, _)| *n != self.profile));
        if let Some(c) = self.root.take() {
            let _ = unsafe { c.Close() };
        }
    }
}

/// Tabs (and adopted popups) are created in their store's profile of the shared environment.
pub struct Config {
    profile: String,
}

/// Browser arguments of the shared environment. In-memory browsing data is enforced by the
/// InPrivate profiles (`SetIsInPrivateModeEnabled`), the switches only keep Edge features from
/// phoning home or writing caches into the (ephemeral) user data folder.
const BROWSER_ARGS: &str = "--in-memory --incognito \
--disable-features=msSmartScreenProtection,AutofillServerCommunication,msWebOOUI,msPdfOOUI \
--disable-sync --disable-gpu-shader-disk-cache --no-first-run";
/// Browser UI (toolbar, panel, lock screen, popovers): its own profile, no state shared with pages.
const UI_PROFILE: &str = "crow-ui";
/// Name of Crow's isolated JS world in every frame of a tab.
const WORLD: &str = "crow";
/// Folder prefix of the per-run user data folder in `%TEMP%` (removed on exit / next start).
const DATA_PREFIX: &str = "crow-wv2-";
/// A hung page reports "unresponsive" repeatedly; one crash report per tab in this window.
const CRASH_DEBOUNCE: Duration = Duration::from_secs(5);

type TlsHandler = dyn Fn(u32, String, Vec<&'static str>);
type NetHandler = dyn Fn(u32, String, &'static str);

/// Per-view state, keyed by the view's `ICoreWebView2` pointer (see `key`).
#[derive(Default)]
struct ViewState {
    tab: Option<u32>,
    /// Content blocker on for this view (`set_rules`).
    rules: bool,
    /// The catch-all `WebResourceRequested` filter is installed.
    filtered: bool,
    /// Built-in page served under `internal_base` while it is shown (`load_internal`).
    internal: Option<Rc<str>>,
    internal_base: String,
    /// The next navigation to `internal_base` was started by Crow (any other one is refused).
    internal_pending: bool,
    /// Main-frame navigation in progress / last committed, and its host (third-party checks).
    nav_uri: String,
    top_host: String,
    /// The current navigation failed for a reason already handled (it turned into a download, or
    /// its certificate was refused and reported): no network error page.
    quiet: bool,
    /// Certificate refused for the current navigation: reported once it has ended, so the
    /// interstitial is not wiped by the cancelled load finishing after it.
    cert_refused: Option<&'static str>,
    tls: Option<Tls>,
    chain: Vec<Vec<u8>>,
    /// DevTools id of the main frame (stable for the view's lifetime).
    frame: Option<String>,
    crashed_at: Option<Instant>,
    security: Option<ICoreWebView2DevToolsProtocolEventReceiver>,
}

/// Domain rules of one content-blocker list (see `compile_rules`).
#[derive(Default)]
struct Rules {
    /// Blocked domain → (resource-type mask, 0 = any; third-party only) per rule.
    block: HashMap<Box<str>, Vec<(u16, bool)>>,
    /// Unconditional exceptions (`@@||domain^`): cancel this list's rules for the domain.
    allow: HashSet<Box<str>>,
}

thread_local! {
    static ENV: RefCell<Option<ICoreWebView2Environment>> = const { RefCell::new(None) };
    static DATA_DIR: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    static MAIN_HWND: Cell<isize> = const { Cell::new(0) };
    static NEXT_STORE: Cell<u32> = const { Cell::new(1) };
    static VIEWS: RefCell<HashMap<usize, ViewState>> = RefCell::new(HashMap::new());
    static RULES: RefCell<Vec<(&'static str, Rc<Rules>)>> = const { RefCell::new(Vec::new()) };
    static BLOCKED: RefCell<HashMap<u32, u32>> = RefCell::new(HashMap::new());
    static BLOCKED_HOSTS: RefCell<HashMap<String, u32>> = RefCell::new(HashMap::new());
    static APPROVED: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static PROFILES: RefCell<Vec<(String, ICoreWebView2Profile)>> = const { RefCell::new(Vec::new()) };
    static SCHEME: Cell<COREWEBVIEW2_PREFERRED_COLOR_SCHEME> = const { Cell::new(COREWEBVIEW2_PREFERRED_COLOR_SCHEME_AUTO) };
    static ON_CMD: RefCell<Option<Rc<dyn Fn(String)>>> = const { RefCell::new(None) };
    static ON_BLOCKED: RefCell<Option<Rc<dyn Fn()>>> = const { RefCell::new(None) };
    static ON_TLS: RefCell<Option<Rc<TlsHandler>>> = const { RefCell::new(None) };
    static ON_NET_ERROR: RefCell<Option<Rc<NetHandler>>> = const { RefCell::new(None) };
    static ON_CRASH: RefCell<Option<Rc<dyn Fn(u32)>>> = const { RefCell::new(None) };
    static ON_FULLSCREEN: RefCell<Option<Rc<dyn Fn(u32, bool)>>> = const { RefCell::new(None) };
}

fn key(v: &WebView) -> usize {
    v.webview().as_raw() as usize
}

fn with_view<R>(k: usize, f: impl FnOnce(&mut ViewState) -> R) -> R {
    VIEWS.with(|m| f(m.borrow_mut().entry(k).or_default()))
}

fn handler<T: ?Sized>(key: &'static std::thread::LocalKey<RefCell<Option<Rc<T>>>>) -> Option<Rc<T>> {
    key.with(|h| h.borrow().clone())
}

fn pwstr(f: impl FnOnce(&mut PWSTR) -> windows::core::Result<()>) -> String {
    let mut p = PWSTR::null();
    if f(&mut p).is_err() {
        return String::new();
    }
    take_pwstr(p)
}

/// Per-run user data folder in `%TEMP%`. WebView2 needs one for runtime-level state; browsing
/// data never lands there (InPrivate profiles). Removed on exit, leftovers of dead runs on start.
fn data_dir() -> PathBuf {
    DATA_DIR.with(|d| {
        d.borrow_mut()
            .get_or_insert_with(|| {
                let nanos = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |t| t.subsec_nanos());
                std::env::temp_dir().join(format!("{DATA_PREFIX}{}-{nanos:08x}", std::process::id()))
            })
            .clone()
    })
}

/// Folders of earlier runs whose process is gone (a running instance keeps its own).
fn remove_stale_data() {
    let Ok(dir) = std::fs::read_dir(std::env::temp_dir()) else { return };
    let own = data_dir();
    for e in dir.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Some(pid) = name.strip_prefix(DATA_PREFIX).and_then(|r| r.split('-').next()?.parse::<u32>().ok()) else {
            continue;
        };
        let alive = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
            .map(|h| unsafe { CloseHandle(h) })
            .is_ok();
        if !alive && e.path() != own {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

fn create_env() -> windows::core::Result<ICoreWebView2Environment> {
    let options = CoreWebView2EnvironmentOptions::default();
    unsafe {
        options.set_additional_browser_arguments(BROWSER_ARGS.to_string());
        options.set_are_browser_extensions_enabled(false);
        options.set_enable_tracking_prevention(true);
        options.set_allow_single_sign_on_using_os_primary_account(false);
        // Crash dumps stay in the (deleted) user data folder instead of going to Microsoft.
        options.set_is_custom_crash_reporting_enabled(true);
    }
    let (tx, rx) = mpsc::channel();
    let done = CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(move |hr, env| {
        let _ = tx.send(hr.and_then(|()| env.ok_or_else(|| E_POINTER.into())));
        Ok(())
    }));
    let dir = HSTRING::from(data_dir().as_os_str());
    unsafe {
        CreateCoreWebView2EnvironmentWithOptions(
            PCWSTR::null(),
            &dir,
            &ICoreWebView2EnvironmentOptions::from(options),
            &done,
        )?;
    }
    wait_with_pump(rx).map_err(|_| windows::core::Error::from(E_FAIL))?
}

/// The shared environment (created on first use; recreated after its browser process died).
fn env() -> ICoreWebView2Environment {
    ENV.with(|e| {
        e.borrow_mut()
            .get_or_insert_with(|| create_env().expect("Microsoft Edge WebView2 Runtime is not available"))
            .clone()
    })
}

fn hwnd() -> HWND {
    HWND(MAIN_HWND.with(Cell::get) as _)
}

/// Hidden InPrivate view of `profile` in the main window.
fn root_view(profile: &str) -> windows::core::Result<ICoreWebView2Controller> {
    let env10: ICoreWebView2Environment10 = env().cast()?;
    let (tx, rx) = mpsc::channel();
    let done = CreateCoreWebView2ControllerCompletedHandler::create(Box::new(move |hr, c| {
        let _ = tx.send(hr.and_then(|()| c.ok_or_else(|| E_POINTER.into())));
        Ok(())
    }));
    unsafe {
        let opts = env10.CreateCoreWebView2ControllerOptions()?;
        opts.SetIsInPrivateModeEnabled(true)?;
        opts.SetProfileName(&HSTRING::from(profile))?;
        env10.CreateCoreWebView2ControllerWithOptions(hwnd(), &opts, &done)?;
    }
    let c = wait_with_pump(rx).map_err(|_| windows::core::Error::from(E_FAIL))??;
    unsafe {
        c.SetIsVisible(false)?;
        c.SetBounds(RECT::default())?;
        if let Ok(p) = c.CoreWebView2().and_then(|w| w.cast::<ICoreWebView2_13>()).and_then(|w| w.Profile()) {
            track_profile(profile, p);
        }
    }
    Ok(c)
}

fn make_store() -> Store {
    let n = NEXT_STORE.with(|s| s.replace(s.get() + 1));
    let profile = format!("crow-{n}");
    let root = root_view(&profile).map_err(|e| eprintln!("webview2 store: {e}")).ok();
    Rc::new(StoreInner { profile, root })
}

/// Fresh in-memory profile for the vault-backed jar of an unlocked session.
pub fn new_store() -> Store {
    make_store()
}

/// Separate in-memory profile of a container (own cookies, storage, cache).
pub fn new_container_store() -> Store {
    make_store()
}

/// Nothing extra: the root view closes when the store's last handle drops (right after this),
/// the profile's in-memory data with its last tab.
pub fn drop_container_store(_store: &Store) {}

/// Chromium's own picture-in-picture window (`toggle_pip`).
pub const PIP: bool = true;

pub fn config(store: &Store, _adblock: bool) -> Config {
    Config { profile: store.profile.clone() }
}

fn profile_of(w: &ICoreWebView2) -> Option<String> {
    let p = unsafe { w.cast::<ICoreWebView2_13>().and_then(|w| w.Profile()) }.ok()?;
    Some(pwstr(|s| unsafe { p.ProfileName(s) }))
}

/// window.open target: same environment and profile as the opener (keeps `window.opener`).
#[allow(clippy::needless_pass_by_value)] // same signature as the other backends
pub fn popup_config(features: NewWindowFeatures) -> Config {
    Config { profile: profile_of(&features.opener.webview).unwrap_or_else(|| "crow-1".into()) }
}

/// Tab views (including adopted popups); the browser UI uses `ui_builder`. Browser accelerator
/// keys are off (no built-in find bar, print, devtools); Crow's shortcuts come from `init_window`.
/// `page_js`: the page-world shim (`adblock::defuser`), injected into every frame.
pub fn builder<'a>(config: Config, page_js: String) -> WebViewBuilder<'a> {
    WebViewBuilder::new()
        .with_environment(env())
        .with_incognito(true)
        .with_profile_name(config.profile)
        .with_browser_accelerator_keys(false)
        .with_initialization_script_for_main_only(page_js, false)
}

pub fn ui_builder<'a>() -> WebViewBuilder<'a> {
    WebViewBuilder::new()
        .with_environment(env())
        .with_incognito(true)
        .with_profile_name(UI_PROFILE)
        .with_browser_accelerator_keys(false)
        .with_default_context_menus(false)
}

fn track_profile(name: &str, p: ICoreWebView2Profile) {
    let _ = unsafe { p.SetPreferredColorScheme(SCHEME.with(Cell::get)) };
    PROFILES.with(|l| {
        let mut l = l.borrow_mut();
        if !l.iter().any(|(n, _)| n == name) {
            l.push((name.to_string(), p));
        }
    });
}

/// Every view: no native script dialogs (`alert` & co. are dismissed), no host objects, no
/// devtools, no password / form autofill. Shortcuts are read from the controller.
fn harden(v: &WebView) -> windows::core::Result<()> {
    let w = v.webview();
    unsafe {
        let s = w.Settings()?;
        s.SetAreDefaultScriptDialogsEnabled(false)?;
        s.SetAreHostObjectsAllowed(false)?;
        s.SetAreDevToolsEnabled(false)?;
        s.SetIsStatusBarEnabled(false)?;
        if let Ok(s4) = s.cast::<ICoreWebView2Settings4>() {
            s4.SetIsPasswordAutosaveEnabled(false)?;
            s4.SetIsGeneralAutofillEnabled(false)?;
        }
        if let Some(name) = profile_of(&w)
            && let Ok(p) = w.cast::<ICoreWebView2_13>().and_then(|w| w.Profile())
        {
            track_profile(&name, p);
        }
        let mut token = 0i64;
        v.controller().add_AcceleratorKeyPressed(
            &AcceleratorKeyPressedEventHandler::create(Box::new(|_, args| {
                let Some(args) = args else { return Ok(()) };
                let mut kind = COREWEBVIEW2_KEY_EVENT_KIND::default();
                args.KeyEventKind(&mut kind)?;
                if kind != COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN && kind != COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN {
                    return Ok(());
                }
                let mut vk = 0u32;
                args.VirtualKey(&mut vk)?;
                let Some(cmd) = shortcut(vk) else { return Ok(()) };
                args.SetHandled(true)?;
                let mut status = COREWEBVIEW2_PHYSICAL_KEY_STATUS::default();
                args.PhysicalKeyStatus(&mut status)?;
                if !status.WasKeyDown.as_bool()
                    && let Some(f) = handler(&ON_CMD)
                {
                    f(cmd.to_string());
                }
                Ok(())
            })),
            &mut token,
        )?;
    }
    Ok(())
}

pub fn attach(b: WebViewBuilder, rect: Rect, window: &Window) -> wry::Result<WebView> {
    let v = b.with_bounds(rect).build_as_child(window)?;
    if let Err(e) = harden(&v) {
        eprintln!("webview2 settings: {e}");
    }
    Ok(v)
}

/// Built-in page `html` under the document address `base` (see `app::INTERNAL_BASE`): served
/// from memory by `WebResourceRequested` (a secure context, no request leaves the browser).
pub fn load_internal(v: &WebView, html: &str, base: &str) {
    let k = key(v);
    let first = with_view(k, |s| {
        let first = s.internal_base.is_empty();
        s.internal = Some(html.into());
        s.internal_base = base.to_string();
        s.internal_pending = true;
        first
    });
    if first {
        let filter = HSTRING::from(format!("{base}*"));
        let _ = unsafe { v.webview().AddWebResourceRequestedFilter(&filter, COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL) };
    }
    let _ = v.load_url(base);
}

/// DevTools protocol call on `w`; `done` gets the parsed result (None on failure).
fn cdp(w: &ICoreWebView2, method: &str, params: &Value, done: impl FnOnce(Option<Value>) + 'static) {
    let done = Rc::new(Cell::new(Some(done)));
    let d = done.clone();
    let cb = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(move |hr, result| {
        if let Some(f) = d.take() {
            f(hr.ok().and_then(|()| serde_json::from_str(&result).ok()));
        }
        Ok(())
    }));
    let sent = unsafe {
        w.CallDevToolsProtocolMethod(&HSTRING::from(method), &HSTRING::from(params.to_string()), &cb)
    };
    if sent.is_err()
        && let Some(f) = done.take()
    {
        f(None);
    }
}

/// Runs `js` at document start in every frame of `v`, in Crow's isolated JS world (see
/// `eval_isolated`). Called before the view's first load.
pub fn add_isolated_script(v: &WebView, js: &str) {
    let w = v.webview();
    cdp(&w, "Page.enable", &json!({}), |_| {});
    cdp(&w, "Page.addScriptToEvaluateOnNewDocument", &json!({ "source": js, "worldName": WORLD }), |_| {});
}

/// `js` in the main frame's isolated world; `gesture`: counts as a user activation.
fn eval_world(v: &WebView, js: &str, gesture: bool, done: impl FnOnce(String) + 'static) {
    let (w, k) = (v.webview(), key(v));
    let js = js.to_string();
    let run = move |w: ICoreWebView2, frame: String| {
        let w2 = w.clone();
        cdp(&w, "Page.createIsolatedWorld", &json!({ "frameId": frame, "worldName": WORLD }), move |r| {
            let Some(ctx) = r.and_then(|r| r["executionContextId"].as_i64()) else {
                with_view(k, |s| s.frame = None);
                return done(String::new());
            };
            let params = json!({
                "expression": js, "contextId": ctx, "returnByValue": true,
                "userGesture": gesture, "awaitPromise": gesture,
            });
            cdp(&w2, "Runtime.evaluate", &params, move |r| {
                done(r.and_then(|r| r["result"]["value"].as_str().map(str::to_string)).unwrap_or_default());
            });
        });
    };
    if let Some(frame) = with_view(k, |s| s.frame.clone()) {
        return run(w, frame);
    }
    let w2 = w.clone();
    cdp(&w, "Page.getFrameTree", &json!({}), move |r| {
        let Some(frame) = r.and_then(|r| r["frameTree"]["frame"]["id"].as_str().map(str::to_string)) else {
            return;
        };
        with_view(k, |s| s.frame = Some(frame.clone()));
        run(w2, frame);
    });
}

/// Runs `js` in the main frame in Crow's own JS world: same DOM, but globals set there are
/// invisible to the page's scripts. `done` gets the result if it is a string, else "".
pub fn eval_isolated(v: &WebView, js: &str, done: impl FnOnce(String) + 'static) {
    eval_world(v, js, false, done);
}

/// Picture-in-picture toggle: the largest playing video, else the largest one, in Chromium's
/// floating always-on-top window (MSE / blob streams stay in their page). Needs a user
/// activation, granted to this one evaluation only. `done`: "pip" | "inline" | "none".
const PIP_JS: &str = "(async () => { if (document.pictureInPictureElement) { \
await document.exitPictureInPicture(); return 'inline'; } \
const vs = Array.from(document.querySelectorAll('video')).filter(v => v.readyState > 0 && !v.disablePictureInPicture); \
const score = (v) => v.clientWidth * v.clientHeight + (v.paused ? 0 : 1e9); \
const v = vs.sort((a, b) => score(b) - score(a))[0]; \
if (!v || !document.pictureInPictureEnabled) return 'none'; \
try { await v.requestPictureInPicture(); return 'pip'; } catch (e) { return 'none'; } })()";

pub fn toggle_pip(v: &WebView, done: impl FnOnce(String) + 'static) {
    eval_world(v, PIP_JS, true, done);
}

/// Left mouse button is held right now (a "drag" IPC can arrive after a quick click).
pub fn left_button_down() -> bool {
    (unsafe { GetAsyncKeyState(i32::from(VK_LBUTTON.0)) }) < 0
}

pub fn set_bounds(v: &WebView, rect: Rect) {
    let _ = v.set_bounds(rect);
}

pub fn popup_response(v: &WebView) -> NewWindowResponse {
    NewWindowResponse::Create { webview: v.webview() }
}

/// Content blocker of `v` on/off. Requests of every frame and worker are routed through
/// `WebResourceRequested` (see `watch`) once it was on.
pub fn set_rules(v: &WebView, on: bool) {
    let k = key(v);
    let install = with_view(k, |s| {
        s.rules = on;
        let install = on && !s.filtered;
        s.filtered |= on;
        install
    });
    if install {
        let w = v.webview();
        unsafe {
            let all = w.cast::<ICoreWebView2_22>().and_then(|w22| {
                w22.AddWebResourceRequestedFilterWithRequestSourceKinds(
                    w!("*"),
                    COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
                    COREWEBVIEW2_WEB_RESOURCE_REQUEST_SOURCE_KINDS_ALL,
                )
            });
            if all.is_err() {
                let _ = w.AddWebResourceRequestedFilter(w!("*"), COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL);
            }
        }
    }
}

#[derive(Deserialize)]
struct CbRuleIn {
    action: CbActionIn,
    trigger: CbTriggerIn,
}

#[derive(Deserialize)]
struct CbActionIn {
    #[serde(rename = "type")]
    typ: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
struct CbTriggerIn {
    url_filter: String,
    if_domain: Option<Vec<String>>,
    unless_domain: Option<Vec<String>>,
    if_top_url: Option<Vec<String>>,
    unless_top_url: Option<Vec<String>>,
    resource_type: Option<Vec<String>>,
    #[serde(default)]
    load_type: Vec<String>,
}

/// `url-filter` the adblock crate emits for `||domain^`.
const HOST_ANCHOR: &str = "^[^:]+:(//)?([^/]+\\.)?";

fn rule_domain(filter: &str) -> Option<Box<str>> {
    let d = filter.strip_prefix(HOST_ANCHOR)?.replace("\\.", ".").to_ascii_lowercase();
    let ok = d.contains('.') && d.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-');
    ok.then(|| d.into_boxed_str())
}

fn type_bit(t: &str) -> u16 {
    match t {
        "document" => 1,
        "image" => 2,
        "style-sheet" => 4,
        "script" => 8,
        "font" => 16,
        "raw" => 32,
        "svg-document" => 64,
        "media" => 128,
        "popup" => 256,
        _ => 0,
    }
}

fn context_bit(c: COREWEBVIEW2_WEB_RESOURCE_CONTEXT) -> u16 {
    match c {
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_DOCUMENT => 1,
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_IMAGE => 2,
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_STYLESHEET => 4,
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_SCRIPT => 8,
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_FONT => 16,
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_MEDIA | COREWEBVIEW2_WEB_RESOURCE_CONTEXT_TEXT_TRACK => 128,
        _ => 32,
    }
}

/// WebView2 has no content-blocker engine: the WebKit rule JSON is reduced to its domain rules
/// (`||domain^` with optional resource types / `$third-party`, and unconditional `@@||domain^`
/// exceptions), which are matched for every request before it leaves the browser. Path patterns,
/// per-site conditions and cosmetic rules are WebKit-only.
fn parse_rules(json: &str) -> Option<Rules> {
    let list: Vec<CbRuleIn> = serde_json::from_str(json).ok()?;
    let mut r = Rules::default();
    for rule in list {
        let t = rule.trigger;
        let conditional =
            t.if_domain.is_some() || t.unless_domain.is_some() || t.if_top_url.is_some() || t.unless_top_url.is_some();
        let Some(domain) = rule_domain(&t.url_filter).filter(|_| !conditional) else { continue };
        let mask = t.resource_type.as_deref().map_or(0, |ts| ts.iter().fold(0, |m, t| m | type_bit(t)));
        let third = t.load_type.iter().any(|l| l == "third-party");
        let first = t.load_type.iter().any(|l| l == "first-party");
        match rule.action.typ.as_str() {
            "block" if !first && (t.resource_type.is_none() || mask != 0) => {
                r.block.entry(domain).or_default().push((mask, third));
            }
            "ignore-previous-rules" if t.resource_type.is_none() && t.load_type.is_empty() => {
                r.allow.insert(domain);
            }
            _ => {}
        }
    }
    Some(r)
}

fn rules_file(id: &str) -> PathBuf {
    crate::settings::data_dir().join(format!("wv2-{id}.rules"))
}

fn save_rules(id: &str, r: &Rules) {
    let mut out = String::new();
    for d in &r.allow {
        let _ = writeln!(out, "a {d}");
    }
    for (d, list) in &r.block {
        for (mask, third) in list {
            let _ = writeln!(out, "b {mask} {} {d}", u8::from(*third));
        }
    }
    let _ = std::fs::create_dir_all(crate::settings::data_dir());
    let _ = std::fs::write(rules_file(id), out);
}

fn read_rules(id: &str) -> Option<Rules> {
    let text = std::fs::read_to_string(rules_file(id)).ok()?;
    let mut r = Rules::default();
    for l in text.lines() {
        let mut it = l.split(' ');
        match (it.next(), it.next(), it.next(), it.next()) {
            (Some("a"), Some(d), None, None) => {
                r.allow.insert(d.into());
            }
            (Some("b"), Some(mask), Some(third), Some(d)) => {
                r.block.entry(d.into()).or_default().push((mask.parse().ok()?, third == "1"));
            }
            _ => return None,
        }
    }
    Some(r)
}

fn keep_rules(id: &'static str, r: Rules) {
    RULES.with(|l| {
        let mut l = l.borrow_mut();
        l.retain(|(i, _)| *i != id);
        l.push((id, Rc::new(r)));
    });
}

/// Compiles rule list `id` (domain rules, cached in `data_dir()` for `load_rules`).
#[allow(clippy::needless_pass_by_value)]
pub fn compile_rules(id: &'static str, json: String, done: impl Fn() + 'static) {
    let Some(r) = parse_rules(&json) else {
        return eprintln!("adblock: list {id} is not valid content-blocker JSON");
    };
    drop(json);
    save_rules(id, &r);
    keep_rules(id, r);
    done();
}

/// Loads the previously compiled list `id`; `fail` if the cache no longer has it.
pub fn load_rules(id: &'static str, ok: impl Fn() + 'static, fail: impl Fn() + 'static) {
    match read_rules(id) {
        Some(r) => {
            keep_rules(id, r);
            ok();
        }
        None => fail(),
    }
}

/// Disabled list: forgotten here and removed from the cache.
pub fn drop_rules(id: &'static str) {
    RULES.with(|l| l.borrow_mut().retain(|(i, _)| *i != id));
    let _ = std::fs::remove_file(rules_file(id));
}

/// Registrable part of `host` (heuristic: two labels, three under short second-level labels
/// such as `co.uk`).
fn site_of(host: &str) -> &str {
    let mut dots = host.rmatch_indices('.').map(|(i, _)| i);
    let (Some(a), b) = (dots.next(), dots.next()) else { return host };
    let Some(b) = b else { return host };
    let second = &host[b + 1..a];
    let keep = if second.len() <= 3 && host.len() - a - 1 == 2 { dots.next().map_or(0, |c| c + 1) } else { b + 1 };
    &host[keep..]
}

fn host_of(uri: &str) -> &str {
    let rest = uri.split_once("://").map_or("", |(_, r)| r);
    let auth = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = auth.rsplit('@').next().unwrap_or("");
    if host.starts_with('[') { host } else { host.split(':').next().unwrap_or("") }
}

fn suffixes(host: &str) -> impl Iterator<Item = &str> {
    std::iter::once(host).chain(host.match_indices('.').map(|(i, _)| &host[i + 1..]))
}

/// Exceptions only cancel rules of their own list (as in WebKit).
fn is_blocked(host: &str, top: &str, kind: u16) -> bool {
    let third = !top.is_empty() && site_of(host) != site_of(top);
    RULES.with(|l| {
        l.borrow().iter().any(|(_, r)| {
            !suffixes(host).any(|s| r.allow.contains(s))
                && suffixes(host).any(|s| {
                    r.block.get(s).is_some_and(|rs| rs.iter().any(|&(m, tp)| (m == 0 || m & kind != 0) && (!tp || third)))
                })
        })
    })
}

fn count_blocked(tab: u32, host: &str) {
    BLOCKED_HOSTS.with(|b| *b.borrow_mut().entry(host.to_string()).or_default() += 1);
    let first = BLOCKED.with(|b| {
        let mut b = b.borrow_mut();
        let first = b.is_empty();
        *b.entry(tab).or_default() += 1;
        first
    });
    if first && let Some(f) = handler(&ON_BLOCKED) {
        f();
    }
}

enum Answer {
    Page(Rc<str>),
    Status(i32, &'static str),
}

fn respond(args: &ICoreWebView2WebResourceRequestedEventArgs, answer: Answer) -> windows::core::Result<()> {
    let e = env();
    let res = unsafe {
        match answer {
            Answer::Page(html) => {
                let stream = SHCreateMemStream(Some(html.as_bytes()));
                e.CreateWebResourceResponse(
                    stream.as_ref(),
                    200,
                    w!("OK"),
                    w!("Content-Type: text/html; charset=utf-8\r\nCache-Control: no-store"),
                )?
            }
            Answer::Status(code, reason) => e.CreateWebResourceResponse(None, code, &HSTRING::from(reason), w!(""))?,
        }
    };
    unsafe { args.SetResponse(&res) }
}

/// Built-in page, 404 for anything else under its address, or the content blocker's verdict.
fn on_request(k: usize, tab: u32, args: &ICoreWebView2WebResourceRequestedEventArgs) -> windows::core::Result<()> {
    let uri = pwstr(|p| unsafe { args.Request()?.Uri(p) });
    let mut ctx = COREWEBVIEW2_WEB_RESOURCE_CONTEXT::default();
    unsafe { args.ResourceContext(&mut ctx)? };
    let (answer, top, rules) = with_view(k, |s| {
        let internal = !s.internal_base.is_empty() && uri.starts_with(&s.internal_base);
        let answer = match &s.internal {
            Some(html) if internal && uri == s.internal_base && ctx == COREWEBVIEW2_WEB_RESOURCE_CONTEXT_DOCUMENT => {
                Some(Answer::Page(html.clone()))
            }
            _ if internal => Some(Answer::Status(404, "Not Found")),
            _ => None,
        };
        (answer, s.top_host.clone(), s.rules)
    });
    if let Some(a) = answer {
        return respond(args, a);
    }
    let host = host_of(&uri).to_ascii_lowercase();
    if !rules || host.is_empty() || !(uri.starts_with("http:") || uri.starts_with("https:") || uri.starts_with("ws")) {
        return Ok(());
    }
    if is_blocked(&host, &top, context_bit(ctx)) {
        count_blocked(tab, &host);
        return respond(args, Answer::Status(403, "Blocked"));
    }
    Ok(())
}

pub const BLOCK_STATS: bool = true;

/// Called once per batch of blocked loads (collect them with `take_blocked`).
pub fn on_blocked(f: impl Fn() + 'static) {
    ON_BLOCKED.with(|o| *o.borrow_mut() = Some(Rc::new(f)));
}

/// Called when a navigation is refused because of the server certificate (tab, URL, reasons).
pub fn on_tls_error(f: impl Fn(u32, String, Vec<&'static str>) + 'static) {
    ON_TLS.with(|o| *o.borrow_mut() = Some(Rc::new(f)));
}

/// The main page of a tab could not be loaded (network): `f` gets tab, URL and reason.
pub fn on_net_error(f: impl Fn(u32, String, &'static str) + 'static) {
    ON_NET_ERROR.with(|o| *o.borrow_mut() = Some(Rc::new(f)));
}

/// Render process of a tab ended or hangs (crash, out of memory, WebGL hang): `f` gets the tab.
pub fn on_crash(f: impl Fn(u32) + 'static) {
    ON_CRASH.with(|o| *o.borrow_mut() = Some(Rc::new(f)));
}

/// Element fullscreen (video players): WebView2 only reports it, the app fullscreens the window.
pub fn on_fullscreen(f: impl Fn(u32, bool) + 'static) {
    ON_FULLSCREEN.with(|o| *o.borrow_mut() = Some(Rc::new(f)));
}

fn tls_reason(s: COREWEBVIEW2_WEB_ERROR_STATUS) -> &'static str {
    match s {
        COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_COMMON_NAME_IS_INCORRECT => "cert_err_bad_identity",
        COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_EXPIRED => "cert_err_expired",
        COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_REVOKED => "cert_err_revoked",
        COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_IS_INVALID => "cert_err_untrusted",
        COREWEBVIEW2_WEB_ERROR_STATUS_CLIENT_CERTIFICATE_CONTAINS_ERRORS => "cert_err_client_cert",
        _ => "cert_err_generic",
    }
}

/// Reason (`locales.json` key) of a failed main-frame navigation, or None when it is not an
/// error to show (cancelled, certificate: reported by `ServerCertificateErrorDetected`).
fn net_reason(s: COREWEBVIEW2_WEB_ERROR_STATUS) -> Option<&'static str> {
    Some(match s {
        COREWEBVIEW2_WEB_ERROR_STATUS_DISCONNECTED => "net_offline",
        COREWEBVIEW2_WEB_ERROR_STATUS_HOST_NAME_NOT_RESOLVED => "net_dns",
        COREWEBVIEW2_WEB_ERROR_STATUS_CANNOT_CONNECT | COREWEBVIEW2_WEB_ERROR_STATUS_SERVER_UNREACHABLE => "net_refused",
        COREWEBVIEW2_WEB_ERROR_STATUS_TIMEOUT => "net_timeout",
        COREWEBVIEW2_WEB_ERROR_STATUS_CONNECTION_ABORTED | COREWEBVIEW2_WEB_ERROR_STATUS_CONNECTION_RESET => "net_lost",
        COREWEBVIEW2_WEB_ERROR_STATUS_ERROR_HTTP_INVALID_SERVER_RESPONSE
        | COREWEBVIEW2_WEB_ERROR_STATUS_REDIRECT_FAILED
        | COREWEBVIEW2_WEB_ERROR_STATUS_UNEXPECTED_ERROR => "net_generic",
        _ => return None,
    })
}

/// `Security.visibleSecurityStateChanged` (DevTools protocol): Chromium's own verdict and the
/// certificate chain (base64 DER, leaf first) of the committed page.
fn security_state(json: &str) -> Option<(Tls, Vec<Vec<u8>>)> {
    let v: Value = serde_json::from_str(json).ok()?;
    let s = &v["visibleSecurityState"];
    let issues: Vec<&str> = s["securityStateIssueIds"].as_array().map_or_else(Vec::new, |a| a.iter().filter_map(Value::as_str).collect());
    let tls = match s["securityState"].as_str()? {
        "secure" => Tls::Secure,
        _ if issues.iter().any(|i| i.contains("mixed")) => Tls::Mixed,
        _ => Tls::Unverified,
    };
    let chain = s["certificateSecurityState"]["certificate"]
        .as_array()
        .map_or_else(Vec::new, |a| a.iter().filter_map(Value::as_str).filter_map(base64).take(8).collect());
    Some((tls, chain))
}

fn base64(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    };
    let (mut out, mut acc, mut bits) = (Vec::with_capacity(s.len() * 3 / 4), 0u32, 0);
    for c in s.bytes().filter(|&c| c != b'=') {
        acc = (acc << 6) | u32::from(val(c)?);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xff) as u8);
        }
    }
    Some(out)
}

/// Tab hardening and reporting of `v` as tab `tab`: no web messages (`window.chrome.webview`
/// cannot reach Crow), no built-in error pages; certificate errors refuse the load, network
/// errors / render process failures / fullscreen changes / blocked requests go to the app;
/// Chromium's security state of each page is kept for `tls` / `cert_chain`.
pub fn watch(v: &WebView, tab: u32) {
    if let Err(e) = watch_inner(v, tab) {
        eprintln!("webview2 tab {tab}: {e}");
    }
}

fn watch_inner(v: &WebView, tab: u32) -> windows::core::Result<()> {
    let (w, k) = (v.webview(), key(v));
    with_view(k, |s| s.tab = Some(tab));
    let mut token = 0i64;
    unsafe {
        let s = w.Settings()?;
        s.SetIsWebMessageEnabled(false)?;
        s.SetIsBuiltInErrorPageEnabled(false)?;

        w.add_NavigationStarting(
            &NavigationStartingEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                let uri = pwstr(|p| args.Uri(p));
                let refuse = with_view(k, |s| {
                    let internal = !s.internal_base.is_empty() && uri.starts_with(&s.internal_base);
                    if internal && !std::mem::take(&mut s.internal_pending) {
                        return true; // only Crow opens built-in pages
                    }
                    if !internal {
                        s.internal = None;
                    }
                    s.top_host = host_of(&uri).to_ascii_lowercase();
                    s.nav_uri = uri;
                    s.quiet = false;
                    s.cert_refused = None;
                    s.tls = None;
                    s.chain.clear();
                    false
                });
                if refuse {
                    args.SetCancel(true)?;
                }
                Ok(())
            })),
            &mut token,
        )?;

        w.add_NavigationCompleted(
            &NavigationCompletedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                let mut ok = windows::core::BOOL::default();
                args.IsSuccess(&mut ok)?;
                let mut status = COREWEBVIEW2_WEB_ERROR_STATUS::default();
                args.WebErrorStatus(&mut status)?;
                let (uri, quiet, cert) =
                    with_view(k, |s| (s.nav_uri.clone(), std::mem::take(&mut s.quiet), s.cert_refused.take()));
                if let (Some(reason), Some(f)) = (cert, handler(&ON_TLS)) {
                    f(tab, uri, vec![reason]);
                    return Ok(());
                }
                if ok.as_bool() || quiet || !uri.starts_with("http") {
                    return Ok(());
                }
                if let (Some(reason), Some(f)) = (net_reason(status), handler(&ON_NET_ERROR)) {
                    f(tab, uri, reason);
                }
                Ok(())
            })),
            &mut token,
        )?;

        if let Ok(w14) = w.cast::<ICoreWebView2_14>() {
            w14.add_ServerCertificateErrorDetected(
                &ServerCertificateErrorDetectedEventHandler::create(Box::new(move |_, args| {
                    let Some(args) = args else { return Ok(()) };
                    // Never an "ignore" fallback: the load is refused.
                    args.SetAction(COREWEBVIEW2_SERVER_CERTIFICATE_ERROR_ACTION_CANCEL)?;
                    let uri = pwstr(|p| args.RequestUri(p));
                    let mut status = COREWEBVIEW2_WEB_ERROR_STATUS::default();
                    args.ErrorStatus(&mut status)?;
                    with_view(k, |s| {
                        if s.nav_uri == uri {
                            s.quiet = true;
                            s.cert_refused = Some(tls_reason(status));
                        }
                    });
                    Ok(())
                })),
                &mut token,
            )?;
        }

        w.add_ProcessFailed(
            &ProcessFailedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
                args.ProcessFailedKind(&mut kind)?;
                let why = match kind {
                    COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED => "exited",
                    COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE => "unresponsive",
                    COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED => {
                        // Every view of the environment is gone; new views get a new one.
                        ENV.with(|e| e.borrow_mut().take());
                        "browser exited"
                    }
                    _ => return Ok(()),
                };
                let report = with_view(k, |s| {
                    s.frame = None;
                    let fresh = s.crashed_at.is_none_or(|t| t.elapsed() >= CRASH_DEBOUNCE);
                    if fresh {
                        s.crashed_at = Some(Instant::now());
                    }
                    fresh
                });
                if report {
                    eprintln!("web process of tab {tab} {why}");
                    if let Some(f) = handler(&ON_CRASH) {
                        f(tab);
                    }
                }
                Ok(())
            })),
            &mut token,
        )?;

        w.add_ContainsFullScreenElementChanged(
            &ContainsFullScreenElementChangedEventHandler::create(Box::new(move |w, _| {
                let Some(w) = w else { return Ok(()) };
                let mut on = windows::core::BOOL::default();
                w.ContainsFullScreenElement(&mut on)?;
                if let Some(f) = handler(&ON_FULLSCREEN) {
                    f(tab, on.as_bool());
                }
                Ok(())
            })),
            &mut token,
        )?;

        if let Ok(w4) = w.cast::<ICoreWebView2_4>() {
            w4.add_DownloadStarting(
                &DownloadStartingEventHandler::create(Box::new(move |_, _| {
                    with_view(k, |s| s.quiet = true);
                    Ok(())
                })),
                &mut token,
            )?;
        }

        w.add_WebResourceRequested(
            &WebResourceRequestedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                on_request(k, tab, &args)
            })),
            &mut token,
        )?;

        cdp(&w, "Security.enable", &json!({}), |_| {});
        let receiver = w.GetDevToolsProtocolEventReceiver(w!("Security.visibleSecurityStateChanged"))?;
        receiver.add_DevToolsProtocolEventReceived(
            &DevToolsProtocolEventReceivedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                let json = pwstr(|p| args.ParameterObjectAsJson(p));
                if let Some((tls, chain)) = security_state(&json) {
                    with_view(k, |s| {
                        s.tls = Some(tls);
                        s.chain = chain;
                    });
                }
                Ok(())
            })),
            &mut token,
        )?;
        with_view(k, |s| s.security = Some(receiver));
    }
    Ok(())
}

/// Tab is closing/unloading: its state goes and late callbacks are ignored.
pub fn forget(v: &WebView) {
    let k = key(v);
    VIEWS.with(|m| m.borrow_mut().remove(&k));
}

/// TLS state of the committed page as verified by Chromium (DevTools security state). Until
/// that arrives: secure, because certificate errors always refuse the load (never from the URL
/// alone: a page that failed its certificate never commits).
pub fn tls(v: &WebView) -> Tls {
    with_view(key(v), |s| s.tls.unwrap_or(if s.nav_uri.starts_with("https:") { Tls::Secure } else { Tls::Unverified }))
}

/// Blocked loads per tab since the last call.
pub fn take_blocked() -> Vec<(u32, u32)> {
    BLOCKED.with(|b| b.borrow_mut().drain().collect())
}

/// Blocked loads per domain since the last call.
pub fn take_blocked_hosts() -> Vec<(String, u32)> {
    BLOCKED_HOSTS.with(|b| b.borrow_mut().drain().collect())
}

fn cookie_manager(store: &Store) -> Option<ICoreWebView2CookieManager> {
    let root = store.root.as_ref()?;
    unsafe { root.CoreWebView2().and_then(|w| w.cast::<ICoreWebView2_2>()).and_then(|w| w.CookieManager()) }.ok()
}

fn cookie_rec(c: &ICoreWebView2Cookie) -> windows::core::Result<CookieRec> {
    unsafe {
        let (mut secure, mut http_only, mut session) = Default::default();
        c.IsSecure(&mut secure)?;
        c.IsHttpOnly(&mut http_only)?;
        c.IsSession(&mut session)?;
        let mut same_site = COREWEBVIEW2_COOKIE_SAME_SITE_KIND::default();
        c.SameSite(&mut same_site)?;
        let mut expires = 0f64;
        c.Expires(&mut expires)?;
        Ok(CookieRec {
            name: pwstr(|p| c.Name(p)),
            value: pwstr(|p| c.Value(p)),
            domain: pwstr(|p| c.Domain(p)),
            path: pwstr(|p| c.Path(p)),
            secure: secure.as_bool(),
            http_only: http_only.as_bool(),
            same_site: match same_site {
                COREWEBVIEW2_COOKIE_SAME_SITE_KIND_LAX => Some("lax".into()),
                COREWEBVIEW2_COOKIE_SAME_SITE_KIND_STRICT => Some("strict".into()),
                _ => None,
            },
            expires: (!session.as_bool()).then_some(expires),
        })
    }
}

pub fn export_cookies(store: &Store, done: impl Fn(Vec<CookieRec>) + 'static) {
    let Some(cm) = cookie_manager(store) else { return done(Vec::new()) };
    let done = Rc::new(done);
    let d = done.clone();
    let cb = GetCookiesCompletedHandler::create(Box::new(move |hr, list| {
        let mut recs = Vec::new();
        if let (Ok(()), Some(list)) = (hr, list) {
            let mut n = 0u32;
            unsafe { list.Count(&mut n)? };
            for i in 0..n {
                if let Ok(rec) = unsafe { list.GetValueAtIndex(i) }.and_then(|c| cookie_rec(&c)) {
                    recs.push(rec);
                }
            }
        }
        d(recs);
        Ok(())
    }));
    if unsafe { cm.GetCookies(PCWSTR::null(), &cb) }.is_err() {
        done(Vec::new());
    }
}

fn add_cookie(cm: &ICoreWebView2CookieManager, c: &CookieRec) -> windows::core::Result<()> {
    unsafe {
        let k = cm.CreateCookie(
            &HSTRING::from(c.name.as_str()),
            &HSTRING::from(c.value.as_str()),
            &HSTRING::from(c.domain.as_str()),
            &HSTRING::from(c.path.as_str()),
        )?;
        if let Some(e) = c.expires {
            k.SetExpires(e)?;
        }
        k.SetIsSecure(c.secure)?;
        k.SetIsHttpOnly(c.http_only)?;
        k.SetSameSite(match c.same_site.as_deref().map(str::to_ascii_lowercase).as_deref() {
            Some("lax") => COREWEBVIEW2_COOKIE_SAME_SITE_KIND_LAX,
            Some("strict") => COREWEBVIEW2_COOKIE_SAME_SITE_KIND_STRICT,
            _ => COREWEBVIEW2_COOKIE_SAME_SITE_KIND_NONE,
        })?;
        cm.AddOrUpdateCookie(&k)
    }
}

pub fn import_cookies(store: &Store, cookies: &[CookieRec], done: impl Fn() + 'static) {
    let now = crate::vault::now_ms() as f64 / 1000.0;
    if let Some(cm) = cookie_manager(store) {
        for c in cookies.iter().filter(|c| c.expires.is_none_or(|e| e > now)) {
            let _ = add_cookie(&cm, c);
        }
    }
    done();
}

/// Removes the given cookies (site-info "clear cookies").
pub fn delete_cookies(store: &Store, cookies: &[CookieRec], done: impl Fn() + 'static) {
    if let Some(cm) = cookie_manager(store) {
        for c in cookies {
            let _ = unsafe {
                cm.DeleteCookiesWithDomainAndPath(
                    &HSTRING::from(c.name.as_str()),
                    &HSTRING::from(c.domain.as_str()),
                    &HSTRING::from(c.path.as_str()),
                )
            };
        }
    }
    done();
}

/// Server certificate chain (DER, leaf first) and problems of the committed page.
pub fn cert_chain(v: &WebView) -> Option<(Vec<Vec<u8>>, Vec<&'static str>)> {
    with_view(key(v), |s| {
        let errors = match s.tls? {
            Tls::Secure => Vec::new(),
            Tls::Mixed => vec!["cert_err_mixed"],
            Tls::Unverified => vec!["cert_err_unverified"],
        };
        (!s.chain.is_empty()).then(|| (s.chain.clone(), errors))
    })
}

/// Wipes cookies, cache, storage and credentials of the store's profile; `done` runs after.
pub fn clear_store(store: &Store, done: impl FnOnce() + 'static) {
    let profile = store.root.as_ref().and_then(|r| unsafe {
        r.CoreWebView2()
            .and_then(|w| w.cast::<ICoreWebView2_13>())
            .and_then(|w| w.Profile())
            .and_then(|p| p.cast::<ICoreWebView2Profile2>())
            .ok()
    });
    let Some(profile) = profile else { return done() };
    let done = Rc::new(Cell::new(Some(done)));
    let d = done.clone();
    let cb = ClearBrowsingDataCompletedHandler::create(Box::new(move |_| {
        if let Some(f) = d.take() {
            f();
        }
        Ok(())
    }));
    if unsafe { profile.ClearBrowsingDataAll(&cb) }.is_err()
        && let Some(f) = done.take()
    {
        f();
    }
}

/// Window chrome follows the theme; pages and browser UI get it as `prefers-color-scheme`.
pub fn set_theme(window: &Window, theme: Option<Theme>) {
    window.set_theme(theme);
    let scheme = match theme {
        Some(Theme::Dark) => COREWEBVIEW2_PREFERRED_COLOR_SCHEME_DARK,
        Some(Theme::Light) => COREWEBVIEW2_PREFERRED_COLOR_SCHEME_LIGHT,
        _ => COREWEBVIEW2_PREFERRED_COLOR_SCHEME_AUTO,
    };
    SCHEME.with(|s| s.set(scheme));
    PROFILES.with(|l| {
        for (_, p) in &*l.borrow() {
            let _ = unsafe { p.SetPreferredColorScheme(scheme) };
        }
    });
}

pub fn is_light(window: &Window) -> bool {
    window.theme() == Theme::Light
}

/// Adapts the macOS-written browser UI: shortcut labels, paths, File Explorer. `windows` body
/// class: drawn caption buttons (see `frame_proc`). No backslashes: the replacements also
/// run over the embedded locale JSON.
pub fn ui_html(html: String) -> String {
    const MAP: &[(&str, &str)] = &[
        ("⌃⌘L", "Ctrl+Shift+L"),
        ("⌘⌥B", "Ctrl+Shift+B"),
        ("⌘⌥L", "Ctrl+J"),
        ("⌘⇧H", "Alt+Home"),
        ("⌘⇧G", "Shift+F3"),
        ("⌘+ / ⌘− / ⌘0", "Ctrl++ / Ctrl+− / Ctrl+0"),
        ("⌘[", "Alt+←"),
        ("⌘]", "Alt+→"),
        ("⌘G", "F3"),
        ("⌘", "Ctrl+"),
        ("Zobraziť vo Finderi", "Zobraziť v priečinku"),
        ("ukáže vo Finderi", "ukáže v Prieskumníkovi"),
        ("Zobrazit ve Finderu", "Zobrazit ve složce"),
        ("zobrazí ve Finderu", "zobrazí v Průzkumníkovi"),
        ("Show in Finder", "Show in folder"),
        ("shown in Finder", "shown in File Explorer"),
        ("Im Finder zeigen", "Im Ordner zeigen"),
        ("im Finder angezeigt", "im Explorer angezeigt"),
        ("⌥klik", "Alt+klik"),
        ("⌥click", "Alt+click"),
        ("⌥Klick", "Alt+Klick"),
        ("~/Library/Application Support/Crow/", "%APPDATA%/Crow/"),
    ];
    let mut html = html;
    for (a, b) in MAP {
        html = html.replace(a, b);
    }
    html.replacen("<body>", "<body class=\"windows\">", 1)
}

/// Download handler gate: true only once per user-approved URL.
pub fn take_approved(url: &str) -> bool {
    APPROVED.with(|a| {
        let mut a = a.borrow_mut();
        let i = a.iter().position(|u| u == url);
        i.map(|i| a.remove(i)).is_some()
    })
}

/// Answer to the download prompt. wry's handler is synchronous, so the first attempt was
/// cancelled; on approval the URL is whitelisted once and re-requested in its tab.
pub fn decide_download(_key: u32, allow: bool, url: &str, view: Option<&WebView>) {
    if !allow {
        return;
    }
    if let Some(v) = view {
        APPROVED.with(|a| a.borrow_mut().push(url.to_string()));
        let _ = v.load_url(url);
    }
}

pub fn download_dir() -> PathBuf {
    unsafe { SHGetKnownFolderPath(&FOLDERID_Downloads, KF_FLAG_DEFAULT, None) }
        .map_or_else(|_| PathBuf::from(std::env::var_os("USERPROFILE").unwrap_or_default()).join("Downloads"), |p| PathBuf::from(take_pwstr(p)))
}

/// Never opens/executes a downloaded file: selects it in File Explorer
/// (`SHOpenFolderAndSelectItems`), falls back to opening its folder.
pub fn reveal(path: &str) {
    let path = path.to_string();
    std::thread::spawn(move || {
        let shown = unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let pidl = ILCreateFromPathW(&HSTRING::from(path.as_str()));
            if pidl.is_null() {
                false
            } else {
                let ok = SHOpenFolderAndSelectItems(pidl, None, 0).is_ok();
                ILFree(Some(pidl));
                ok
            }
        };
        if !shown && let Some(dir) = Path::new(&path).parent().filter(|d| d.is_dir()) {
            let _ = std::process::Command::new("explorer.exe").arg(dir).status();
        }
    });
}

/// Main window for the stores' hidden root views, and Crow's shortcuts: every controller hands
/// accelerator keys (Ctrl / Alt combinations, F-keys) to `shortcut` before the page sees them.
/// Also turns the window into a client-side decorated one (`frame_proc`): the title bar goes,
/// the tab strip with Crow's caption buttons takes its place.
pub fn init_window(window: &Window, on_cmd: impl Fn(String) + 'static) {
    MAIN_HWND.with(|h| h.set(window.hwnd()));
    ON_CMD.with(|o| *o.borrow_mut() = Some(Rc::new(on_cmd)));
    remove_stale_data();
    let hwnd = hwnd();
    unsafe {
        let _ = SetWindowSubclass(hwnd, Some(frame_proc), 1, 0);
        extend_frame(hwnd);
        // Re-run WM_NCCALCSIZE with the subclass in place.
        let _ = SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

/// Height of the tab strip (`app::TABS_H`) in logical pixels: the window's caption.
const CAPTION_H: i32 = 36;

/// 1 px of DWM frame inside the client area: native shadow, border, snap layouts and
/// animations stay, although the title bar is gone.
fn extend_frame(hwnd: HWND) {
    let m = MARGINS { cxLeftWidth: 0, cxRightWidth: 0, cyTopHeight: 1, cyBottomHeight: 0 };
    let _ = unsafe { DwmExtendFrameIntoClientArea(hwnd, &m) };
}

/// Resize border thickness (frame + padding) at the window's DPI.
fn frame_size(hwnd: HWND) -> (i32, i32) {
    unsafe {
        let dpi = GetDpiForWindow(hwnd);
        let pad = GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi);
        (GetSystemMetricsForDpi(SM_CXFRAME, dpi) + pad, GetSystemMetricsForDpi(SM_CYFRAME, dpi) + pad)
    }
}

/// Client-side decorations. WM_NCCALCSIZE: the client area starts at the window's top edge
/// (no caption); left, right and bottom keep their native, invisible resize borders, which no
/// WebView2 child window can cover. Maximized, the frame the system pushes off-screen is
/// trimmed so the tab strip stays visible. WM_NCHITTEST: top edge / corners resize, the rest of
/// the tab strip is the caption (only where no view covers the client area; over the toolbar,
/// moving and top resizing come through its IPC: `drag`, `resize:*`).
#[allow(clippy::cast_possible_truncation)] // GET_X_LPARAM / GET_Y_LPARAM
unsafe extern "system" fn frame_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _id: usize, _data: usize) -> LRESULT {
    unsafe {
        match msg {
            WM_NCCALCSIZE if wp.0 != 0 => {
                let params = &mut *(lp.0 as *mut NCCALCSIZE_PARAMS);
                let top = params.rgrc[0].top;
                let r = DefSubclassProc(hwnd, msg, wp, lp);
                if r.0 != 0 {
                    return r;
                }
                params.rgrc[0].top = top + if IsZoomed(hwnd).as_bool() { frame_size(hwnd).1 } else { 0 };
                LRESULT(0)
            }
            WM_NCHITTEST => {
                let r = DefSubclassProc(hwnd, msg, wp, lp);
                if r.0 != HTCLIENT as isize {
                    return r;
                }
                let (x, y) = (lp.0 as u16 as i16, (lp.0 >> 16) as u16 as i16);
                let mut pt = POINT { x: i32::from(x), y: i32::from(y) };
                let _ = ScreenToClient(hwnd, &mut pt);
                let mut rc = RECT::default();
                let _ = GetClientRect(hwnd, &mut rc);
                let (fx, fy) = frame_size(hwnd);
                let caption = CAPTION_H * GetDpiForWindow(hwnd) as i32 / 96;
                let hit = if !IsZoomed(hwnd).as_bool() && pt.y < fy {
                    if pt.x < fx {
                        HTTOPLEFT
                    } else if pt.x >= rc.right - fx {
                        HTTOPRIGHT
                    } else {
                        HTTOP
                    }
                } else if pt.y < caption {
                    HTCAPTION
                } else {
                    HTCLIENT
                };
                LRESULT(hit as isize)
            }
            WM_ACTIVATE | WM_DWMCOMPOSITIONCHANGED => {
                extend_frame(hwnd);
                DefSubclassProc(hwnd, msg, wp, lp)
            }
            _ => DefSubclassProc(hwnd, msg, wp, lp),
        }
    }
}

/// Caption buttons of the tab strip (`window-minimize`, `window-maximize`, `window-close`).
/// Close goes through WM_CLOSE, i.e. the regular quit path (vault saved first).
pub fn caption(cmd: &str) {
    let hwnd = hwnd();
    unsafe {
        match cmd {
            "window-minimize" => {
                let _ = ShowWindow(hwnd, SW_MINIMIZE);
            }
            "window-maximize" => {
                let _ = ShowWindow(hwnd, if IsZoomed(hwnd).as_bool() { SW_RESTORE } else { SW_MAXIMIZE });
            }
            "window-close" => {
                let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
            _ => {}
        }
    }
}

/// Native exit (after every view was dropped): the environment goes, then its folder.
pub fn shutdown() {
    VIEWS.with(|m| m.borrow_mut().clear());
    PROFILES.with(|p| p.borrow_mut().clear());
    ENV.with(|e| e.borrow_mut().take());
    // The browser process exits shortly after its last controller closed and holds the folder
    // until then; leftovers are removed on the next start (`remove_stale_data`).
    if let Some(dir) = DATA_DIR.with(|d| d.borrow().clone()) {
        for _ in 0..30 {
            if std::fs::remove_dir_all(&dir).is_ok() || !dir.exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

fn down(vk: VIRTUAL_KEY) -> bool {
    (unsafe { GetKeyState(i32::from(vk.0)) }) < 0
}

/// macOS menu shortcuts mapped to Windows conventions (⌘ → Ctrl, plus common browser aliases).
/// Digits and punctuation by virtual key, so they work on any layout.
fn shortcut(vk: u32) -> Option<&'static str> {
    let (ctrl, shift, alt) = (down(VK_CONTROL), down(VK_SHIFT), down(VK_MENU));
    let vk = u16::try_from(vk).ok()?;
    let is = |k: VIRTUAL_KEY| vk == k.0;
    let letter = char::from_u32(u32::from(vk)).filter(char::is_ascii_uppercase);
    Some(match (ctrl, shift, alt) {
        (true, false, false) => match letter {
            Some('T') => "new-tab",
            Some('W') => "close-tab",
            Some('L') => "location",
            Some('F') => "find",
            Some('G') => "find-next",
            Some('R') => "reload",
            Some('D') => "bookmark",
            Some('H' | 'Y') => "panel:history",
            Some('J') => "panel:downloads",
            Some('Q') => "quit",
            None if (0x31..=0x39).contains(&vk) => {
                return Some(["tab:1", "tab:2", "tab:3", "tab:4", "tab:5", "tab:6", "tab:7", "tab:8", "tab:9"][usize::from(vk - 0x31)]);
            }
            None if vk == 0x30 || is(VK_NUMPAD0) => "zoom-reset",
            None if is(VK_OEM_MINUS) || is(VK_SUBTRACT) => "zoom-out",
            None if is(VK_OEM_PLUS) || is(VK_ADD) => "zoom-in",
            None if is(VK_OEM_4) => "back",
            None if is(VK_OEM_6) => "forward",
            None if is(VK_OEM_COMMA) => "panel:settings",
            None if is(VK_F4) => "close-tab",
            None if is(VK_TAB) || is(VK_NEXT) => "next-tab",
            None if is(VK_PRIOR) => "prev-tab",
            _ => return None,
        },
        (true, true, false) => match letter {
            Some('T') => "reopen-tab",
            Some('G') => "find-prev",
            Some('H') => "home",
            Some('B' | 'O') => "panel:bookmarks",
            Some('Y') => "panel:downloads",
            Some('L') => "lock",
            Some('X') => "panic",
            None if is(VK_OEM_PLUS) => "zoom-in",
            None if is(VK_TAB) || is(VK_OEM_4) => "prev-tab",
            None if is(VK_OEM_6) => "next-tab",
            _ => return None,
        },
        (false, false, true) => match letter {
            Some('D') => "location",
            _ if is(VK_HOME) => "home",
            _ if is(VK_LEFT) => "back",
            _ if is(VK_RIGHT) => "forward",
            _ => return None,
        },
        (false, false, false) => match () {
            () if is(VK_F3) => "find-next",
            () if is(VK_F5) => "reload",
            () if is(VK_F6) => "location",
            () if is(VK_F11) => "fullscreen",
            () => return None,
        },
        (false, true, false) if is(VK_F3) => "find-prev",
        _ => return None,
    })
}
