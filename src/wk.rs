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
    WKContentRuleList, WKContentRuleListStore, WKUserContentController, WKWebViewConfiguration,
    WKWebsiteDataStore,
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

pub fn config(store: &Store, adblock: bool) -> Config {
    unsafe {
        let c = WKWebViewConfiguration::new(mtm());
        c.setWebsiteDataStore(store);
        if adblock {
            set_manager_rules(&c.userContentController(), true);
        }
        Config(c)
    }
}

/// window.open target: WebKit's own configuration keeps `window.opener` working.
pub fn popup_config(features: NewWindowFeatures) -> Config {
    Config(features.opener.target_configuration)
}

/// Monterey's WebKit reports an old Safari version, so sites (e.g. Google) serve legacy layouts.
/// Native Safari identity only: a Chrome UA makes Google serve Blink-only layouts that break in WebKit.
/// `customUserAgent` covers the request header and `navigator.userAgent` alike.
#[cfg(target_os = "macos")]
const USER_AGENT: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.3 Safari/605.1.15";

/// Tab views (including adopted popups); the browser UI uses `ui_builder`.
pub fn builder<'a>(config: Config) -> WebViewBuilder<'a> {
    WebViewBuilder::new()
        .with_webview_configuration(config.0)
        .with_user_agent(USER_AGENT)
        .with_initialization_script_for_main_only(crate::adblock::DEFUSER, false)
}

/// Browser UI (toolbar, panel, lock screen, popovers): its own non-persistent data store
/// (`incognito` = `nonPersistentDataStore`), so no UI cache or storage ever reaches the disk.
pub fn ui_builder<'a>() -> WebViewBuilder<'a> {
    WebViewBuilder::new().with_incognito(true)
}

pub fn attach(b: WebViewBuilder, rect: Rect, window: &Window) -> wry::Result<WebView> {
    b.with_bounds(rect).build_as_child(window)
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
    _url: *mut AnyObject,
) {
    let Some(action) = (unsafe { action.as_ref() }) else { return };
    let blocked = unsafe {
        let known: Bool = msg_send![action, respondsToSelector: sel!(blockedLoad)];
        known.as_bool() && msg_send![action, blockedLoad]
    };
    let (true, Some(tab)) = (blocked, tab_of(web_view)) else { return };
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
        -1201 => "Certifikát má neplatný dátum (vypršal alebo ešte neplatí)",
        -1202 => "Certifikát nie je dôveryhodný pre túto doménu",
        -1203 => "Certifikát nevydala dôveryhodná autorita",
        -1204 => "Certifikát ešte nie je platný",
        -1200 => "Zabezpečené pripojenie sa nepodarilo nadviazať",
        -1205 | -1206 => "Server vyžaduje klientsky certifikát",
        _ => return None,
    })
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
        let url = (url as *mut NSString).as_ref().map(|u| u.to_string()).unwrap_or_default();
        (domain.as_ref().map(|d| d.to_string()).unwrap_or_default(), code, url)
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
            &TAB_KEY as *const u8 as *const c_void,
            ptr,
            objc2::ffi::OBJC_ASSOCIATION_RETAIN_NONATOMIC,
        )
    };
}

fn tab_of(web_view: *mut AnyObject) -> Option<u32> {
    if web_view.is_null() {
        return None;
    }
    let v = unsafe { objc2::ffi::objc_getAssociatedObject(web_view, &TAB_KEY as *const u8 as *const c_void) };
    unsafe { (v as *const NSString).as_ref() }?.to_string().parse().ok()
}

/// Adds `imp` for `sel` to the delegate's class unless the class already implements it.
unsafe fn add_method(cls: &AnyClass, sel: Sel, imp: Imp, types: &std::ffi::CStr) {
    if cls.instance_method(sel).is_none() {
        unsafe { objc2::ffi::class_addMethod(cls as *const AnyClass as *mut AnyClass, sel, imp, types.as_ptr()) };
    }
}

/// Reports blocked loads and certificate failures of `v` as tab `tab`: adds the methods to wry's
/// navigation delegate class and re-sets the delegate, so WebKit re-reads which methods exist.
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
    let errors = if secure_only { Vec::new() } else { vec!["Stránka obsahuje aj nezabezpečený (HTTP) obsah"] };
    Some((chain, errors))
}

/// Wipes cookies, cache and storage from the in-memory store.
pub fn clear_store(store: &Store) {
    unsafe {
        store.removeDataOfTypes_modifiedSince_completionHandler(
            &WKWebsiteDataStore::allWebsiteDataTypes(mtm()),
            &NSDate::distantPast(),
            &RcBlock::new(|| {}),
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
