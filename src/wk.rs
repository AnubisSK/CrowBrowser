//! Direct WebKit glue (macOS): shared in-memory data store, cookie export/import, content blockers,
//! plus the small platform surface `app.rs` needs (view attach, theme, file actions).
//! Linux counterpart: `wk_gtk.rs` (same API).

use crate::vault::CookieRec;
use block2::RcBlock;
use objc2::{MainThreadMarker, rc::Retained, runtime::AnyObject};
use objc2_foundation::{
    NSArray, NSDate, NSError, NSHTTPCookie, NSHTTPCookieDomain, NSHTTPCookieExpires,
    NSHTTPCookieName, NSHTTPCookiePath, NSHTTPCookiePropertyKey, NSHTTPCookieSecure,
    NSHTTPCookieValue, NSMutableDictionary, NSString, ns_string,
};
use objc2_web_kit::{
    WKContentRuleList, WKContentRuleListStore, WKContentWorld, WKUserContentController,
    WKUserScript, WKUserScriptInjectionTime, WKWebViewConfiguration, WKWebsiteDataStore,
};
use objc2::{
    encode::{Encoding, RefEncode},
    msg_send,
    runtime::{AnyClass, Bool, Imp, Sel},
    sel,
};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    ffi::c_void,
    ptr::NonNull,
    rc::Rc,
};
use tao::window::{Theme, Window};
use wry::{
    NewWindowFeatures, NewWindowResponse, Rect, WebView, WebViewBuilder, WebViewBuilderExtMacos,
    WebViewExtMacOS,
};

