use crate::{
    adblock, cert, download,
    settings::{self, ENGINES, Settings},
    update,
    vault::{self, Bookmark, CookieRec, Download, HistoryEntry, MAX_HISTORY, Vault, VaultData},
    wk,
};
use serde::Deserialize;
use serde_json::json;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    path::{Path, PathBuf},
    rc::Rc,
    time::{Duration, Instant},
};
use tao::{
    event_loop::EventLoopProxy,
    window::{Fullscreen, Theme, Window},
};
use wry::{NewWindowFeatures, NewWindowResponse, PageLoadEvent, RGBA, Rect, WebView, dpi};
use zeroize::Zeroize;

const TABS_H: f64 = 36.0;
const NAV_H: f64 = 40.0;
const FIND_H: f64 = 36.0;
const ASK_H: f64 = 36.0;
const UPD_H: f64 = 36.0;
const SITE_W: f64 = 360.0;
const BG_DARK: RGBA = (0x1f, 0x1f, 0x23, 0xff);
const BG_LIGHT: RGBA = (0xf4, 0xf4, 0xf6, 0xff);
const ZOOM_STEPS: &[f64] = &[
    0.5, 0.67, 0.75, 0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0,
];
const TICK: Duration = Duration::from_secs(30);
const QUIT_TIMEOUT: Duration = Duration::from_secs(3);
/// Max wait for WebKit to confirm the cookie import before the session starts anyway.
const RESTORE_TIMEOUT: Duration = Duration::from_secs(3);
/// Favicon: polled after load (tab views have no IPC), a 32x32 PNG data URL of bounded size.
const FAV_POLL: Duration = Duration::from_millis(500);
const FAV_TRIES: u8 = 8;
const FAV_MAX: usize = 48 * 1024;
const FAV_READ: &str = "window.__crowIcon || ''";
const MAX_ALLOW: usize = 1000;
/// Reopen-closed-tab memory and download list are bounded (oldest entries go first).
const MAX_CLOSED_TABS: usize = 50;
const MAX_DOWNLOADS: usize = 500;
/// Shield estimates per blocked load (typical ad/tracker response size and request latency).
const BLOCK_BYTES: u64 = 50 * 1024;
const BLOCK_MS: u64 = 50;

const FIND_JS: &str = r#"(function (q, dir) {
  const sel = getSelection();
  if (!q) { sel.removeAllRanges(); return 0; }
  if (dir === 'new') sel.removeAllRanges();
  const ok = window.find(q, false, dir === 'prev', true, false, false, false);
  const t = (document.body ? document.body.innerText : '').toLowerCase(), s = q.toLowerCase();
  let n = 0, i = 0;
  while ((i = t.indexOf(s, i)) !== -1) { n++; i += s.length; }
  return ok ? n : 0;
})"#;

/// Runs in the page: tries <link rel=icon> candidates then /favicon.ico, re-encodes the first
/// one that loads into a PNG data URL (only CORS-readable images survive the canvas).
const FAV_JS: &str = r#"(function () {
  window.__crowIcon = '';
  const c = [...document.querySelectorAll('link[rel~="icon" i]')].map(l => l.href).filter(h => /^(https?|data):/i.test(h));
  if (/^https?:$/.test(location.protocol)) c.push(location.origin + '/favicon.ico');
  const next = () => {
    const src = c.shift();
    if (!src) { window.__crowIcon = '-'; return; }
    const img = new Image();
    if (!src.startsWith('data:')) img.crossOrigin = 'anonymous';
    img.onload = () => {
      try {
        const cv = document.createElement('canvas'); cv.width = cv.height = 32;
        cv.getContext('2d').drawImage(img, 0, 0, 32, 32);
        window.__crowIcon = cv.toDataURL('image/png');
      } catch (e) { next(); }
    };
    img.onerror = next;
    img.src = src;
  };
  next();
})()"#;

pub enum UserEvent {
    Menu(String),
    Ipc(String),
    PageLoad {
        tab: u32,
        started: bool,
        url: String,
    },
    Title {
        tab: u32,
        title: String,
    },
    NewTab(String),
    AdoptTab(u32),
    /// A download wants to write to disk; held until the user answers the toolbar prompt.
    /// `key` identifies the pending download inside `wk`, `tab` the requesting tab (if known).
    DownloadAsk {
        key: u32,
        tab: Option<u32>,
        url: String,
        name: String,
    },
    DownloadStarted {
        url: String,
        path: PathBuf,
    },
    DownloadDone {
        url: String,
        ok: bool,
    },
    FindResult(String),
    /// Exported cookies for a save: session token and cookie epoch at request time.
    Cookies(Vec<CookieRec>, After, u64, u64),
    /// Fresh cookie list for the site-info popover; `clear` = delete the site's cookies first.
    SiteCookies {
        cookies: Vec<CookieRec>,
        clear: bool,
    },
    CookiesRestored,
    /// Content-blocker list `id` is unchanged since WebKit compiled it: load, don't rebuild.
    RulesCached {
        id: &'static str,
        hash: String,
    },
    /// Freshly converted part `id` of a filter group (`parts` lists in total), for WebKit.
    RulesBuilt {
        id: &'static str,
        hash: String,
        json: String,
        parts: usize,
    },
    /// List has no sources (disabled).
    RulesOff(&'static str),
    RulesReady,
    /// Content blocker blocked loads (collected via `wk::take_blocked`).
    Blocked,
    /// Live cookies for the logins panel; `revoke` = delete this site's cookies first.
    VaultSessions {
        cookies: Vec<CookieRec>,
        revoke: Option<String>,
    },
    /// A tab's page entered/left element fullscreen (Linux; macOS uses WebKit's own window).
    Fullscreen { tab: u32, on: bool },
    /// Navigation refused by WebKit because of the server certificate.
    TlsError {
        tab: u32,
        url: String,
        reasons: Vec<&'static str>,
    },
    /// Result of a filter-list download the user asked for.
    /// Manual list refresh finished: failed lists as (`locales.json` key, list name).
    RulesNote(Vec<(&'static str, &'static str)>),
    /// Result of `FAV_READ` in a tab (JSON string).
    Icon {
        tab: u32,
        data: String,
    },
    Update {
        result: Result<Option<update::Info>, String>,
        manual: bool,
    },
    UpdateSaved(Option<PathBuf>),
}

#[derive(Clone, Copy, PartialEq)]
pub enum After {
    Nothing,
    Quit,
    Lock,
}

thread_local! {
    /// Popup webviews created inside WebKit's callback, waiting to be adopted as tabs.
    static PENDING: RefCell<Vec<(u32, WebView)>> = const { RefCell::new(Vec::new()) };
}

/// Everything a tab's WebKit callbacks need; cheap to clone into closures.
#[derive(Clone)]
struct TabCtx {
    window: Rc<Window>,
    proxy: EventLoopProxy<UserEvent>,
    next_id: Rc<Cell<u32>>,
    /// Global ad-block switch, for popups created inside WebKit callbacks.
    adblock: Rc<Cell<bool>>,
    /// UI language, baked into every UI view (also before unlock, see `settings::stored_language`).
    lang: Rc<RefCell<String>>,
}

impl TabCtx {
    fn next(&self) -> u32 {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        id
    }

    fn send(&self, e: UserEvent) {
        let _ = self.proxy.send_event(e);
    }

    /// Creates a tab view. Content rules (`blocking`) and certificate-failure reporting are
    /// attached before the first navigation starts, so no request leaves unfiltered.
    fn build(
        &self,
        id: u32,
        url: Option<&str>,
        config: wk::Config,
        rect: Rect,
        blocking: bool,
    ) -> wry::Result<WebView> {
        let (p1, p2) = (self.proxy.clone(), self.proxy.clone());
        let ctx = self.clone();
        let mut b = wk::builder(config)
            .with_background_color(bg_of(&self.window))
            .with_on_page_load_handler(move |ev, url| {
                let started = matches!(ev, PageLoadEvent::Started);
                let _ = p1.send_event(UserEvent::PageLoad {
                    tab: id,
                    started,
                    url,
                });
            })
            .with_document_title_changed_handler(move |title| {
                let _ = p2.send_event(UserEvent::Title { tab: id, title });
            })
            .with_new_window_req_handler(move |url, features| ctx.popup(url, features));
        // Linux: downloads are watched once on the shared context (wk::watch_downloads).
        #[cfg(target_os = "macos")]
        {
            let (p3, p4) = (self.proxy.clone(), self.proxy.clone());
            b = b
                .with_download_started_handler(move |url, path| {
                    // WebKit's suggested name goes through the same sanitizing as on Linux.
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    let name = download::sanitize(&name);
                    if wk::take_approved(&url) {
                        if let Some(dir) = path.parent().map(Path::to_path_buf) {
                            *path = download::unique_path(&dir, &name);
                        }
                        let _ = p3.send_event(UserEvent::DownloadStarted {
                            url,
                            path: path.clone(),
                        });
                        return true;
                    }
                    // No drive-by downloads: cancel, ask the user, retry on approval.
                    let _ = p3.send_event(UserEvent::DownloadAsk {
                        key: 0,
                        tab: Some(id),
                        url,
                        name,
                    });
                    false
                })
                .with_download_completed_handler(move |url, _, ok| {
                    let _ = p4.send_event(UserEvent::DownloadDone { url, ok });
                });
        }
        let view = wk::attach(b, rect, &self.window)?;
        wk::set_rules(&view, blocking);
        wk::watch(&view, id);
        if let Some(url) = url {
            view.load_url(url)?;
        }
        Ok(view)
    }

    /// window.open / target=_blank: a real WKWebView keeping `window.opener`
    /// (required by OAuth login popups), adopted as a new tab.
    fn popup(&self, url: String, features: NewWindowFeatures) -> NewWindowResponse {
        let id = self.next();
        let rect = content_rect(&self.window, TABS_H + NAV_H);
        // Rules go on before WebKit navigates the popup (adoption as a tab happens later).
        match self.build(
            id,
            None,
            wk::popup_config(features),
            rect,
            self.adblock.get(),
        ) {
            Ok(view) => {
                let response = wk::popup_response(&view);
                PENDING.with(|p| p.borrow_mut().push((id, view)));
                self.send(UserEvent::AdoptTab(id));
                response
            }
            Err(_) => {
                self.send(UserEvent::NewTab(url));
                NewWindowResponse::Deny
            }
        }
    }

