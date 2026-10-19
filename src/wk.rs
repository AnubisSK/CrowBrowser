//! Direct WebKit glue: shared in-memory data store, cookie export/import, content blockers.

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
use std::{
    cell::{Cell, RefCell},
    ptr::NonNull,
    rc::Rc,
};

thread_local! {
    static RULES: RefCell<Option<Retained<WKContentRuleList>>> = const { RefCell::new(None) };
}

fn mtm() -> MainThreadMarker {
    MainThreadMarker::new().expect("main thread")
}

/// Nothing written to disk by WebKit; persistence goes through the encrypted vault.
pub fn new_store() -> Retained<WKWebsiteDataStore> {
    unsafe { WKWebsiteDataStore::nonPersistentDataStore(mtm()) }
}

pub fn config(store: &WKWebsiteDataStore, adblock: bool) -> Retained<WKWebViewConfiguration> {
    unsafe {
        let c = WKWebViewConfiguration::new(mtm());
        c.setWebsiteDataStore(store);
        if adblock {
            set_rules(&c.userContentController(), true);
        }
        c
    }
}

pub fn set_rules(manager: &WKUserContentController, on: bool) {
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

pub fn export_cookies(store: &WKWebsiteDataStore, done: impl Fn(Vec<CookieRec>) + 'static) {
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

pub fn import_cookies(store: &WKWebsiteDataStore, cookies: &[CookieRec], done: impl Fn() + 'static) {
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

/// Wipes cookies, cache and storage from the in-memory store.
pub fn clear_store(store: &WKWebsiteDataStore) {
    unsafe {
        store.removeDataOfTypes_modifiedSince_completionHandler(
            &WKWebsiteDataStore::allWebsiteDataTypes(mtm()),
            &NSDate::distantPast(),
            &RcBlock::new(|| {}),
        );
    }
}