pub type Store = Retained<WKWebsiteDataStore>;
/// Certificate failure callback: tab, refused URL, reasons.
type TlsHandler = dyn Fn(u32, String, Vec<&'static str>);
/// Web process crash callback: tab.
type CrashHandler = dyn Fn(u32);

/// Engine-confirmed connection security of a committed page.
#[derive(Clone, Copy, PartialEq)]
pub enum Tls {
    /// Certificate validated by WebKit, no insecure subresources.
    Secure,
    /// Valid certificate, but HTTP content on the page.
    Mixed,
    /// No validated certificate (certificate errors, or not HTTPS).
    Unverified,
}
pub struct Config(Retained<WKWebViewConfiguration>);

thread_local! {
    /// Compiled content-blocker lists by id (see `adblock::LISTS`).
    static RULES: RefCell<Vec<(&'static str, Retained<WKContentRuleList>)>> = const { RefCell::new(Vec::new()) };
    /// Loads blocked per tab since the last `take_blocked`.
    static BLOCKED: RefCell<HashMap<u32, u32>> = RefCell::new(HashMap::new());
    static ON_BLOCKED: RefCell<Option<Box<dyn Fn()>>> = const { RefCell::new(None) };
    static ON_TLS: RefCell<Option<Box<TlsHandler>>> = const { RefCell::new(None) };
    static ON_CRASH: RefCell<Option<Box<CrashHandler>>> = const { RefCell::new(None) };
    /// Blocked loads per domain since the last `take_blocked_hosts`.
    static BLOCKED_HOSTS: RefCell<HashMap<String, u32>> = RefCell::new(HashMap::new());
    /// URLs the user confirmed in the download prompt; consumed by the next download of that URL.
    static APPROVED: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

fn mtm() -> MainThreadMarker {
    MainThreadMarker::new().expect("main thread")
}

/// Nothing written to disk by WebKit; persistence goes through the encrypted vault.
pub fn new_store() -> Store {
    unsafe { WKWebsiteDataStore::nonPersistentDataStore(mtm()) }
}

/// Separate in-memory store of a container: every `nonPersistentDataStore` call is a new,
/// isolated one (own cookies, storage, cache), freed when its last view and owner drop it.
pub fn new_container_store() -> Store {
    new_store()
}

/// The store's last owner lets go of it (macOS frees it with its views).
pub fn drop_container_store(_store: &Store) {}

/// Native picture-in-picture for page videos (`PIP_JS`).
pub const PIP: bool = true;

/// Picture-in-picture is off in WKWebView by default; the private preference (guarded) turns on
/// the system's floating player for `<video>` (also MSE / blob streams).
#[cfg(target_os = "macos")]
fn pip_prefs(c: &WKWebViewConfiguration) {
    unsafe {
        let prefs: *mut AnyObject = msg_send![c, preferences];
        let Some(prefs) = prefs.as_ref() else { return };
        let ok: Bool = msg_send![prefs, respondsToSelector: sel!(_setAllowsPictureInPictureMediaPlayback:)];
        if ok.as_bool() {
            let _: () = msg_send![prefs, _setAllowsPictureInPictureMediaPlayback: true];
        }
    }
}

/// Media playback of tab views: AirPlay routing allowed (`allowsAirPlayForMediaPlayback`) and no
/// media type gated behind a user gesture (`WKAudiovisualMediaTypeNone` = 0), so a started video
/// can be sent to an AirPlay device. Both are WebKit's macOS defaults; set explicitly so a changed
/// default cannot silently disable them. Storage settings are untouched.
///
/// Element fullscreen (`requestFullscreen` / `webkitRequestFullscreen`, e.g. YouTube): public
/// `elementFullscreenEnabled` (macOS 12.3+), private `fullScreenEnabled` key before that. wry sets
/// it too; explicit here because tab configurations are built by Crow. WebKit then shows the
/// element in its own fullscreen window and puts the view back afterwards.
#[cfg(target_os = "macos")]
fn media_prefs(c: &WKWebViewConfiguration) {
    unsafe {
        let _: () = msg_send![c, setAllowsAirPlayForMediaPlayback: true];
        let _: () = msg_send![c, setMediaTypesRequiringUserActionForPlayback: 0usize];
        let prefs: *mut AnyObject = msg_send![c, preferences];
        let Some(prefs) = prefs.as_ref() else { return };
        let public: Bool = msg_send![prefs, respondsToSelector: sel!(setElementFullscreenEnabled:)];
        if public.as_bool() {
            let _: () = msg_send![prefs, setElementFullscreenEnabled: true];
        } else {
            let yes = objc2_foundation::NSNumber::new_bool(true);
            let _: () = msg_send![prefs, setValue: &*yes, forKey: ns_string!("fullScreenEnabled")];
        }
    }
}

/// Web storage of tab views: localStorage lives in memory in the non-persistent store
/// (IndexedDB and service workers have no switch and are on). WebKit's default; set explicitly
/// via the private `_localStorageEnabled` key (guarded) so a changed default cannot break sites
/// that hydrate from storage.
#[cfg(target_os = "macos")]
fn storage_prefs(c: &WKWebViewConfiguration) {
    unsafe {
        let prefs: *mut AnyObject = msg_send![c, preferences];
        let Some(prefs) = prefs.as_ref() else { return };
        let ok: Bool = msg_send![prefs, respondsToSelector: sel!(_setLocalStorageEnabled:)];
        if ok.as_bool() {
            let _: () = msg_send![prefs, _setLocalStorageEnabled: true];
        }
    }
}

/// Local files: a `file:` page may read neither other files nor other origins. WebKit's defaults;
/// set explicitly via the private keys (guarded; the public-looking setters do not exist) so a
/// changed default cannot open them up.
#[cfg(target_os = "macos")]
fn file_prefs(c: &WKWebViewConfiguration) {
    unsafe {
        let ok: Bool = msg_send![c, respondsToSelector: sel!(_setAllowUniversalAccessFromFileURLs:)];
        if ok.as_bool() {
            let _: () = msg_send![c, _setAllowUniversalAccessFromFileURLs: false];
        }
        let prefs: *mut AnyObject = msg_send![c, preferences];
        let Some(prefs) = prefs.as_ref() else { return };
        let ok: Bool = msg_send![prefs, respondsToSelector: sel!(_setAllowFileAccessFromFileURLs:)];
        if ok.as_bool() {
            let _: () = msg_send![prefs, _setAllowFileAccessFromFileURLs: false];
        }
    }
}

/// Gamepad API on or off. WKWebView enables it by default; the only switch is the private
/// feature list (`_features` / `_setEnabled:forFeature:`, guarded). There is no `gamepadEnabled`
/// preference key: KVC on it would throw. Without the SPI the page-side shim still removes the API.
#[cfg(target_os = "macos")]
fn gamepad_prefs(c: &WKWebViewConfiguration, on: bool) {
    unsafe {
        let Some(cls) = AnyClass::get(c"WKPreferences") else { return };
        let prefs: *mut AnyObject = msg_send![c, preferences];
        let Some(prefs) = prefs.as_ref() else { return };
        let list: Bool = msg_send![cls, respondsToSelector: sel!(_features)];
        let set: Bool = msg_send![prefs, respondsToSelector: sel!(_setEnabled:forFeature:)];
        if !list.as_bool() || !set.as_bool() {
            return;
        }
        let features: *mut NSArray<AnyObject> = msg_send![cls, _features];
        let Some(features) = features.as_ref() else { return };
        for f in features {
            let key: *mut NSString = msg_send![&*f, key];
            if key.as_ref().is_some_and(|k| k.to_string() == "GamepadsEnabled") {
                let _: () = msg_send![prefs, _setEnabled: on, forFeature: &*f];
            }
        }
    }
}

/// Built-in page `html` under the document address `base` (see `app::INTERNAL_BASE`).
pub fn load_internal(v: &WebView, html: &str, base: &str) {
    unsafe {
        let url = objc2_foundation::NSURL::URLWithString(&NSString::from_str(base));
        let _: *mut AnyObject = msg_send![&*v.webview(), loadHTMLString: &*NSString::from_str(html), baseURL: url.as_deref()];
    }
}

unsafe extern "C" {
    fn dlopen(path: *const std::ffi::c_char, mode: std::ffi::c_int) -> *mut c_void;
}

/// Game controllers macOS itself sees (GameController framework), for the crow:gamepad
/// diagnostics. The framework is loaded only when asked (WebKit loads it anyway for gamepads).
pub fn system_gamepads() -> Option<Vec<String>> {
    static LOAD: std::sync::Once = std::sync::Once::new();
    LOAD.call_once(|| unsafe {
        dlopen(c"/System/Library/Frameworks/GameController.framework/GameController".as_ptr(), 1);
    });
    unsafe {
        let cls = AnyClass::get(c"GCController")?;
        let list: *mut NSArray<AnyObject> = msg_send![cls, controllers];
        let list = list.as_ref()?;
        let text = |o: *mut NSString| o.as_ref().map(ToString::to_string);
        Some(
            list.iter()
                .map(|c| {
                    let name = text(msg_send![&*c, vendorName]).unwrap_or_else(|| "?".into());
                    match text(msg_send![&*c, productCategory]) {
                        Some(kind) if kind != name => format!("{name} ({kind})"),
                        _ => name,
                    }
                })
                .collect(),
        )
    }
}

/// Runs `js` at document start in every frame of `v`, in Crow's isolated JS world (see
/// `eval_isolated`). Called before the view's first load.
pub fn add_isolated_script(v: &WebView, js: &str) {
    unsafe {
        let world = WKContentWorld::worldWithName(ns_string!("crow"), mtm());
        let script = WKUserScript::initWithSource_injectionTime_forMainFrameOnly_inContentWorld(
            mtm().alloc::<WKUserScript>(),
            &NSString::from_str(js),
            WKUserScriptInjectionTime::AtDocumentStart,
            false,
            &world,
        );
        v.webview().configuration().userContentController().addUserScript(&script);
    }
}

/// WebKit runs element fullscreen in its own window on macOS: nothing for the app to resize.
pub fn on_fullscreen(_f: impl Fn(u32, bool) + 'static) {}

pub fn config(store: &Store, adblock: bool, gamepad: bool) -> Config {
    unsafe {
        let c = WKWebViewConfiguration::new(mtm());
        c.setWebsiteDataStore(store);
        if adblock {
            set_manager_rules(&c.userContentController(), true);
        }
        media_prefs(&c);
        storage_prefs(&c);
        file_prefs(&c);
        gamepad_prefs(&c, gamepad);
        pip_prefs(&c);
        Config(c)
    }
}

/// window.open target: WebKit's own configuration keeps `window.opener` working.
pub fn popup_config(features: NewWindowFeatures) -> Config {
    let c = features.opener.target_configuration;
    media_prefs(&c);
    storage_prefs(&c);
    file_prefs(&c);
    Config(c)
}

/// Monterey's WebKit reports an old Safari version, so sites (e.g. Google) serve legacy layouts.
/// Native Safari identity only: a Chrome UA makes Google serve Blink-only layouts that break in WebKit.
/// `customUserAgent` covers the request header and `navigator.userAgent` alike.
#[cfg(target_os = "macos")]
const USER_AGENT: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.3 Safari/605.1.15";

/// Tab views (including adopted popups); the browser UI uses `ui_builder`.
/// `page_js`: the page-world shim (`adblock::defuser`), injected into every frame.
pub fn builder<'a>(config: Config, page_js: String) -> WebViewBuilder<'a> {
    WebViewBuilder::new()
        .with_webview_configuration(config.0)
        .with_user_agent(USER_AGENT)
        .with_initialization_script_for_main_only(page_js, false)
}

/// Browser UI (toolbar, panel, lock screen, popovers): its own non-persistent data store
/// (`incognito` = `nonPersistentDataStore`), so no UI cache or storage ever reaches the disk.
pub fn ui_builder<'a>() -> WebViewBuilder<'a> {
    WebViewBuilder::new().with_incognito(true)
}

pub fn attach(b: WebViewBuilder, rect: Rect, window: &Window) -> wry::Result<WebView> {
    b.with_bounds(rect).build_as_child(window)
}

/// Runs `js` in the main frame in Crow's own JS world: same DOM, but globals set there are
/// invisible to the page's scripts. `done` gets the result if it is a string, else "".
pub fn eval_isolated(v: &WebView, js: &str, done: impl FnOnce(String) + 'static) {
    let done = Cell::new(Some(done));
    let block = RcBlock::new(move |res: *mut AnyObject, _err: *mut NSError| {
        let s = unsafe { res.as_ref() }
            .and_then(|o| o.downcast_ref::<NSString>())
            .map(ToString::to_string)
            .unwrap_or_default();
        if let Some(f) = done.take() {
            f(s);
        }
    });
    unsafe {
        let world = WKContentWorld::worldWithName(ns_string!("crow"), mtm());
        v.webview().evaluateJavaScript_inFrame_inContentWorld_completionHandler(
            &NSString::from_str(js),
            None,
            &world,
            Some(&block),
        );
    }
}

/// Left mouse button is held right now. `drag_window` replays AppKit's current event; started
/// after the button was released, AppKit logs "Window move completed without beginning".
pub fn left_button_down() -> bool {
    let pressed: usize = unsafe { msg_send![objc2::class!(NSEvent), pressedMouseButtons] };
    pressed & 1 != 0
}

pub fn set_bounds(v: &WebView, rect: Rect) {
    let _ = v.set_bounds(rect);
}

pub fn popup_response(v: &WebView) -> NewWindowResponse {
    NewWindowResponse::Create { webview: Retained::into_super(v.webview()) }
}

pub fn set_rules(v: &WebView, on: bool) {
    set_manager_rules(&v.manager(), on);
}

fn set_manager_rules(manager: &WKUserContentController, on: bool) {
    unsafe {
        manager.removeAllContentRuleLists();
        if on {
            RULES.with(|r| {
                for (_, list) in &*r.borrow() {
                    manager.addContentRuleList(list);
                }
            });
        }
    }
}

fn store() -> Option<Retained<WKContentRuleListStore>> {
    unsafe { WKContentRuleListStore::defaultStore(mtm()) }
}

fn keep_rules(id: &'static str, list: Retained<WKContentRuleList>) {
    RULES.with(|r| {
        let mut r = r.borrow_mut();
        r.retain(|(i, _)| *i != id);
        r.push((id, list));
    });
}

/// Compiles rule list `id` (stored by WebKit, reused via `load_rules`).
/// Takes `json` by value to match the Linux backend, which hands it to GLib without copying.
#[allow(clippy::needless_pass_by_value)]
pub fn compile_rules(id: &'static str, json: String, done: impl Fn() + 'static) {
    let Some(store) = store() else { return };
    let block = RcBlock::new(move |list: *mut WKContentRuleList, err: *mut NSError| {
        if let Some(list) = unsafe { Retained::retain(list) } {
            keep_rules(id, list);
            done();
        } else if let Some(err) = unsafe { err.as_ref() } {
            eprintln!("adblock: {}", err.localizedDescription());
        }
    });
    unsafe {
        store.compileContentRuleListForIdentifier_encodedContentRuleList_completionHandler(
            Some(&NSString::from_str(id)),
            Some(&NSString::from_str(&json)),
            Some(&block),
        );
    }
}

/// Loads the previously compiled list `id`; `fail` if WebKit no longer has it.
pub fn load_rules(id: &'static str, ok: impl Fn() + 'static, fail: impl Fn() + 'static) {
    let Some(store) = store() else { return };
    let block = RcBlock::new(move |list: *mut WKContentRuleList, _err: *mut NSError| {
        match unsafe { Retained::retain(list) } {
            Some(list) => {
                keep_rules(id, list);
                ok();
            }
            None => fail(),
        }
    });
    unsafe { store.lookUpContentRuleListForIdentifier_completionHandler(Some(&NSString::from_str(id)), Some(&block)) };
}

/// Disabled list: forgotten here and removed from WebKit's store.
pub fn drop_rules(id: &'static str) {
    RULES.with(|r| r.borrow_mut().retain(|(i, _)| *i != id));
    if let Some(store) = store() {
        unsafe { store.removeContentRuleListForIdentifier_completionHandler(Some(&NSString::from_str(id)), Some(&RcBlock::new(|_: *mut NSError| {}))) };
    }
}

/// Blocked loads are reported (and counted) on this platform.
pub const BLOCK_STATS: bool = true;

/// Called once per batch of blocked loads (collect them with `take_blocked`).
pub fn on_blocked(f: impl Fn() + 'static) {
    ON_BLOCKED.with(|o| *o.borrow_mut() = Some(Box::new(f)));
}

/// WebKit SPI (macOS 10.15.4+, `WKNavigationDelegatePrivate`): called for every content-blocker
/// action. Content blockers have no public reporting API.
fn rule_action_sel() -> Sel {
    sel!(_webView:contentRuleListWithIdentifier:performedAction:forURL:)
}

unsafe extern "C-unwind" fn rule_action(
    _this: *mut AnyObject,
    _cmd: Sel,
    web_view: *mut AnyObject,
    _id: *mut AnyObject,
    action: *mut AnyObject,
    url: *mut AnyObject,
) {
    let Some(action) = (unsafe { action.as_ref() }) else { return };
    let blocked = unsafe {
        let known: Bool = msg_send![action, respondsToSelector: sel!(blockedLoad)];
        known.as_bool() && msg_send![action, blockedLoad]
    };
    let (true, Some(tab)) = (blocked, tab_of(web_view)) else { return };
    let host: Option<String> = unsafe { url.as_ref() }.and_then(|u| {
        let h: *mut NSString = unsafe { msg_send![u, host] };
        unsafe { h.as_ref() }.map(|h| h.to_string().to_ascii_lowercase())
    });
    if let Some(host) = host.filter(|h| !h.is_empty()) {
        BLOCKED_HOSTS.with(|b| *b.borrow_mut().entry(host).or_default() += 1);
    }
    let first = BLOCKED.with(|b| {
        let mut b = b.borrow_mut();
        let first = b.is_empty();
        *b.entry(tab).or_default() += 1;
        first
    });
    if first {
        ON_BLOCKED.with(|f| {
            if let Some(f) = &*f.borrow() {
                f();
            }
        });
    }
}

/// Called when a navigation is refused because of the server certificate (tab, URL, reasons).
pub fn on_tls_error(f: impl Fn(u32, String, Vec<&'static str>) + 'static) {
    ON_TLS.with(|o| *o.borrow_mut() = Some(Box::new(f)));
}

/// NSURLErrorDomain TLS codes. wry answers no authentication challenge, so WebKit's default
/// trust evaluation rejects bad certificates; this only tells the user why the page did not load.
fn tls_reason(code: isize) -> Option<&'static str> {
    Some(match code {
        -1201 => "cert_err_bad_date",
        -1202 => "cert_err_untrusted",
        -1203 => "cert_err_unknown_ca",
        -1204 => "cert_err_not_activated",
        -1200 => "cert_err_tls_failed",
        -1205 | -1206 => "cert_err_client_cert",
        _ => return None,
    })
}

/// Web content process of a tab ended (crash, out of memory, WebGL hang): `f` gets the tab.
pub fn on_crash(f: impl Fn(u32) + 'static) {
    ON_CRASH.with(|o| *o.borrow_mut() = Some(Box::new(f)));
}

/// `webViewWebContentProcessDidTerminate:` (WKNavigationDelegate; wry does not implement it).
unsafe extern "C-unwind" fn web_process_terminated(_this: *mut AnyObject, _cmd: Sel, web_view: *mut AnyObject) {
    let Some(tab) = tab_of(web_view) else { return };
    ON_CRASH.with(|f| {
        if let Some(f) = &*f.borrow() {
            f(tab);
        }
    });
}

unsafe extern "C-unwind" fn nav_failed(
    _this: *mut AnyObject,
    _cmd: Sel,
    web_view: *mut AnyObject,
    _nav: *mut AnyObject,
    error: *mut AnyObject,
) {
    let Some(error) = (unsafe { error.as_ref() }) else { return };
    let (domain, code, url) = unsafe {
        let domain: *mut NSString = msg_send![error, domain];
        let code: isize = msg_send![error, code];
        let info: *mut AnyObject = msg_send![error, userInfo];
        let url: *mut AnyObject = match info.as_ref() {
            Some(info) => msg_send![info, objectForKey: ns_string!("NSErrorFailingURLStringKey")],
            None => std::ptr::null_mut(),
        };
        let url = url.cast::<NSString>().as_ref().map(std::string::ToString::to_string).unwrap_or_default();
        (domain.as_ref().map(std::string::ToString::to_string).unwrap_or_default(), code, url)
    };
    let tab = tab_of(web_view);
    let (Some(tab), Some(reason)) = (tab, tls_reason(code).filter(|_| domain == "NSURLErrorDomain")) else { return };
    ON_TLS.with(|f| {
        if let Some(f) = &*f.borrow() {
            f(tab, url, vec![reason]);
        }
    });
}

/// Key of the tab id associated with a WKWebView (address of this static, never dereferenced).
static TAB_KEY: u8 = 0;

/// Tags the view itself with its tab id (associated object): the tag lives and dies with that
/// exact WKWebView, so a recycled address can never map callbacks to another tab (no ABA, no
/// global pointer map). Tab ids are monotonic and never reused.
fn set_tab(web_view: *mut AnyObject, tab: Option<u32>) {
    let value = tab.map(|t| NSString::from_str(&t.to_string()));
    let ptr = value.as_ref().map_or(std::ptr::null_mut(), |v| Retained::as_ptr(v) as *mut AnyObject);
    unsafe {
        objc2::ffi::objc_setAssociatedObject(
            web_view,
            (&raw const TAB_KEY).cast::<c_void>(),
            ptr,
            objc2::ffi::OBJC_ASSOCIATION_RETAIN_NONATOMIC,
        );
    };
}

fn tab_of(web_view: *mut AnyObject) -> Option<u32> {
    if web_view.is_null() {
        return None;
    }
    let v = unsafe { objc2::ffi::objc_getAssociatedObject(web_view, (&raw const TAB_KEY).cast::<c_void>()) };
    unsafe { v.cast::<NSString>().as_ref() }?.to_string().parse().ok()
}

/// Adds `imp` for `sel` to the delegate's class unless the class already implements it.
unsafe fn add_method(cls: &AnyClass, sel: Sel, imp: Imp, types: &std::ffi::CStr) {
    if cls.instance_method(sel).is_none() {
        unsafe { objc2::ffi::class_addMethod(std::ptr::from_ref::<AnyClass>(cls).cast_mut(), sel, imp, types.as_ptr()) };
    }
}

/// Reports blocked loads, certificate failures and web process crashes of `v` as tab `tab`: adds
/// the methods to wry's navigation delegate class and re-sets the delegate, so WebKit re-reads
/// which methods exist.
pub fn watch(v: &WebView, tab: u32) {
    let wv = v.webview();
    set_tab(Retained::as_ptr(&wv) as *mut AnyObject, Some(tab));
    unsafe {
        let d: *mut AnyObject = msg_send![&*wv, navigationDelegate];
        let Some(d) = d.as_ref() else { return };
        let cls = d.class();
        let imp: Imp = std::mem::transmute(rule_action as unsafe extern "C-unwind" fn(_, _, _, _, _, _));
        add_method(cls, rule_action_sel(), imp, c"v@:@@@@");
        let imp: Imp = std::mem::transmute(nav_failed as unsafe extern "C-unwind" fn(_, _, _, _, _));
        add_method(cls, sel!(webView:didFailProvisionalNavigation:withError:), imp, c"v@:@@@");
        let imp: Imp = std::mem::transmute(web_process_terminated as unsafe extern "C-unwind" fn(_, _, _));
        add_method(cls, sel!(webViewWebContentProcessDidTerminate:), imp, c"v@:@");
        let _: () = msg_send![&*wv, setNavigationDelegate: d];
    }
}

/// TLS state of the committed page as verified by WebKit (never inferred from the URL).
pub fn tls(v: &WebView) -> Tls {
    let wv = v.webview();
    let trust: *const SecTrust = unsafe { msg_send![&*wv, serverTrust] };
    if trust.is_null() {
        return Tls::Unverified;
    }
    let secure_only: bool = unsafe { msg_send![&*wv, hasOnlySecureContent] };
    if secure_only { Tls::Secure } else { Tls::Mixed }
}

/// Tab is closing/unloading: late callbacks from this view (teardown) are ignored.
pub fn forget(v: &WebView) {
    set_tab(Retained::as_ptr(&v.webview()) as *mut AnyObject, None);
}

/// Blocked loads per tab since the last call.
pub fn take_blocked() -> Vec<(u32, u32)> {
    BLOCKED.with(|b| b.borrow_mut().drain().collect())
}

/// Blocked loads per domain since the last call.
pub fn take_blocked_hosts() -> Vec<(String, u32)> {
    BLOCKED_HOSTS.with(|b| b.borrow_mut().drain().collect())
}

pub fn export_cookies(store: &Store, done: impl Fn(Vec<CookieRec>) + 'static) {
    let block = RcBlock::new(move |arr: NonNull<NSArray<NSHTTPCookie>>| {
        let cookies = unsafe { arr.as_ref() }
            .to_vec()
            .iter()
            .map(|c| CookieRec {
                name: c.name().to_string(),
                value: c.value().to_string(),
                domain: c.domain().to_string(),
                path: c.path().to_string(),
                secure: c.isSecure(),
                http_only: c.isHTTPOnly(),
                same_site: c.sameSitePolicy().map(|s| s.to_string()),
                expires: c.expiresDate().map(|d| d.timeIntervalSince1970()),
            })
            .collect();
        done(cookies);
    });
    unsafe { store.httpCookieStore().getAllCookies(&block) };
}

pub fn import_cookies(store: &Store, cookies: &[CookieRec], done: impl Fn() + 'static) {
    let now = crate::vault::now_ms() as f64 / 1000.0;
    let list: Vec<_> = cookies
        .iter()
        .filter(|c| c.expires.is_none_or(|e| e > now))
        .filter_map(to_ns_cookie)
        .collect();
    if list.is_empty() {
        done();
        return;
    }
    let left = Rc::new(Cell::new(list.len()));
    let done = Rc::new(done);
    let cs = unsafe { store.httpCookieStore() };
    for c in list {
        let (left, done) = (left.clone(), done.clone());
        let block = RcBlock::new(move || {
            left.set(left.get() - 1);
            if left.get() == 0 {
                done();
            }
        });
        unsafe { cs.setCookie_completionHandler(&c, Some(&block)) };
    }
}

fn to_ns_cookie(c: &CookieRec) -> Option<Retained<NSHTTPCookie>> {
    unsafe {
        let p: Retained<NSMutableDictionary<NSHTTPCookiePropertyKey, AnyObject>> =
            NSMutableDictionary::new();
        p.insert(NSHTTPCookieName, &*NSString::from_str(&c.name));
        p.insert(NSHTTPCookieValue, &*NSString::from_str(&c.value));
        p.insert(NSHTTPCookieDomain, &*NSString::from_str(&c.domain));
        p.insert(NSHTTPCookiePath, &*NSString::from_str(&c.path));
        if let Some(e) = c.expires {
            p.insert(NSHTTPCookieExpires, &*NSDate::dateWithTimeIntervalSince1970(e));
        }
        if c.secure {
            p.insert(NSHTTPCookieSecure, ns_string!("TRUE"));
        }
        if c.http_only {
            p.insert(ns_string!("HttpOnly"), ns_string!("TRUE"));
        }
        if let Some(s) = &c.same_site {
            p.insert(ns_string!("SameSite"), &*NSString::from_str(s));
        }
        NSHTTPCookie::cookieWithProperties(&p)
    }
}

/// Removes the given cookies from WebKit (site-info "clear cookies").
pub fn delete_cookies(store: &Store, cookies: &[CookieRec], done: impl Fn() + 'static) {
    let list: Vec<_> = cookies.iter().filter_map(to_ns_cookie).collect();
    if list.is_empty() {
        done();
        return;
    }
    let left = Rc::new(Cell::new(list.len()));
    let done = Rc::new(done);
    let cs = unsafe { store.httpCookieStore() };
    for c in list {
        let (left, done) = (left.clone(), done.clone());
        let block = RcBlock::new(move || {
            left.set(left.get() - 1);
            if left.get() == 0 {
                done();
            }
        });
        unsafe { cs.deleteCookie_completionHandler(&c, Some(&block)) };
    }
}

#[repr(C)]
struct SecTrust {
    _p: [u8; 0],
}

unsafe impl RefEncode for SecTrust {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("__SecTrust", &[]));
}

#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    fn SecTrustGetCertificateCount(trust: *const SecTrust) -> isize;
    fn SecTrustGetCertificateAtIndex(trust: *const SecTrust, i: isize) -> *const c_void;
    fn SecCertificateCopyData(cert: *const c_void) -> *const c_void;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFDataGetLength(data: *const c_void) -> isize;
    fn CFDataGetBytePtr(data: *const c_void) -> *const u8;
    fn CFRelease(obj: *const c_void);
}

/// Server certificate chain (DER, leaf first) and problems of the committed page.
/// WebKit refuses untrusted certificates, so a loaded page has a valid chain.
pub fn cert_chain(v: &WebView) -> Option<(Vec<Vec<u8>>, Vec<&'static str>)> {
    let wv = v.webview();
    let trust: *const SecTrust = unsafe { msg_send![&*wv, serverTrust] };
    if trust.is_null() {
        return None;
    }
    let mut chain = Vec::new();
    unsafe {
        for i in 0..SecTrustGetCertificateCount(trust).min(8) {
            let cert = SecTrustGetCertificateAtIndex(trust, i);
            if cert.is_null() {
                continue;
            }
            let data = SecCertificateCopyData(cert);
            if data.is_null() {
                continue;
            }
            let bytes = std::slice::from_raw_parts(CFDataGetBytePtr(data), CFDataGetLength(data) as usize);
            chain.push(bytes.to_vec());
            CFRelease(data);
        }
    }
    let secure_only: bool = unsafe { msg_send![&*wv, hasOnlySecureContent] };
    let errors = if secure_only { Vec::new() } else { vec!["cert_err_mixed"] };
    Some((chain, errors))
}

/// Wipes cookies, cache, storage and credentials from the in-memory store; `done` runs after.
pub fn clear_store(store: &Store, done: impl FnOnce() + 'static) {
    let done = Cell::new(Some(done));
    let block = RcBlock::new(move || {
        if let Some(f) = done.take() {
            f();
        }
    });
    unsafe {
        store.removeDataOfTypes_modifiedSince_completionHandler(
            &WKWebsiteDataStore::allWebsiteDataTypes(mtm()),
            &NSDate::distantPast(),
            &block,
        );
    }
}

pub fn set_theme(window: &Window, theme: Option<Theme>) {
    window.set_theme(theme);
}

pub fn is_light(window: &Window) -> bool {
    window.theme() == Theme::Light
}

/// Browser-UI HTML is written for macOS; nothing to adapt.
pub fn ui_html(html: String) -> String {
    html
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

/// Never opens/executes a downloaded file, only highlights it in Finder.
pub fn reveal(path: &str) {
    let _ = std::process::Command::new("open").arg("-R").arg(path).spawn();
}