    /// Internal UI with `window.ipc`. Strict CSP: no network, no external scripts/styles/frames;
    /// only its own inline code. Tab views never get an IPC handler, so wry never injects
    /// `window.ipc` there (popups inherit the opener tab's configuration, also without IPC).
    fn ui_view(&self, html: String, rect: Rect) -> Option<WebView> {
        const CSP: &str = "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; base-uri 'none'; form-action 'none'\">";
        // Dictionaries + `t()` go in first, so the page's own script can use them right away.
        let i18n = include_str!("i18n.js")
            .replace("/*LOCALES*/", include_str!("locales.json"))
            .replace("/*LANG*/", &js(&self.lang.borrow()))
            .replace("</", "<\\/");
        let html = html.replacen("<head>", &format!("<head>{CSP}<script>{i18n}</script>"), 1);
        let p = self.proxy.clone();
        let b = wk::ui_builder()
            .with_html(wk::ui_html(html))
            .with_background_color(bg_of(&self.window))
            .with_focused(true)
            .with_ipc_handler(move |req| {
                let _ = p.send_event(UserEvent::Ipc(req.into_body()));
            });
        wk::attach(b, rect, &self.window).ok()
    }
}

struct Ask {
    id: u32,
    key: u32,
    tab: Option<u32>,
    url: String,
    name: String,
    /// Executable/script type: shown as a warning in the prompt.
    risk: Option<String>,
}

struct Tab {
    id: u32,
    url: String,
    title: String,
    view: Option<WebView>, // None = unloaded to save memory
    hidden_since: Instant,
    loading: bool,
    icon: String,
    fav_tries: u8,
    /// Content blocker state last applied to `view` (None = unknown, apply on next check).
    blocking: Option<bool>,
    /// Loads blocked on the current page.
    blocked: u32,
    /// Engine-verified security of the committed page: "secure" | "mixed" | "broken" | "http" | "".
    tls: &'static str,
    /// Showing the certificate-error interstitial for `url`.
    tls_error: bool,
    /// Showing a built-in page (`url` = `crow:…`), rendered in the tab without network or IPC.
    internal: bool,
}

/// Every way a tab disappears (close, session lock, app exit) releases its view the same way.
impl Drop for Tab {
    fn drop(&mut self) {
        self.drop_view();
    }
}

impl Tab {
    fn new(id: u32, url: String, view: Option<WebView>) -> Self {
        Self {
            id,
            url,
            title: String::new(),
            view,
            hidden_since: Instant::now(),
            loading: false,
            icon: String::new(),
            fav_tries: 0,
            blocking: None,
            blocked: 0,
            tls: "",
            tls_error: false,
            internal: false,
        }
    }

    /// Frees the page (unload/close/lock); stops its reporting and releases the view.
    fn drop_view(&mut self) {
        if let Some(v) = self.view.take() {
            wk::forget(&v);
        }
    }

    fn label(&self) -> String {
        if !self.title.trim().is_empty() {
            return self.title.clone();
        }
        let host = self
            .url
            .split("://")
            .nth(1)
            .and_then(|r| r.split('/').next());
        // Empty = "new tab", worded by the toolbar in the UI language.
        host.filter(|h| !h.is_empty()).unwrap_or("").to_string()
    }
}

struct Session {
    vault: Vault,
    data: VaultData,
    store: wk::Store,
    toolbar: WebView,
    tabs: Vec<Tab>,
    active: usize,
    panel: Option<WebView>,
    panel_section: String,
    find_open: bool,
    zoom: f64,
    dirty: bool,
    saving: bool,
    /// Save requested while an export was running (strongest `After` wins), run afterwards.
    queued: Option<After>,
    /// Bumped whenever cookies are deleted; an export requested before that is stale.
    cookie_epoch: u64,
    /// Identifies this unlocked session: async results of a locked (older) session are dropped.
    token: u64,
    ready: bool,
    /// WebKit confirmed the cookie import (false if the session started on timeout).
    cookies_ok: bool,
    closed: Vec<String>,
    asks: Vec<Ask>,
    /// Popover under the address bar: site info (lock icon) or ad-block shield; left edge, height.
    site: Option<WebView>,
    shield_open: bool,
    /// Tab whose page is in element fullscreen (video): its view fills the window, toolbar hidden.
    video_fullscreen: Option<u32>,
    site_x: f64,
    site_h: f64,
    site_closed: Instant,
    fav_due: Option<Instant>,
    update: Option<update::Info>,
    update_bar: bool,
    update_busy: bool,
    /// UI language (shared with `TabCtx::lang`) for backend notifications.
    lang: Rc<RefCell<String>>,
}

impl Session {
    fn top(&self) -> f64 {
        TABS_H
            + NAV_H
            + if self.find_open { FIND_H } else { 0.0 }
            + if self.asks.is_empty() { 0.0 } else { ASK_H }
            + if self.update_bar { UPD_H } else { 0.0 }
    }

    fn poll_icons(&mut self, proxy: &EventLoopProxy<UserEvent>) {
        self.fav_due = None;
        for t in &mut self.tabs {
            if t.fav_tries == 0 {
                continue;
            }
            let Some(v) = &t.view else {
                t.fav_tries = 0;
                continue;
            };
            t.fav_tries -= 1;
            let (p, tab) = (proxy.clone(), t.id);
            let _ = v.evaluate_script_with_callback(FAV_READ, move |data| {
                let _ = p.send_event(UserEvent::Icon { tab, data });
            });
            if t.fav_tries > 0 {
                self.fav_due = Some(Instant::now() + FAV_POLL);
            }
        }
    }

    fn cur(&self) -> Option<&Tab> {
        self.tabs.get(self.active)
    }

    fn view(&self) -> Option<&WebView> {
        self.cur().and_then(|t| t.view.as_ref())
    }

    /// Re-evaluates the content blocker of every loaded tab (global toggle, allowlist, new rules).
    fn apply_rules_all(&mut self, cfg: &Settings) {
        for t in &mut self.tabs {
            t.blocking = None;
            let url = t.url.clone();
            apply_rules(cfg, t, &url);
        }
    }

    fn tab_idx(&self, id: u32) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }

    fn eval(&self, js: &str) {
        let _ = self.toolbar.evaluate_script(js);
    }

    fn toast(&self, msg: &str) {
        self.eval(&format!("toast({})", js(msg)));
    }

    fn tr(&self, key: &str, args: &[&str]) -> String {
        settings::tr(&self.lang.borrow(), key, args)
    }

    /// Notification from `locales.json`, formatted in the current UI language.
    fn note(&self, key: &str, args: &[&str]) {
        self.toast(&self.tr(key, args));
    }

    fn loaded(&self) -> impl Iterator<Item = &WebView> {
        self.tabs.iter().filter_map(|t| t.view.as_ref())
    }

    fn close_panel(&mut self) -> bool {
        self.site = None;
        if self.panel.take().is_none() {
            return false;
        }
        if let Some(v) = self.view() {
            let _ = v.focus();
        }
        true
    }

    fn snapshot(&mut self) {
        self.data.session.clear();
        self.data.active = 0;
        for (i, t) in self.tabs.iter().enumerate() {
            if t.url.starts_with("http") {
                if i == self.active {
                    self.data.active = self.data.session.len();
                }
                self.data.session.push(t.url.clone());
            }
        }
    }

    fn save_now(&mut self, cfg: &Settings) {
        self.snapshot();
        self.data.settings = Some(cfg.clone());
        if let Err(e) = self.vault.save(&self.data) {
            eprintln!("vault save failed: {e}");
        }
        self.dirty = false;
    }

    fn sync(&self, cfg: &Settings) {
        let cur = self.cur();
        let url = cur.map(|t| t.url.as_str()).unwrap_or("");
        let tabs: Vec<_> = self
            .tabs
            .iter()
            .map(|t| json!({ "id": t.id, "title": t.label(), "loading": t.loading, "asleep": t.view.is_none(), "icon": t.icon }))
            .collect();
        let state = json!({
            "tabs": tabs,
            "active": self.active,
            "url": url,
            "loading": cur.is_some_and(|t| t.loading),
            "tls": cur.map_or("", |t| t.tls),
            "login": logged_in(&self.data.cookies, &host_of(url)),
            "blocked": cur.map_or(0, |t| t.blocked),
            "bookmarked": self.data.bookmarks.iter().any(|b| b.url == url),
            "downloads": self.data.downloads.iter().filter(|d| d.state == "active").count(),
            "panel": if self.panel.is_some() { self.panel_section.as_str() } else { "" },
            "adblock": cfg.adblock,
            "host": if url.starts_with("http") { host_of(url) } else { String::new() },
            "siteAllowed": cfg.allowed(&host_of(url)),
            "ask": self.asks.first().map(|a| json!({
                "id": a.id, "name": a.name, "url": a.url, "more": self.asks.len() - 1, "risk": a.risk,
            })),
            "update": self.update.as_ref().filter(|_| self.update_bar).map(|u| json!({
                "version": u.version, "busy": self.update_busy, "asset": u.asset.is_some(),
            })),
        });
        self.eval(&format!("render({state})"));
    }

    /// Shield popover data (`get-adblock-stats`): current site and lifetime counts with estimates.
    fn shield_data(&self, cfg: &Settings) -> serde_json::Value {
        let url = self.cur().map_or("", |t| t.url.as_str());
        let host = if url.starts_with("http") {
            host_of(url)
        } else {
            String::new()
        };
        let tab = self.cur().map_or(0, |t| t.blocked) as u64;
        let total = self.data.blocked_total;
        json!({
            "host": host,
            "is_enabled": cfg.adblock,
            "allowed": cfg.allowed(&host),
            "available": wk::BLOCK_STATS,
            "tab_blocked": tab,
            "tab_bytes": tab * BLOCK_BYTES,
            "tab_ms": tab * BLOCK_MS,
            "total_blocked": total,
            "total_bytes": total * BLOCK_BYTES,
            "total_ms": total * BLOCK_MS,
        })
    }

    fn push_shield(&self, cfg: &Settings) {
        if let (Some(v), true) = (&self.site, self.shield_open) {
            let _ = v.evaluate_script(&format!("update({})", self.shield_data(cfg)));
        }
    }

    fn panel_data(&self, cfg: &Settings, section: &str) -> String {
        let history: Vec<&HistoryEntry> = self.data.history.iter().rev().take(3000).collect();
        let downloads: Vec<&Download> = self.data.downloads.iter().rev().collect();
        let engines: Vec<[&str; 2]> = ENGINES.iter().map(|e| [e.0, e.1]).collect();
        json!({
            "section": section,
            "settings": cfg,
            "engines": engines,
            "history": history,
            "bookmarks": self.data.bookmarks,
            "downloads": downloads,
            "allow": cfg.adblock_allow,
            "adblockLists": adblock::status(),
            "version": update::CURRENT,
            "update": self.update,
            "updateBusy": self.update_busy,
        })
        .to_string()
    }

    fn refresh_panel(&self, cfg: &Settings) {
        if let Some(p) = &self.panel {
            let _ = p.evaluate_script(&format!("setData({})", self.panel_data(cfg, "")));
        }
    }
}

