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
};
use std::{
    cell::{Cell, RefCell},
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
pub struct Config(Retained<WKWebViewConfiguration>);

thread_local! {
    static RULES: RefCell<Option<Retained<WKContentRuleList>>> = const { RefCell::new(None) };
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
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.6 Safari/605.1.15";

/// Tab views (including adopted popups); the browser UI uses `ui_builder`.
pub fn builder<'a>(config: Config) -> WebViewBuilder<'a> {
    WebViewBuilder::new().with_webview_configuration(config.0).with_user_agent(USER_AGENT)
}

pub fn ui_builder<'a>() -> WebViewBuilder<'a> {
    WebViewBuilder::new()
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
                if let Some(list) = &*r.borrow() {
                    manager.addContentRuleList(list);
                }
            });
        }
    }
}

pub fn compile_rules(json: String, done: impl Fn() + 'static) {
    let Some(store) = (unsafe { WKContentRuleListStore::defaultStore(mtm()) }) else {
        return;
    };
    let block = RcBlock::new(move |list: *mut WKContentRuleList, err: *mut NSError| {
        if let Some(list) = unsafe { Retained::retain(list) } {
            RULES.with(|r| *r.borrow_mut() = Some(list));
            done();
        } else if let Some(err) = unsafe { err.as_ref() } {
            eprintln!("adblock: {}", err.localizedDescription());
        }
    });
    unsafe {
        store.compileContentRuleListForIdentifier_encodedContentRuleList_completionHandler(
            Some(ns_string!("crow-adblock")),
            Some(&NSString::from_str(&json)),
            Some(&block),
        );
    }
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