pub struct App {
    ctx: TabCtx,
    cfg: Settings,
    lock: Option<WebView>,
    s: Option<Session>,
    pub exit: bool,
    next_tick: Instant,
    quit_at: Option<Instant>,
    restore_by: Option<Instant>,
    update_checked: bool,
    /// Source of `Session::token`: unique per unlock, never reused.
    sessions: u64,
}

impl App {
    pub fn new(window: Window, proxy: EventLoopProxy<UserEvent>) -> Self {
        refresh_rules(proxy.clone(), Vec::new(), false);
        let p = proxy.clone();
        wk::on_blocked(move || {
            let _ = p.send_event(UserEvent::Blocked);
        });
        let p = proxy.clone();
        wk::on_fullscreen(move |tab, on| {
            let _ = p.send_event(UserEvent::Fullscreen { tab, on });
        });
        let p = proxy.clone();
        wk::on_tls_error(move |tab, url, reasons| {
            let _ = p.send_event(UserEvent::TlsError { tab, url, reasons });
        });
        #[cfg(target_os = "linux")]
        {
            let (p0, p1, p2) = (proxy.clone(), proxy.clone(), proxy.clone());
            wk::watch_downloads(
                &wk::new_store(),
                move |key, url, name| {
                    let _ = p0.send_event(UserEvent::DownloadAsk {
                        key,
                        tab: None,
                        url,
                        name,
                    });
                },
                move |url, path| {
                    let _ = p1.send_event(UserEvent::DownloadStarted { url, path });
                },
                move |url, ok| {
                    let _ = p2.send_event(UserEvent::DownloadDone { url, ok });
                },
            );
        }
        let ctx = TabCtx {
            window: Rc::new(window),
            proxy,
            next_id: Rc::new(Cell::new(1)),
            adblock: Rc::new(Cell::new(true)),
            lang: Rc::new(RefCell::new(settings::stored_language())),
        };
        let mut app = Self {
            ctx,
            cfg: Settings::default(), // real settings live in the vault
            lock: None,
            s: None,
            exit: false,
            next_tick: Instant::now() + TICK,
            quit_at: None,
            restore_by: None,
            update_checked: false,
            sessions: 0,
        };
        app.show_lock();
        app
    }

    fn show_lock(&mut self) {
        let mode = if Vault::exists() { "unlock" } else { "create" };
        let html = include_str!("lock.html")
            .replace("/*MODE*/", &js(mode))
            .replace("/*LOGO*/", include_str!("../assets/logo.svg"));
        self.lock = self.ctx.ui_view(html, full_rect(&self.ctx.window));
    }

    /// Lock-screen error (`key` in `locales.json`), shown in the UI language.
    fn lock_fail(&self, key: &str) {
        if let Some(l) = &self.lock {
            let _ = l.evaluate_script(&format!("fail(t({}))", js(key)));
        }
    }

    pub fn wake(&self) -> Instant {
        let fav = self.s.as_ref().and_then(|s| s.fav_due);
        [self.quit_at, self.restore_by, fav]
            .into_iter()
            .flatten()
            .fold(self.next_tick, Instant::min)
    }

    pub fn resize(&self) {
        let w = &self.ctx.window;
        if let Some(l) = &self.lock {
            wk::set_bounds(l, full_rect(w));
        }
        if let Some(s) = &self.s {
            let full = s.video_fullscreen.and_then(|id| s.tab_idx(id)).and_then(|i| s.tabs[i].view.as_ref());
            let _ = s.toolbar.set_visible(full.is_none());
            wk::set_bounds(&s.toolbar, toolbar_rect(w, s.top()));
            let c = content_rect(w, s.top());
            for v in s.loaded().chain(s.panel.as_ref()) {
                wk::set_bounds(v, c);
            }
            if let Some(v) = full {
                wk::set_bounds(v, full_rect(w));
            }
            if let Some(v) = &s.site {
                wk::set_bounds(v, site_rect(w, s.site_x, s.site_h));
            }
        }
    }

    /// Greys out the drawn window controls (Linux) like macOS does for inactive windows.
    pub fn focused(&self, on: bool) {
        let js = format!("setFocus({on})");
        for v in self.lock.iter().chain(self.s.as_ref().map(|s| &s.toolbar)) {
            let _ = v.evaluate_script(&js);
        }
    }

    pub fn handle(&mut self, ev: UserEvent) {
        match ev {
            UserEvent::Menu(cmd) => self.command(&cmd),
            UserEvent::Ipc(mut msg) => {
                self.command(&msg);
                msg.zeroize(); // may contain a password
            }
            UserEvent::PageLoad { tab, started, url } => self.page_load(tab, started, url),
            UserEvent::Title { tab, title } => self.title(tab, title),
            UserEvent::NewTab(url) => self.open_tab(url),
            UserEvent::AdoptTab(id) => self.adopt(id),
            UserEvent::DownloadAsk {
                key,
                tab,
                url,
                name,
            } => self.download_ask(key, tab, url, name),
            UserEvent::DownloadStarted { url, path } => self.download_started(url, path),
            UserEvent::DownloadDone { url, ok } => self.download_done(url, ok),
            UserEvent::FindResult(r) => {
                if let Some(s) = &self.s {
                    s.eval(&format!(
                        "findCount({})",
                        r.trim().parse::<u32>().unwrap_or(0)
                    ));
                }
            }
            UserEvent::Cookies(c, after, token, epoch) => self.saved(c, after, token, epoch),
            UserEvent::CookiesRestored => self.restored(true),
            UserEvent::SiteCookies { cookies, clear } => self.site_cookies(cookies, clear),
            UserEvent::VaultSessions { cookies, revoke } => self.vault_sessions(cookies, revoke),
            UserEvent::RulesCached { id, hash } => {
                let (p, p2) = (self.ctx.proxy.clone(), self.ctx.proxy.clone());
                let ok = move || {
                    let _ = p.send_event(UserEvent::RulesReady);
                };
                // WebKit lost its compiled copy: convert the group again, use this part.
                let fail = move || {
                    let (p, hash) = (p2.clone(), hash.clone());
                    std::thread::spawn(move || {
                        let Some((g, i)) = adblock::group_of(id) else {
                            return;
                        };
                        let Some(jsons) = adblock::build(g) else {
                            return;
                        };
                        let parts = jsons.len();
                        let e = match jsons.into_iter().nth(i) {
                            Some(json) => UserEvent::RulesBuilt {
                                id,
                                hash,
                                json,
                                parts,
                            },
                            None => UserEvent::RulesOff(id),
                        };
                        let _ = p.send_event(e);
                    });
                };
                wk::load_rules(id, ok, fail);
            }
            UserEvent::RulesBuilt {
                id,
                hash,
                json,
                parts,
            } => {
                let p = self.ctx.proxy.clone();
                wk::compile_rules(id, json, move || {
                    adblock::mark_compiled(id, &hash, parts);
                    let _ = p.send_event(UserEvent::RulesReady);
                });
            }
            UserEvent::RulesOff(id) => {
                wk::drop_rules(id);
                adblock::forget(id);
                self.handle(UserEvent::RulesReady);
            }
            UserEvent::Blocked => {
                let blocked = wk::take_blocked();
                let Some(s) = self.s.as_mut() else { return };
                for (id, n) in blocked {
                    if let Some(i) = s.tab_idx(id) {
                        s.tabs[i].blocked += n;
                        s.data.blocked_total += n as u64;
                        s.dirty = true;
                    }
                }
                let n = s.cur().map_or(0, |t| t.blocked);
                s.eval(&format!("setBlocked({n})"));
                s.push_shield(&self.cfg);
            }
            UserEvent::TlsError { tab, url, reasons } => self.tls_error(tab, url, reasons),
            UserEvent::Fullscreen { tab, on } => {
                let Some(s) = self.s.as_mut() else { return };
                s.video_fullscreen = if on { Some(tab) } else { s.video_fullscreen.filter(|t| *t != tab) };
                if on {
                    s.site = None;
                }
                // WebKit fullscreens the window itself; this makes the view fill it.
                self.resize();
            }
            UserEvent::RulesNote(errors) => {
                if let Some(s) = &self.s {
                    if errors.is_empty() {
                        s.note("adblock_lists_updated", &[]);
                    } else {
                        let msg: Vec<_> = errors.iter().map(|&(k, name)| s.tr(k, &[name])).collect();
                        s.toast(&msg.join(", "));
                    }
                }
            }
            UserEvent::RulesReady => {
                if let Some(s) = self.s.as_mut() {
                    s.apply_rules_all(&self.cfg);
                    s.refresh_panel(&self.cfg);
                }
            }
            UserEvent::Icon { tab, data } => self.icon(tab, data),
            UserEvent::Update { result, manual } => self.update_result(result, manual),
            UserEvent::UpdateSaved(path) => {
                let Some(s) = self.s.as_mut() else { return };
                s.update_busy = false;
                match path {
                    // Installer is never run by Crow; the user opens it from the file manager.
                    Some(p) => {
                        wk::reveal(&p.to_string_lossy());
                        s.update_bar = false;
                        self.resize();
                    }
                    None => s.note("update_download_failed", &[]),
                }
                if let Some(s) = &self.s {
                    s.sync(&self.cfg);
                    s.refresh_panel(&self.cfg);
                }
            }
        }
    }

    pub fn timer(&mut self) {
        let now = Instant::now();
        if self.quit_at.is_some_and(|q| now >= q) {
            // Cookie export never answered; save what we have.
            if let Some(s) = &mut self.s {
                s.save_now(&self.cfg);
            }
            self.exit = true;
            return;
        }
        if self.restore_by.is_some_and(|r| now >= r) {
            eprintln!("cookie restore timed out; starting session without waiting");
            self.restored(false);
        }
        if let Some(s) = self.s.as_mut()
            && s.fav_due.is_some_and(|d| now >= d)
        {
            s.poll_icons(&self.ctx.proxy);
        }
        if now < self.next_tick {
            return;
        }
        self.next_tick = now + TICK;
        let Some(s) = self.s.as_mut() else { return };
        if self.cfg.unload > 0 {
            let limit = Duration::from_secs(self.cfg.unload as u64 * 60);
            let mut changed = false;
            for (i, t) in s.tabs.iter_mut().enumerate() {
                if i != s.active && t.view.is_some() && t.hidden_since.elapsed() >= limit {
                    t.drop_view();
                    t.loading = false;
                    changed = true;
                }
            }
            if changed {
                s.sync(&self.cfg);
            }
        }
        if s.dirty && !s.saving && s.ready {
            self.persist(After::Nothing);
        }
    }

    /// NSApp terminated from outside (Dock, logout): synchronous save with last known cookies.
    pub fn shutdown(&mut self) {
        if let Some(s) = &mut self.s {
            s.save_now(&self.cfg);
        }
    }

    fn command(&mut self, cmd: &str) {
        let (name, arg) = cmd.split_once(':').unwrap_or((cmd, ""));
        match name {
            "drag" => {
                let _ = self.ctx.window.drag_window();
            }
            "quit" | "win-close" => self.quit(),
            "win-min" => self.ctx.window.set_minimized(true),
            "win-max" => {
                let w = &self.ctx.window;
                w.set_maximized(!w.is_maximized());
            }
            "fullscreen" => {
                let w = &self.ctx.window;
                w.set_fullscreen(if w.fullscreen().is_some() {
                    None
                } else {
                    Some(Fullscreen::Borderless(None))
                });
            }
            "unlock" | "create" | "reset" if self.s.is_none() => self.lock_cmd(name, arg),
            _ if self.s.as_ref().is_some_and(|s| s.ready) => self.session_cmd(name, arg),
            _ => {}
        }
    }

    fn lock_cmd(&mut self, name: &str, arg: &str) {
        match name {
            "unlock" => match Vault::unlock(arg) {
                Ok((vault, data)) => self.start(vault, data),
                Err(e) => self.lock_fail(e),
            },
            "create" if !Vault::exists() => {
                if arg.chars().count() < 8 {
                    return self.lock_fail("pw_too_short");
                }
                let vault = Vault::create(arg);
                let data = VaultData::default();
                match vault.save(&data) {
                    Ok(()) => self.start(vault, data),
                    Err(_) => self.lock_fail("vault_save_failed"),
                }
            }
            "reset" => {
                Vault::destroy();
                self.show_lock();
            }
            _ => {}
        }
    }

    fn start(&mut self, vault: Vault, mut data: VaultData) {
        let legacy = data.settings.is_none();
        self.cfg = data
            .settings
            .take()
            .or_else(Settings::take_legacy)
            .unwrap_or_default();
        self.ctx.adblock.set(self.cfg.adblock);
        // The vault is authoritative; the plain copy only serves the next lock screen.
        settings::store_language(&self.cfg.language);
        *self.ctx.lang.borrow_mut() = self.cfg.language.clone();
        let w = &self.ctx.window;
        wk::set_theme(w, theme_of(&self.cfg.theme));
        let html = include_str!("toolbar.html").to_string();
        let Some(toolbar) = self.ctx.ui_view(html, toolbar_rect(w, TABS_H + NAV_H)) else {
            return self.lock_fail("window_failed");
        };
        self.lock = None;
        let store = wk::new_store();
        let p = self.ctx.proxy.clone();
        wk::import_cookies(&store, &data.cookies, move || {
            let _ = p.send_event(UserEvent::CookiesRestored);
        });
        // If WebKit never answers, the session still opens (see `timer`).
        self.restore_by = Some(Instant::now() + RESTORE_TIMEOUT);
        self.sessions += 1;
        self.s = Some(Session {
            lang: self.ctx.lang.clone(),
            token: self.sessions,
            vault,
            data,
            store,
            toolbar,
            tabs: Vec::new(),
            active: 0,
            panel: None,
            panel_section: String::new(),
            find_open: false,
            zoom: self.cfg.zoom,
            dirty: legacy,
            saving: false,
            queued: None,
            cookie_epoch: 0,
            ready: false,
            cookies_ok: false,
            closed: Vec::new(),
            asks: Vec::new(),
            site: None,
            shield_open: false,
            video_fullscreen: None,
            site_x: 0.0,
            site_h: 0.0,
            site_closed: Instant::now(),
            fav_due: None,
            update: None,
            update_bar: false,
            update_busy: false,
        });
    }

    /// `confirmed`: WebKit finished the import; false = forced by `RESTORE_TIMEOUT`.
    fn restored(&mut self, confirmed: bool) {
        self.restore_by = None;
        let Some(s) = self.s.as_mut() else { return };
        s.cookies_ok |= confirmed;
        if s.ready {
            return;
        }
        s.ready = true;
        let restore = self.cfg.startup == "last" && !s.data.session.is_empty();
        let urls = if restore {
            s.data.session.to_vec()
        } else {
            vec![self.cfg.homepage.clone()]
        };
        let active = if restore {
            s.data.active.min(urls.len() - 1)
        } else {
            0
        };
        // Only the active tab is loaded; the rest load on first click.
        for url in urls {
            s.tabs.push(Tab::new(self.ctx.next(), url, None));
        }
        s.active = active;
        self.activate(active);
        if self.cfg.updates && !self.update_checked {
            self.update_checked = true;
            update::check(self.ctx.proxy.clone(), false);
            let stale = adblock::stale(self.cfg.adblock_easylist, self.cfg.adblock_hosts);
            if !stale.is_empty() {
                refresh_rules(self.ctx.proxy.clone(), stale, false);
            }
        }
    }

    fn activate(&mut self, idx: usize) {
        let Some(s) = self.s.as_mut() else { return };
        if idx >= s.tabs.len() {
            return;
        }
        s.close_panel();
        // Switching away from a fullscreen video ends it (WebKit then restores the window).
        if let Some(v) = s.video_fullscreen.take().and_then(|id| s.tab_idx(id)).and_then(|i| s.tabs[i].view.as_ref()) {
            let _ = v.evaluate_script("document.webkitExitFullscreen && document.webkitExitFullscreen()");
        }
        if s.active != idx {
            if let Some(t) = s.tabs.get_mut(s.active) {
                t.hidden_since = Instant::now();
                if let Some(v) = &t.view {
                    let _ = v.set_visible(false);
                }
            }
        }
        s.active = idx;
        let rect = content_rect(&self.ctx.window, s.top());
        let tab = &mut s.tabs[idx];
        if tab.view.is_none() {
            let on = blocks(&self.cfg, &tab.url);
            let config = wk::config(&s.store, on);
            let page = internal_page(&tab.url, &self.cfg.language);
            let url = if page.is_some() { None } else { Some(tab.url.as_str()) };
            tab.view = self.ctx.build(tab.id, url, config, rect, on).ok();
            tab.blocking = Some(on);
            if let (Some(html), Some(v)) = (page, &tab.view) {
                tab.internal = true;
                tab.url = CHANGELOG_URL.into();
                let _ = v.load_html(&html);
            }
            if let Some(v) = &tab.view {
                let _ = v.zoom(s.zoom);
            }
        }
        if let Some(v) = &tab.view {
            wk::set_bounds(v, rect);
            let _ = v.set_visible(true);
            let _ = v.focus();
        }
        s.sync(&self.cfg);
    }

    fn open_tab(&mut self, url: String) {
        let Some(s) = self.s.as_mut() else { return };
        let at = (s.active + 1).min(s.tabs.len());
        s.tabs.insert(at, Tab::new(self.ctx.next(), url, None));
        s.dirty = true;
        self.activate(at);
    }

    fn adopt(&mut self, id: u32) {
        let view = PENDING.with(|p| {
            let mut p = p.borrow_mut();
            let i = p.iter().position(|(pid, _)| *pid == id)?;
            Some(p.remove(i).1)
        });
        let (Some(view), Some(s)) = (view, self.s.as_mut()) else {
            return;
        };
        let _ = view.zoom(s.zoom);
        let mut tab = Tab::new(id, String::new(), Some(view));
        apply_rules(&self.cfg, &mut tab, "");
        let at = (s.active + 1).min(s.tabs.len());
        s.tabs.insert(at, tab);
        self.activate(at);
    }

    fn close_tab(&mut self, idx: usize) {
        let Some(s) = self.s.as_mut() else { return };
        if idx >= s.tabs.len() {
            return;
        }
        let mut tab = s.tabs.remove(idx);
        if tab.url.starts_with("http") {
            s.closed.push(tab.url.clone());
            if s.closed.len() > MAX_CLOSED_TABS {
                s.closed.remove(0);
            }
        }
        tab.drop_view();
        s.dirty = true;
        if s.tabs.is_empty() {
            s.active = 0;
            return self.open_tab(self.cfg.homepage.clone());
        }
        s.active = match idx.cmp(&s.active) {
            std::cmp::Ordering::Less => s.active - 1,
            std::cmp::Ordering::Equal => idx.min(s.tabs.len() - 1),
            std::cmp::Ordering::Greater => s.active,
        };
        let active = s.active;
        self.activate(active);
    }

    fn navigate(&mut self, url: String) {
        let Some(s) = self.s.as_mut() else { return };
        s.close_panel();
        if let Some(t) = s.tabs.get_mut(s.active)
            && let Some(html) = internal_page(&url, &self.cfg.language)
        {
            // Built-in page: shown under its own address, no request leaves the browser.
            t.url = CHANGELOG_URL.into();
            t.internal = true;
            t.tls = "";
            t.icon.clear();
            if let Some(v) = &t.view {
                let _ = v.load_html(&html);
            }
            return s.sync(&self.cfg);
        }
        if let Some(t) = s.tabs.get_mut(s.active) {
            match &t.view {
                // Address bar (and TLS badge) follow the finished load, not the request.
                Some(v) => {
                    let _ = v.load_url(&url);
                    let _ = v.focus();
                }
                None => t.url = url,
            }
        }
        s.sync(&self.cfg);
    }

    fn page_load(&mut self, id: u32, started: bool, url: String) {
        let Some(s) = self.s.as_mut() else { return };
        let Some(i) = s.tab_idx(id) else { return };
        let t = &mut s.tabs[i];
        // The interstitial / built-in page itself (about:blank) keeps the shown URL and state.
        let interstitial = (t.tls_error || t.internal) && (url.is_empty() || url == "about:blank");
        // Allowlist follows the page being loaded: switched before its subresources are requested.
        if !url.is_empty() && !interstitial {
            apply_rules(&self.cfg, t, &url);
        }
        if started && !interstitial {
            t.tls_error = false;
            t.internal = false;
        }
        if !started && interstitial {
            t.loading = false;
            return s.sync(&self.cfg);
        }
        if started {
            // Anti-spoofing: the address bar keeps the last finished URL until this load
            // completes; only the spinner changes.
            t.loading = true;
            // New page: per-page blocked count starts over.
            t.blocked = 0;
            if i == s.active {
                if s.shield_open {
                    s.push_shield(&self.cfg); // stays open across its own on/off reload
                } else {
                    s.site = None; // describes the page being left
                }
                s.eval("setBlocked(0)");
            }
            let js = format!("setLoading({id},true)");
            return s.eval(&js);
        }
        t.loading = false;
        if !url.is_empty() {
            t.url = url.clone();
        }
        // Lock only for a certificate WebKit validated, never from the "https://" prefix.
        t.tls = match (url.split(':').next(), &t.view) {
            (Some("https"), Some(v)) => match wk::tls(v) {
                wk::Tls::Secure => "secure",
                wk::Tls::Mixed => "mixed",
                wk::Tls::Unverified => "broken",
            },
            (Some("https"), None) => "broken",
            (Some("http"), _) => "http",
            _ => "",
        };
        match &t.view {
            Some(v) if url.starts_with("http") => {
                let _ = v.evaluate_script(FAV_JS);
                t.fav_tries = FAV_TRIES;
                s.fav_due.get_or_insert_with(|| Instant::now() + FAV_POLL);
            }
            _ => t.icon.clear(),
        }
        if url.starts_with("http") {
            let title = t.title.clone();
            let h = &mut s.data.history;
            if h.last().is_none_or(|e| e.url != url) {
                h.push(HistoryEntry {
                    url,
                    title,
                    time: vault::now_ms(),
                });
                if h.len() > MAX_HISTORY {
                    h.drain(..h.len() - MAX_HISTORY);
                }
            }
            s.dirty = true;
        }
        s.sync(&self.cfg);
    }

    fn title(&mut self, id: u32, title: String) {
        let Some(s) = self.s.as_mut() else { return };
        let Some(i) = s.tab_idx(id) else { return };
        let t = &mut s.tabs[i];
        t.title = title;
        if let Some(e) = s.data.history.last_mut().filter(|e| e.url == t.url) {
            e.title = t.title.clone();
        }
        s.sync(&self.cfg);
    }

    /// WebKit refused the navigation (certificate). Shows a static interstitial in the tab (no
    /// IPC, no way to proceed); the address bar shows the refused URL marked as broken.
    fn tls_error(&mut self, id: u32, url: String, reasons: Vec<&'static str>) {
        let Some(s) = self.s.as_mut() else { return };
        let Some(i) = s.tab_idx(id) else { return };
        let t = &mut s.tabs[i];
        t.loading = false;
        t.tls_error = true;
        t.tls = "broken";
        t.icon.clear();
        if !url.is_empty() {
            t.url = url;
        }
        if let Some(v) = &t.view {
            let _ = v.load_html(&interstitial(&s.lang.borrow(), &host_of(&t.url), &reasons));
        }
        if i == s.active {
            s.site = None;
        }
        s.sync(&self.cfg);
    }

    fn icon(&mut self, id: u32, res: String) {
        let Some(s) = self.s.as_mut() else { return };
        let Some(i) = s.tab_idx(id) else { return };
        let data: String = serde_json::from_str(&res).unwrap_or_default();
        if data.is_empty() {
            return; // still loading, polled again
        }
        let t = &mut s.tabs[i];
        t.fav_tries = 0;
        let valid = data
            .strip_prefix("data:image/png;base64,")
            .is_some_and(|b| {
                b.len() <= FAV_MAX
                    && b.bytes()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'/' | b'='))
            });
        let icon = if valid { data } else { String::new() };
        if t.icon != icon {
            t.icon = icon;
            s.sync(&self.cfg);
        }
    }

    fn update_result(&mut self, result: Result<Option<update::Info>, String>, manual: bool) {
        let Some(s) = self.s.as_mut() else { return };
        match result {
            Ok(Some(info)) => {
                s.update = Some(info);
                if !s.update_bar {
                    s.update_bar = true;
                    self.resize();
                }
            }
            Ok(None) if manual => s.note("up_to_date", &[update::CURRENT]),
            Err(e) if manual => s.note("update_check_failed", &[&s.tr(&e, &[])]),
            _ => {}
        }
        if let Some(s) = &self.s {
            s.sync(&self.cfg);
            s.refresh_panel(&self.cfg);
        }
    }

    /// UI confirmation after an ad-block change: toolbar, panel, site popover; saved to the vault.
    fn adblock_changed(&mut self) {
        let Some(s) = self.s.as_ref() else { return };
        s.sync(&self.cfg);
        s.refresh_panel(&self.cfg);
        if s.shield_open {
            s.push_shield(&self.cfg);
        } else if s.site.is_some() {
            self.request_site_cookies(false);
        }
        self.persist(After::Nothing);
    }

    /// Exports live cookies for the logins panel (`revoke`: delete that site's first).
    fn request_sessions(&self, revoke: Option<String>) {
        let Some(s) = self.s.as_ref() else { return };
        let p = self.ctx.proxy.clone();
        let revoke = RefCell::new(revoke);
        wk::export_cookies(&s.store, move |cookies| {
            let _ = p.send_event(UserEvent::VaultSessions {
                cookies,
                revoke: revoke.borrow_mut().take(),
            });
        });
    }

    /// Logins panel data: one entry per site with cookies. Sanitized: no cookie names or values
    /// leave Rust. Crow stores no passwords, so there is no username to show.
    fn vault_sessions(&mut self, cookies: Vec<CookieRec>, revoke: Option<String>) {
        let Some(s) = self.s.as_mut() else { return };
        if let Some(site) = revoke {
            s.cookie_epoch += 1;
            let (gone, keep): (Vec<_>, Vec<_>) = cookies
                .into_iter()
                .partition(|c| site_of(&c.domain) == site);
            // Vault first (atomic .vault.tmp -> rename in `save_now`), then WebKit's live store.
            if s.cookies_ok {
                s.data.cookies = keep;
            } else {
                s.data.cookies.retain(|c| site_of(&c.domain) != site);
            }
            s.save_now(&self.cfg);
            s.note("signed_out", &[&site]);
            s.sync(&self.cfg);
            // Once WebKit has deleted them, send the panel the fresh list.
            let p = self.ctx.proxy.clone();
            let store = s.store.clone();
            return wk::delete_cookies(&s.store, &gone, move || {
                let p = p.clone();
                wk::export_cookies(&store, move |cookies| {
                    let _ = p.send_event(UserEvent::VaultSessions {
                        cookies,
                        revoke: None,
                    });
                });
            });
        }
        let mut last: HashMap<String, u64> = HashMap::new();
        for e in &s.data.history {
            let t = last.entry(site_of(&host_of(&e.url))).or_default();
            *t = (*t).max(e.time);
        }
        let mut sites: Vec<(String, bool, usize)> = Vec::new();
        for c in &cookies {
            let site = site_of(&c.domain);
            match sites.iter_mut().find(|(d, ..)| *d == site) {
                Some(e) => {
                    e.1 |= is_login_cookie(c);
                    e.2 += 1;
                }
                None => sites.push((site, is_login_cookie(c), 1)),
            }
        }
        let mut list: Vec<_> = sites
            .into_iter()
            .filter(|(d, ..)| !d.is_empty())
            .map(|(domain, has_session, count)| {
                let used = last.get(&domain).copied();
                json!({ "domain": domain, "username": null, "has_session": has_session, "cookies": count, "last_used": used })
            })
            .collect();
        list.sort_by_key(|e| {
            (
                !e["has_session"].as_bool().unwrap_or(false),
                std::cmp::Reverse(e["last_used"].as_u64().unwrap_or(0)),
            )
        });
        if let Some(p) = &s.panel {
            let _ = p.evaluate_script(&format!("setSessions({})", serde_json::Value::Array(list)));
        }
    }

    fn request_site_cookies(&mut self, clear: bool) {
        let Some(s) = self.s.as_ref() else { return };
        let p = self.ctx.proxy.clone();
        wk::export_cookies(&s.store, move |cookies| {
            let _ = p.send_event(UserEvent::SiteCookies { cookies, clear });
        });
    }

    /// Builds (or refreshes) the site-info popover for the active tab.
    fn site_cookies(&mut self, cookies: Vec<CookieRec>, clear: bool) {
        let Some(s) = self.s.as_mut() else { return };
        let url = s.cur().map(|t| t.url.clone()).unwrap_or_default();
        let host = host_of(&url);
        if clear {
            s.cookie_epoch += 1;
            let (site, rest): (Vec<_>, Vec<_>) = cookies
                .into_iter()
                .partition(|c| cookie_for(&host, &c.domain));
            s.dirty = true;
            let p = self.ctx.proxy.clone();
            let rest = RefCell::new(Some(rest));
            return wk::delete_cookies(&s.store, &site, move || {
                if let Some(cookies) = rest.take() {
                    let _ = p.send_event(UserEvent::SiteCookies {
                        cookies,
                        clear: false,
                    });
                }
            });
        }
        let (certs, errors) = s
            .view()
            .filter(|_| url.starts_with("https:"))
            .and_then(wk::cert_chain)
            .map(|(chain, errors)| {
                (
                    chain.iter().filter_map(|d| cert::summarize(d)).collect(),
                    errors,
                )
            })
            .unwrap_or((Vec::new(), Vec::new()));
        let data = json!({
            "url": url,
            "host": host,
            "scheme": url.split(':').next().unwrap_or(""),
            "certs": certs,
            "errors": errors,
            "cookies": cookies.iter().filter(|c| cookie_for(&host, &c.domain)).count(),
            "adblock": self.cfg.adblock,
            "allowed": self.cfg.allowed(&host),
            "tls": s.cur().map_or("", |t| t.tls),
        });
        if let Some(v) = &s.site {
            let _ = v.evaluate_script(&format!("update({data})"));
            return;
        }
        let html = include_str!("siteinfo.html")
            .replace("/*DATA*/", &data.to_string().replace("</", "<\\/"));
        let h = if s.site_h > 0.0 { s.site_h } else { 260.0 };
        s.site = self
            .ctx
            .ui_view(html, site_rect(&self.ctx.window, s.site_x, h));
    }

    fn download_ask(&mut self, key: u32, tab: Option<u32>, url: String, name: String) {
        let Some(s) = self.s.as_mut() else {
            return wk::decide_download(key, false, &url, None);
        };
        let id = self.ctx.next();
        let name = download::sanitize(&name);
        let risk = download::risk(&name);
        s.asks.push(Ask {
            id,
            key,
            tab,
            url,
            name,
            risk,
        });
        if s.asks.len() == 1 {
            self.resize();
        }
        if let Some(s) = &self.s {
            s.sync(&self.cfg);
        }
    }

    fn download_started(&mut self, url: String, path: PathBuf) {
        let Some(s) = self.s.as_mut() else { return };
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        s.data.downloads.push(Download {
            url,
            path: path.to_string_lossy().into_owned(),
            state: "active".into(),
            time: vault::now_ms(),
        });
        // Oldest finished entries go first; running downloads are still looked up by URL.
        while s.data.downloads.len() > MAX_DOWNLOADS {
            let Some(i) = s.data.downloads.iter().position(|d| d.state != "active") else { break };
            s.data.downloads.remove(i);
        }
        s.dirty = true;
        s.note("downloading_prefix", &[&name]);
        s.sync(&self.cfg);
        s.refresh_panel(&self.cfg);
    }

    fn download_done(&mut self, url: String, ok: bool) {
        let Some(s) = self.s.as_mut() else { return };
        let Some(d) = s
            .data
            .downloads
            .iter_mut()
            .rev()
            .find(|d| d.url == url && d.state == "active")
        else {
            return;
        };
        d.state = if ok { "done" } else { "failed" }.into();
        if ok {
            download::quarantine(Path::new(&d.path)); // Gatekeeper inspects it on first open
        }
        let name = d.path.rsplit('/').next().unwrap_or("").to_string();
        s.dirty = true;
        s.note(if ok { "download_complete" } else { "download_failed" }, &[&name]);
        s.sync(&self.cfg);
        s.refresh_panel(&self.cfg);
    }

    fn quit(&mut self) {
        if self.s.is_some() {
            self.quit_at = Some(Instant::now() + QUIT_TIMEOUT);
            self.persist(After::Quit);
        } else {
            self.exit = true;
        }
    }

    /// Exports cookies from WebKit (async), then encrypts everything to disk.
    fn persist(&mut self, after: After) {
        let Some(s) = self.s.as_mut() else { return };
        if !s.ready {
            // Cookies not restored yet: don't overwrite them with a partial set.
            s.save_now(&self.cfg);
            return self.after_save(after);
        }
        // One export at a time: later requests are queued, so an older cookie list can never
        // arrive after (and overwrite) a newer one.
        if s.saving {
            s.queued = Some(stronger(s.queued.unwrap_or(After::Nothing), after));
            return;
        }
        s.saving = true;
        let (p, token, epoch) = (self.ctx.proxy.clone(), s.token, s.cookie_epoch);
        wk::export_cookies(&s.store, move |c| {
            let _ = p.send_event(UserEvent::Cookies(c, after, token, epoch));
        });
    }

    fn saved(&mut self, cookies: Vec<CookieRec>, after: After, token: u64, epoch: u64) {
        let Some(s) = self.s.as_mut() else { return };
        if token != s.token {
            // Late export of a session that was locked meanwhile: never written into this one.
            return;
        }
        s.saving = false;
        let queued = s.queued.take();
        if epoch != s.cookie_epoch {
            // Cookies were deleted while this export ran: it may still contain them. Export again.
            return self.persist(stronger(after, queued.unwrap_or(After::Nothing)));
        }
        if !s.cookies_ok {
            // Import was never confirmed: keep vault cookies WebKit may not have received yet.
            let mut cookies = cookies;
            let missing: Vec<CookieRec> = s
                .data
                .cookies
                .drain(..)
                .filter(|o| {
                    !cookies
                        .iter()
                        .any(|c| c.name == o.name && c.domain == o.domain && c.path == o.path)
                })
                .collect();
            cookies.extend(missing);
            s.data.cookies = cookies;
        } else {
            s.data.cookies = cookies;
        }
        s.save_now(&self.cfg);
        match queued {
            // Requested during this export: one more export so nothing set meanwhile is lost.
            Some(q) => self.persist(stronger(after, q)),
            None => self.after_save(after),
        }
    }

    fn after_save(&mut self, after: After) {
        match after {
            After::Nothing => {}
            After::Quit => self.exit = true,
            After::Lock => {
                self.restore_by = None;
                if let Some(s) = self.s.take() {
                    for a in &s.asks {
                        wk::decide_download(a.key, false, &a.url, None);
                    }
                    wk::clear_store(&s.store);
                }
                // Don't reveal the chosen theme (or anything else) while locked.
                self.cfg = Settings::default();
                wk::set_theme(&self.ctx.window, None);
                self.show_lock();
            }
        }
    }

    fn set_find(&mut self, open: bool) {
        let Some(s) = self.s.as_mut() else { return };
        let changed = s.find_open != open;
        s.find_open = open;
        if changed {
            self.resize();
        }
        let Some(s) = self.s.as_ref() else { return };
        if open {
            let _ = s.toolbar.focus();
            s.eval("openFind()");
        } else {
            s.eval("closeFind()");
            if let Some(v) = s.view() {
                let _ = v.evaluate_script("getSelection().removeAllRanges()");
                let _ = v.focus();
            }
        }
    }

    fn session_cmd(&mut self, name: &str, arg: &str) {
        let Some(s) = self.s.as_mut() else { return };
        match name {
            "new-tab" => self.open_tab(self.cfg.homepage.clone()),
            "close-tab" => {
                let idx = arg.parse().ok().map_or(Some(s.active), |id| s.tab_idx(id));
                if let Some(i) = idx {
                    self.close_tab(i);
                }
            }
            "reopen-tab" => {
                if let Some(url) = s.closed.pop() {
                    self.open_tab(url);
                }
            }
            "move-tab" => {
                let (id, to) = arg.split_once(':').unwrap_or((arg, ""));
                let (Some(from), Ok(to)) = (
                    id.parse().ok().and_then(|id| s.tab_idx(id)),
                    to.parse::<usize>(),
                ) else {
                    return;
                };
                let to = to.min(s.tabs.len() - 1);
                if from != to {
                    let active = s.tabs[s.active].id;
                    let tab = s.tabs.remove(from);
                    s.tabs.insert(to, tab);
                    s.active = s.tab_idx(active).unwrap_or(0);
                    s.dirty = true;
                }
                s.sync(&self.cfg);
            }
            "update-check" => update::check(self.ctx.proxy.clone(), true),
            "update-show" => self.session_cmd("panel", "update"),
            "update-dismiss" => {
                s.update_bar = false;
                self.resize();
                if let Some(s) = &self.s {
                    s.sync(&self.cfg);
                }
            }
            // The click is the explicit confirmation; the file is verified and only revealed.
            "update-get" => {
                if s.update_busy {
                    return;
                }
                let Some(asset) = s.update.as_ref().and_then(|u| u.asset.clone()) else {
                    return;
                };
                if asset.sha256.is_none() {
                    return s.note("installer_no_checksum", &[]);
                }
                s.update_busy = true;
                update::download(self.ctx.proxy.clone(), asset);
                s.sync(&self.cfg);
                s.refresh_panel(&self.cfg);
            }
            "select" => {
                if let Some(i) = arg.parse().ok().and_then(|id| s.tab_idx(id)) {
                    self.activate(i);
                }
            }
            "tab" => {
                let n: usize = arg.parse().unwrap_or(1);
                let i = if n >= 9 {
                    s.tabs.len().saturating_sub(1)
                } else {
                    n.saturating_sub(1)
                };
                self.activate(i);
            }
            "next-tab" | "prev-tab" => {
                let len = s.tabs.len().max(1);
                let i = if name == "next-tab" {
                    (s.active + 1) % len
                } else {
                    (s.active + len - 1) % len
                };
                self.activate(i);
            }
            // Reload on the certificate interstitial retries the refused URL itself; a built-in
            // page is rendered again.
            "reload" if s.cur().is_some_and(|t| t.tls_error || t.internal) => {
                if let (Some(t), Some(v)) = (s.cur(), s.view()) {
                    let _ = match internal_page(&t.url, &self.cfg.language) {
                        Some(html) => v.load_html(&html),
                        None => v.load_url(&t.url),
                    };
                }
            }
            "back" | "forward" | "reload" | "stop" => {
                if let Some(v) = s.view() {
                    let _ = match name {
                        "back" => v.evaluate_script("history.back()"),
                        "forward" => v.evaluate_script("history.forward()"),
                        "reload" => v.reload(),
                        _ => v.evaluate_script("window.stop()"),
                    };
                }
            }
            "home" => self.navigate(self.cfg.homepage.clone()),
            "go" if !arg.trim().is_empty() => {
                let input = arg.trim();
                let url = if is_internal(input) { input.to_string() } else { to_url(input, self.cfg.search_prefix()) };
                self.navigate(url);
            }
            "open" => self.navigate(arg.to_string()),
            "open-new" => self.open_tab(arg.to_string()),
            "location" => {
                let _ = s.toolbar.focus();
                s.eval("focusUrl()");
            }
            "zoom-in" | "zoom-out" | "zoom-reset" => {
                let z = s.zoom;
                s.zoom = match name {
                    "zoom-in" => ZOOM_STEPS
                        .iter()
                        .copied()
                        .find(|x| *x > z + 0.001)
                        .unwrap_or(z),
                    "zoom-out" => ZOOM_STEPS
                        .iter()
                        .rev()
                        .copied()
                        .find(|x| *x < z - 0.001)
                        .unwrap_or(z),
                    _ => self.cfg.zoom,
                };
                for v in s.loaded() {
                    let _ = v.zoom(s.zoom);
                }
                s.note("zoom_level", &[&(s.zoom * 100.0).round().to_string()]);
            }
            "find" if arg.is_empty() => self.set_find(true),
            "find" => {
                #[derive(Deserialize)]
                struct Req {
                    q: String,
                    dir: String,
                }
                let (Ok(r), Some(v)) = (serde_json::from_str::<Req>(arg), s.view()) else {
                    return;
                };
                let p = self.ctx.proxy.clone();
                let _ = v.evaluate_script_with_callback(
                    &format!("{FIND_JS}({},{})", js(&r.q), js(&r.dir)),
                    move |res| {
                        let _ = p.send_event(UserEvent::FindResult(res));
                    },
                );
            }
            "find-next" | "find-prev" => {
                if s.find_open {
                    s.eval(&format!("findStep({})", name == "find-prev"));
                } else {
                    self.set_find(true);
                }
            }
            "find-close" => self.set_find(false),
            "bookmark" => {
                let Some(t) = s.cur().filter(|t| t.url.starts_with("http")) else {
                    return;
                };
                let (url, title) = (t.url.clone(), t.label());
                let bms = &mut s.data.bookmarks;
                if let Some(i) = bms.iter().position(|b| b.url == url) {
                    bms.remove(i);
                    s.note("bookmark_removed", &[]);
                } else {
                    bms.push(Bookmark { url, title });
                    s.note("bookmark_added", &[]);
                }
                s.dirty = true;
                s.sync(&self.cfg);
                s.refresh_panel(&self.cfg);
            }
            "bm-del" => {
                s.data.bookmarks.retain(|b| b.url != arg);
                s.dirty = true;
                s.sync(&self.cfg);
                s.refresh_panel(&self.cfg);
            }
            "hist-del" => {
                if let Ok(time) = arg.parse::<u64>() {
                    s.data.history.retain(|e| e.time != time);
                    s.dirty = true;
                    s.refresh_panel(&self.cfg);
                }
            }
            "panel" => {
                let arg = panel_section(arg);
                s.site = None;
                if s.panel.is_some() && s.panel_section == arg {
                    s.close_panel();
                } else if let Some(p) = &s.panel {
                    let _ = p.evaluate_script(&format!("show({})", js(arg)));
                    s.panel_section = arg.to_string();
                } else {
                    let data = s.panel_data(&self.cfg, arg).replace("</", "<\\/");
                    let html = include_str!("panel.html").replace("/*DATA*/", &data);
                    s.panel = self
                        .ctx
                        .ui_view(html, content_rect(&self.ctx.window, s.top()));
                    s.panel_section = arg.to_string();
                }
                s.sync(&self.cfg);
            }
            "panel-section" => {
                s.panel_section = panel_section(arg).to_string();
                s.sync(&self.cfg);
            }
            "panel-close" => {
                s.close_panel();
                s.sync(&self.cfg);
            }
            "set" => {
                let (k, v) = arg.split_once('=').unwrap_or((arg, ""));
                if !self.cfg.set(k, v) {
                    return;
                }
                s.dirty = true;
                let w = &self.ctx.window;
                match k {
                    "theme" => {
                        wk::set_theme(w, theme_of(&self.cfg.theme));
                        let bg = bg_of(w);
                        let _ = s.toolbar.set_background_color(bg);
                        for v in s.loaded().chain(s.panel.as_ref()) {
                            let _ = v.set_background_color(bg);
                        }
                    }
                    "zoom" => {
                        s.zoom = self.cfg.zoom;
                        for v in s.loaded() {
                            let _ = v.zoom(s.zoom);
                        }
                    }
                    "adblock" => {
                        self.ctx.adblock.set(self.cfg.adblock);
                        s.apply_rules_all(&self.cfg);
                    }
                    "adblock_easylist" | "adblock_hosts" => {
                        let (on, hosts) = match k {
                            "adblock_hosts" => (self.cfg.adblock_hosts, true),
                            _ => (self.cfg.adblock_easylist, false),
                        };
                        if !on {
                            adblock::remove(hosts);
                        }
                        let fetch = if on {
                            adblock::stale(!hosts, hosts)
                        } else {
                            Vec::new()
                        };
                        refresh_rules(self.ctx.proxy.clone(), fetch, true);
                    }
                    _ => {}
                }
                s.sync(&self.cfg);
                self.persist(After::Nothing);
            }
            "adblock-update" => {
                let (easylist, hosts) = (self.cfg.adblock_easylist, self.cfg.adblock_hosts);
                let all: Vec<_> = adblock::SOURCES
                    .iter()
                    .filter(|s| s.enabled(easylist, hosts))
                    .collect();
                s.note("adblock_lists_downloading", &[]);
                refresh_rules(self.ctx.proxy.clone(), all, true);
            }
            // Global switch; takes effect immediately on all loaded tabs.
            "toggle-adblock" => {
                self.cfg.adblock = !self.cfg.adblock;
                self.ctx.adblock.set(self.cfg.adblock);
                s.apply_rules_all(&self.cfg);
                s.note(if self.cfg.adblock { "adblock_enabled" } else { "adblock_disabled" }, &[]);
                s.dirty = true;
                self.adblock_changed();
            }
            // Toggles a site on the allowlist (empty arg = current tab's site), then reloads its tabs.
            "adblock-allow" => {
                let host = if arg.is_empty() {
                    s.cur().map(|t| host_of(&t.url)).unwrap_or_default()
                } else {
                    arg.to_ascii_lowercase()
                };
                let host = host.trim_start_matches("www.").to_string();
                let valid = host.contains('.') || host == "localhost";
                if !valid
                    || !host
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-'))
                {
                    return;
                }
                let list = &mut self.cfg.adblock_allow;
                let allowed = match list.iter().position(|d| *d == host) {
                    Some(i) => {
                        list.remove(i);
                        false
                    }
                    None if list.len() < MAX_ALLOW => {
                        list.push(host.clone());
                        true
                    }
                    None => return s.note("allowlist_full", &[]),
                };
                s.apply_rules_all(&self.cfg);
                for t in s.tabs.iter().filter(|t| {
                    t.view.is_some()
                        && (host_of(&t.url) == host
                            || host_of(&t.url).ends_with(&format!(".{host}")))
                }) {
                    if let Some(v) = &t.view {
                        let _ = v.reload();
                    }
                }
                s.note(if allowed { "adblock_off_for_host" } else { "adblock_on_for_host" }, &[&host]);
                s.dirty = true;
                self.adblock_changed();
            }
            "clear" => {
                match arg {
                    "history" => s.data.history.clear(),
                    "downloads" => s.data.downloads.retain(|d| d.state == "active"),
                    "cookies" => {
                        s.cookie_epoch += 1;
                        wk::clear_store(&s.store);
                        s.data.cookies.clear();
                    }
                    _ => return,
                }
                s.dirty = true;
                s.refresh_panel(&self.cfg);
            }
            // Downloads are never opened/executed, only shown in the file manager.
            "reveal-file" => {
                if s.data
                    .downloads
                    .iter()
                    .any(|d| d.path == arg && d.state == "done")
                {
                    wk::reveal(arg);
                }
            }
            "dl-allow" | "dl-deny" => {
                let Some(i) = arg
                    .parse()
                    .ok()
                    .and_then(|id: u32| s.asks.iter().position(|a| a.id == id))
                else {
                    return;
                };
                let a = s.asks.remove(i);
                let view = a
                    .tab
                    .and_then(|t| s.tab_idx(t))
                    .and_then(|i| s.tabs[i].view.as_ref());
                wk::decide_download(a.key, name == "dl-allow", &a.url, view);
                if s.asks.is_empty() {
                    self.resize();
                }
                if let Some(s) = &self.s {
                    s.sync(&self.cfg);
                }
            }
            "change-pw" => {
                #[derive(Deserialize)]
                struct Req {
                    old: String,
                    new: String,
                }
                let Ok(mut r) = serde_json::from_str::<Req>(arg) else {
                    return;
                };
                let (ok, msg) = if !s.vault.verify(&r.old) {
                    (false, "pw_wrong_current")
                } else if r.new.chars().count() < 8 {
                    (false, "new_pw_too_short")
                } else {
                    // Transactional (`rotate_key`): the new vault must be on disk (tmp + fsync +
                    // rename) before it replaces the in-memory key or the UI reports success.
                    s.snapshot();
                    s.data.settings = Some(self.cfg.clone());
                    match s.vault.rotate_key(&r.new, &s.data) {
                        Ok(()) => {
                            s.dirty = false;
                            (true, "pw_changed")
                        }
                        Err(e) => {
                            eprintln!("password change: vault save failed: {e}");
                            (false, "pw_save_failed")
                        }
                    }
                };
                r.old.zeroize();
                r.new.zeroize();
                if let Some(p) = &s.panel {
                    let _ = p.evaluate_script(&format!("pwResult({ok},{})", js(&s.tr(msg, &[]))));
                }
            }
            "site-info" => {
                // A click on the lock first blurs (closes) an open popover: treat as toggle.
                if s.site.take().is_some() || s.site_closed.elapsed() < Duration::from_millis(300) {
                    return;
                }
                s.site_x = arg.parse().unwrap_or(150.0);
                s.shield_open = false;
                self.request_site_cookies(false);
            }
            // Shield popover: blocked counts and the per-site switch (same toggle rule as site-info).
            "shield-open" => {
                if s.site.take().is_some() || s.site_closed.elapsed() < Duration::from_millis(300) {
                    return;
                }
                s.site_x = arg.parse().unwrap_or(0.0);
                let data = s.shield_data(&self.cfg).to_string().replace("</", "<\\/");
                let html = include_str!("shield.html").replace("/*DATA*/", &data);
                let h = if s.site_h > 0.0 { s.site_h } else { 300.0 };
                s.site = self
                    .ctx
                    .ui_view(html, site_rect(&self.ctx.window, s.site_x, h));
                s.shield_open = true;
            }
            "get-adblock-stats" => s.push_shield(&self.cfg),
            "site-clear" => self.request_site_cookies(true),
            "get-vault-sessions" => self.request_sessions(None),
            // UI language: vault setting + plain copy for the lock screen, then every open UI
            // view re-renders its text without reloading.
            "set-language" => {
                if !self.cfg.set("language", arg) {
                    return;
                }
                settings::store_language(&self.cfg.language);
                *self.ctx.lang.borrow_mut() = self.cfg.language.clone();
                let script = format!("setLanguage({})", js(&self.cfg.language));
                for v in std::iter::once(&s.toolbar).chain(s.panel.as_ref()).chain(s.site.as_ref()) {
                    let _ = v.evaluate_script(&script);
                }
                // Built-in pages have no script: render them again in the new language.
                for t in s.tabs.iter().filter(|t| t.internal) {
                    if let (Some(v), Some(html)) = (&t.view, internal_page(&t.url, &self.cfg.language)) {
                        let _ = v.load_html(&html);
                    }
                }
                s.dirty = true;
                self.persist(After::Nothing);
            }
            "delete-vault-session" => {
                let site = arg.trim().to_ascii_lowercase();
                if !site.is_empty()
                    && site
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-'))
                {
                    self.request_sessions(Some(site));
                }
            }
            "site-close" => {
                if s.site.take().is_some() {
                    s.site_closed = Instant::now();
                }
            }
            "site-size" => {
                s.site_h = arg.parse::<f64>().unwrap_or(0.0).clamp(80.0, 640.0);
                if let Some(v) = &s.site {
                    wk::set_bounds(v, site_rect(&self.ctx.window, s.site_x, s.site_h));
                }
            }
            "site-settings" => {
                s.site = None;
                self.session_cmd("panel", "settings");
            }
            "lock" => self.persist(After::Lock),
            _ => {}
        }
    }
}

/// Off the main thread: downloads `fetch`, then for each content-blocker list either reuses
/// WebKit's compiled copy (inputs unchanged) or converts the filter lists again.
fn refresh_rules(
    proxy: EventLoopProxy<UserEvent>,
    fetch: Vec<&'static adblock::Source>,
    manual: bool,
) {
    std::thread::spawn(move || {
        let send = |e| {
            let _ = proxy.send_event(e);
        };
        let errors: Vec<_> = fetch
            .iter()
            .filter_map(|s| adblock::fetch(s).err().map(|k| (k, s.name)))
            .collect();
        if manual && !fetch.is_empty() {
            send(UserEvent::RulesNote(errors));
        }
        for id in adblock::LEGACY {
            send(UserEvent::RulesOff(id));
        }
        for g in adblock::GROUPS {
            let Some(hash) = adblock::input_hash(g) else {
                g.parts.iter().for_each(|p| send(UserEvent::RulesOff(p)));
                continue;
            };
            if let Some(n) = adblock::compiled_parts(g, &hash) {
                for (i, id) in g.parts.iter().enumerate() {
                    send(if i < n {
                        UserEvent::RulesCached {
                            id,
                            hash: hash.clone(),
                        }
                    } else {
                        UserEvent::RulesOff(id)
                    });
                }
            } else if let Some(jsons) = adblock::build(g) {
                let parts = jsons.len();
                let mut jsons = jsons.into_iter();
                for id in g.parts {
                    send(match jsons.next() {
                        Some(json) => UserEvent::RulesBuilt {
                            id,
                            hash: hash.clone(),
                            json,
                            parts,
                        },
                        None => UserEvent::RulesOff(id),
                    });
                }
            }
        }
    });
}

/// Ad blocker wanted for `url`: globally on and the site not allowlisted.
fn blocks(cfg: &Settings, url: &str) -> bool {
    cfg.adblock && !cfg.allowed(&host_of(url))
}

/// Applies the content blocker for `url` to the tab's view; skips no-op updates.
fn apply_rules(cfg: &Settings, t: &mut Tab, url: &str) {
    let on = blocks(cfg, url);
    if let Some(v) = &t.view
        && t.blocking != Some(on)
    {
        wk::set_rules(v, on);
        t.blocking = Some(on);
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Static certificate-error page (rendered in the tab, which has no IPC), in the UI language.
fn interstitial(lang: &str, host: &str, reasons: &[&str]) -> String {
    let tr = |key: &str| html_escape(&settings::tr(lang, key, &[]));
    let items: String = reasons.iter().map(|r| format!("<li>{}</li>", tr(r))).collect();
    let host = if host.is_empty() { tr("this_site") } else { html_escape(host) };
    // Template escaped first, then the escaped host inserted in bold.
    let desc = tr("interstitial_desc").replace("{0}", &format!("<b>{host}</b>"));
    let (title, hint, back) = (tr("not_private_title"), tr("interstitial_hint"), tr("back_to_safety"));
    let lang = html_escape(lang);
    format!(
        r#"<!doctype html><html lang="{lang}"><head><meta charset="utf-8"><title>{title}</title>
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'">
<style>
:root {{ color-scheme: dark; }}
body {{ margin: 0; min-height: 100vh; display: grid; place-items: center; background: #1b1b1f; color: #e6e6e9;
  font: 14px -apple-system, BlinkMacSystemFont, system-ui, "Inter", "Cantarell", "Noto Sans", sans-serif; }}
main {{ max-width: 560px; padding: 32px; }}
.icon {{ width: 48px; height: 48px; color: #e5534b; }}
h1 {{ font-size: 22px; margin: 16px 0 8px; }}
p {{ color: #9a9aa3; line-height: 1.5; }}
b {{ color: #e6e6e9; }}
ul {{ color: #e5534b; padding-left: 20px; line-height: 1.6; }}
button {{ margin-top: 16px; font: inherit; color: #fff; background: #5b8def; border: 0; border-radius: 8px; padding: 8px 16px; }}
</style></head><body><main>
<svg class="icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3l10 18H2z"/><path d="M12 10v4M12 17.5v.01"/></svg>
<h1>{title}</h1>
<p>{desc}</p>
<ul>{items}</ul>
<p>{hint}</p>
<button onclick="history.back()">{back}</button>
</main></body></html>"#
    )
}

/// Combined follow-up of two save requests: Quit > Lock > Nothing.
fn stronger(a: After, b: After) -> After {
    let rank = |x: After| match x {
        After::Nothing => 0,
        After::Lock => 1,
        After::Quit => 2,
    };
    if rank(b) > rank(a) { b } else { a }
}

fn js(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_default()
}

pub fn theme_of(name: &str) -> Option<Theme> {
    match name {
        "dark" => Some(Theme::Dark),
        "light" => Some(Theme::Light),
        _ => None,
    }
}

fn bg_of(window: &Window) -> RGBA {
    if wk::is_light(window) {
        BG_LIGHT
    } else {
        BG_DARK
    }
}

fn rect(window: &Window, y: f64, h: Option<f64>) -> Rect {
    let size = window.inner_size().to_logical::<f64>(window.scale_factor());
    let h = h.unwrap_or(size.height - y).max(0.0);
    Rect {
        position: dpi::LogicalPosition::new(0.0, y).into(),
        size: dpi::LogicalSize::new(size.width, h).into(),
    }
}

/// Popover under the address bar's lock icon, kept inside the window.
fn site_rect(window: &Window, x: f64, h: f64) -> Rect {
    let size = window.inner_size().to_logical::<f64>(window.scale_factor());
    let w = SITE_W.min(size.width - 16.0).max(0.0);
    let x = (x - 6.0).clamp(8.0, (size.width - w - 8.0).max(8.0));
    let y = TABS_H + NAV_H - 4.0;
    Rect {
        position: dpi::LogicalPosition::new(x, y).into(),
        size: dpi::LogicalSize::new(w, h.min(size.height - y - 8.0).max(0.0)).into(),
    }
}

/// Host part of a URL (no userinfo/port), lowercase.
fn host_of(url: &str) -> String {
    let rest = url.split_once("://").map_or("", |(_, r)| r);
    let auth = rest.split(['/', '?', '#']).next().unwrap_or("");
    let auth = auth.rsplit_once('@').map_or(auth, |(_, h)| h);
    let host = if auth.starts_with('[') {
        auth.split_inclusive(']').next().unwrap_or(auth)
    } else {
        auth.split(':').next().unwrap_or(auth)
    };
    host.to_ascii_lowercase()
}

/// Registrable site of a host or cookie domain ("mail.google.com" -> "google.com",
/// "www.bbc.co.uk" -> "bbc.co.uk"); heuristic, no public-suffix list.
fn site_of(domain: &str) -> String {
    let d = domain.trim_start_matches('.').to_ascii_lowercase();
    let parts: Vec<&str> = d.split('.').collect();
    if parts.len() <= 2 || d.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
        return d;
    }
    let n = parts.len();
    let second_level = [
        "co", "com", "org", "net", "ac", "gov", "edu", "or", "ne", "go",
    ]
    .contains(&parts[n - 2]);
    let take = if second_level && parts[n - 1].len() == 2 {
        3
    } else {
        2
    };
    parts[n - take..].join(".")
}

/// Cookie name typical for a login session (heuristic; analytics/consent cookies don't match).
fn is_login_cookie(c: &CookieRec) -> bool {
    const KEYS: &[&str] = &[
        "sess", "sid", "auth", "token", "login", "logged", "user", "account", "jwt", "remember",
        "li_at",
    ];
    let n = c.name.to_ascii_lowercase();
    KEYS.iter().any(|k| n.contains(k)) || n.starts_with("__host-") || n.starts_with("__secure-")
}

/// The site of `host` holds a login-like cookie (vault snapshot; refreshed on every save).
fn logged_in(cookies: &[CookieRec], host: &str) -> bool {
    if host.is_empty() {
        return false;
    }
    let site = site_of(host);
    cookies
        .iter()
        .any(|c| is_login_cookie(c) && site_of(&c.domain) == site)
}

/// Cookie `domain` applies to `host` (exact or parent domain).
fn cookie_for(host: &str, domain: &str) -> bool {
    let d = domain.trim_start_matches('.').to_ascii_lowercase();
    !d.is_empty() && (host == d || host.ends_with(&format!(".{d}")))
}

fn full_rect(window: &Window) -> Rect {
    rect(window, 0.0, None)
}

fn toolbar_rect(window: &Window, h: f64) -> Rect {
    rect(window, 0.0, Some(h))
}

fn content_rect(window: &Window, top: f64) -> Rect {
    rect(window, top, None)
}

/// Panel sections the UI may request over IPC; anything else opens the settings.
const VALID_SECTIONS: &[&str] = &["settings", "history", "bookmarks", "downloads", "logins", "about", "update"];

fn panel_section(arg: &str) -> &'static str {
    VALID_SECTIONS.iter().find(|s| **s == arg).copied().unwrap_or("settings")
}

/// Address of the built-in changelog (also reachable as `about:changelog`).
const CHANGELOG_URL: &str = "crow:changelog";

/// Built-in offline pages: static HTML with its own strict CSP and no script, loaded into the
/// tab view (which has no IPC) instead of being fetched. Text is filled in `lang` here, so the
/// tab never gets the dictionaries or `t()`.
fn internal_page(url: &str, lang: &str) -> Option<String> {
    is_internal(url).then(|| localize_page(include_str!("changelog.html"), lang))
}

fn is_internal(url: &str) -> bool {
    matches!(url.trim().to_ascii_lowercase().as_str(), "crow:changelog" | "about:changelog")
}

/// Replaces `{{key}}` with the escaped translation (`**bold**` → `<b>`, `` `code` `` →
/// `<code>`) and `{{@lang}}` with the language code.
fn localize_page(src: &str, lang: &str) -> String {
    let mut out = String::with_capacity(src.len() * 2);
    let mut rest = src;
    while let Some(i) = rest.find("{{") {
        let Some(j) = rest[i..].find("}}") else { break };
        out.push_str(&rest[..i]);
        match &rest[i + 2..i + j] {
            "@lang" => out.push_str(&html_escape(lang)),
            key => out.push_str(&markup(&html_escape(&settings::tr(lang, key, &[])))),
        }
        rest = &rest[i + j + 2..];
    }
    out.push_str(rest);
    out
}

/// Inline markup of already escaped text: `**bold**` and `` `code` ``.
fn markup(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 32);
    for (i, part) in s.split("**").enumerate() {
        let bold = i % 2 == 1;
        if bold {
            out.push_str("<b>");
        }
        for (j, p) in part.split('`').enumerate() {
            if j % 2 == 1 {
                out.push_str("<code>");
                out.push_str(p);
                out.push_str("</code>");
            } else {
                out.push_str(p);
            }
        }
        if bold {
            out.push_str("</b>");
        }
    }
    out
}

fn to_url(input: &str, search_prefix: &str) -> String {
    // Explicit schemes are navigated as typed, never sent to the search engine.
    const SCHEMES: &[&str] = &[
        "http://", "https://", "file:", "about:", "data:", "blob:", "mailto:",
    ];
    let lower = input.to_ascii_lowercase();
    if SCHEMES.iter().any(|s| lower.starts_with(s)) {
        return input.to_string();
    }
    let host = input.split(['/', '?', '#']).next().unwrap_or("");
    let host = match host.rsplit_once(':') {
        Some((h, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => h,
        _ => host,
    }
    .to_ascii_lowercase();
    // Local dev servers and mDNS hosts usually have no TLS.
    let local =
        matches!(host.as_str(), "localhost" | "127.0.0.1" | "[::1]") || host.ends_with(".local");
    if !input.contains(' ') && local {
        format!("http://{input}")
    } else if !input.contains(' ') && input.contains('.') {
        format!("https://{input}")
    } else {
        let mut q = String::from(search_prefix);
        for b in input.bytes() {
            match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    q.push(b as char)
                }
                b' ' => q.push('+'),
                _ => q.push_str(&format!("%{b:02X}")),
            }
        }
        q
    }
}
