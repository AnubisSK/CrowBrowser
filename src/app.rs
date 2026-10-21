use crate::{
    adblock, cert,
    settings::{ENGINES, Settings},
    update,
    vault::{self, Bookmark, CookieRec, Download, HistoryEntry, MAX_HISTORY, Vault, VaultData},
    wk,
};
use serde::Deserialize;
use serde_json::json;
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
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
const ZOOM_STEPS: &[f64] = &[0.5, 0.67, 0.75, 0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0];
const TICK: Duration = Duration::from_secs(30);
const QUIT_TIMEOUT: Duration = Duration::from_secs(3);
/// Max wait for WebKit to confirm the cookie import before the session starts anyway.
const RESTORE_TIMEOUT: Duration = Duration::from_secs(3);
/// Favicon: polled after load (tab views have no IPC), a 32x32 PNG data URL of bounded size.
const FAV_POLL: Duration = Duration::from_millis(500);
const FAV_TRIES: u8 = 8;
const FAV_MAX: usize = 48 * 1024;
const FAV_READ: &str = "window.__crowIcon || ''";

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
    PageLoad { tab: u32, started: bool, url: String },
    Title { tab: u32, title: String },
    NewTab(String),
    AdoptTab(u32),
    /// A download wants to write to disk; held until the user answers the toolbar prompt.
    /// `key` identifies the pending download inside `wk`, `tab` the requesting tab (if known).
    DownloadAsk { key: u32, tab: Option<u32>, url: String, name: String },
    DownloadStarted { url: String, path: PathBuf },
    DownloadDone { url: String, ok: bool },
    FindResult(String),
    Cookies(Vec<CookieRec>, After),
    /// Fresh cookie list for the site-info popover; `clear` = delete the site's cookies first.
    SiteCookies { cookies: Vec<CookieRec>, clear: bool },
    CookiesRestored,
    RulesReady,
    /// Result of `FAV_READ` in a tab (JSON string).
    Icon { tab: u32, data: String },
    Update { result: Result<Option<update::Info>, String>, manual: bool },
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

    fn build(
        &self,
        id: u32,
        url: Option<&str>,
        config: wk::Config,
        rect: Rect,
    ) -> wry::Result<WebView> {
        let (p1, p2) = (self.proxy.clone(), self.proxy.clone());
        let ctx = self.clone();
        let mut b = wk::builder(config)
            .with_background_color(bg_of(&self.window))
            .with_on_page_load_handler(move |ev, url| {
                let started = matches!(ev, PageLoadEvent::Started);
                let _ = p1.send_event(UserEvent::PageLoad { tab: id, started, url });
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
                    if wk::take_approved(&url) {
                        let _ = p3.send_event(UserEvent::DownloadStarted { url, path: path.clone() });
                        return true;
                    }
                    // No drive-by downloads: cancel, ask the user, retry on approval.
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let _ = p3.send_event(UserEvent::DownloadAsk { key: 0, tab: Some(id), url, name });
                    false
                })
                .with_download_completed_handler(move |url, _, ok| {
                    let _ = p4.send_event(UserEvent::DownloadDone { url, ok });
                });
        }
        if let Some(url) = url {
            b = b.with_url(url);
        }
        wk::attach(b, rect, &self.window)
    }

    /// window.open / target=_blank: a real WKWebView keeping `window.opener`
    /// (required by OAuth login popups), adopted as a new tab.
    fn popup(&self, url: String, features: NewWindowFeatures) -> NewWindowResponse {
        let id = self.next();
        let rect = content_rect(&self.window, TABS_H + NAV_H);
        match self.build(id, None, wk::popup_config(features), rect) {
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

    fn ui_view(&self, html: String, rect: Rect) -> Option<WebView> {
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
}

impl Tab {
    fn new(id: u32, url: String, view: Option<WebView>) -> Self {
        Self { id, url, title: String::new(), view, hidden_since: Instant::now(), loading: false, icon: String::new(), fav_tries: 0 }
    }

    fn label(&self) -> String {
        if !self.title.trim().is_empty() {
            return self.title.clone();
        }
        let host = self.url.split("://").nth(1).and_then(|r| r.split('/').next());
        host.filter(|h| !h.is_empty()).unwrap_or("Nová karta").to_string()
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
    ready: bool,
    /// WebKit confirmed the cookie import (false if the session started on timeout).
    cookies_ok: bool,
    closed: Vec<String>,
    asks: Vec<Ask>,
    /// Site-info popover (lock icon), its left edge and content height.
    site: Option<WebView>,
    site_x: f64,
    site_h: f64,
    site_closed: Instant,
    fav_due: Option<Instant>,
    update: Option<update::Info>,
    update_bar: bool,
    update_busy: bool,
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

    fn tab_idx(&self, id: u32) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }

    fn eval(&self, js: &str) {
        let _ = self.toolbar.evaluate_script(js);
    }

    fn toast(&self, msg: &str) {
        self.eval(&format!("toast({})", js(msg)));
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
            "bookmarked": self.data.bookmarks.iter().any(|b| b.url == url),
            "downloads": self.data.downloads.iter().filter(|d| d.state == "active").count(),
            "panel": if self.panel.is_some() { self.panel_section.as_str() } else { "" },
            "adblock": cfg.adblock,
            "ask": self.asks.first().map(|a| json!({
                "id": a.id, "name": a.name, "url": a.url, "more": self.asks.len() - 1,
            })),
            "update": self.update.as_ref().filter(|_| self.update_bar).map(|u| json!({
                "version": u.version, "busy": self.update_busy, "asset": u.asset.is_some(),
            })),
        });
        self.eval(&format!("render({state})"));
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
}

impl App {
    pub fn new(window: Window, proxy: EventLoopProxy<UserEvent>) -> Self {
        let p = proxy.clone();
        wk::compile_rules(adblock::rules_json(), move || {
            let _ = p.send_event(UserEvent::RulesReady);
        });
        #[cfg(target_os = "linux")]
        {
            let (p0, p1, p2) = (proxy.clone(), proxy.clone(), proxy.clone());
            wk::watch_downloads(
                &wk::new_store(),
                move |key, url, name| {
                    let _ = p0.send_event(UserEvent::DownloadAsk { key, tab: None, url, name });
                },
                move |url, path| {
                    let _ = p1.send_event(UserEvent::DownloadStarted { url, path });
                },
                move |url, ok| {
                    let _ = p2.send_event(UserEvent::DownloadDone { url, ok });
                },
            );
        }
        let ctx = TabCtx { window: Rc::new(window), proxy, next_id: Rc::new(Cell::new(1)) };
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

    fn lock_fail(&self, msg: &str) {
        if let Some(l) = &self.lock {
            let _ = l.evaluate_script(&format!("fail({})", js(msg)));
        }
    }

    pub fn wake(&self) -> Instant {
        let fav = self.s.as_ref().and_then(|s| s.fav_due);
        [self.quit_at, self.restore_by, fav].into_iter().flatten().fold(self.next_tick, Instant::min)
    }

    pub fn resize(&self) {
        let w = &self.ctx.window;
        if let Some(l) = &self.lock {
            wk::set_bounds(l, full_rect(w));
        }
        if let Some(s) = &self.s {
            wk::set_bounds(&s.toolbar, toolbar_rect(w, s.top()));
            let c = content_rect(w, s.top());
            for v in s.loaded().chain(s.panel.as_ref()) {
                wk::set_bounds(v, c);
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
            UserEvent::DownloadAsk { key, tab, url, name } => self.download_ask(key, tab, url, name),
            UserEvent::DownloadStarted { url, path } => self.download_started(url, path),
            UserEvent::DownloadDone { url, ok } => self.download_done(url, ok),
            UserEvent::FindResult(r) => {
                if let Some(s) = &self.s {
                    s.eval(&format!("findCount({})", r.trim().parse::<u32>().unwrap_or(0)));
                }
            }
            UserEvent::Cookies(c, after) => self.saved(c, after),
            UserEvent::CookiesRestored => self.restored(true),
            UserEvent::SiteCookies { cookies, clear } => self.site_cookies(cookies, clear),
            UserEvent::RulesReady => {
                if let (Some(s), true) = (&self.s, self.cfg.adblock) {
                    for v in s.loaded() {
                        wk::set_rules(v, true);
                    }
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
                    None => s.toast("Aktualizáciu sa nepodarilo stiahnuť alebo overiť"),
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
                    t.view = None;
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
                w.set_fullscreen(if w.fullscreen().is_some() { None } else { Some(Fullscreen::Borderless(None)) });
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
                    return self.lock_fail("Heslo musí mať aspoň 8 znakov");
                }
                let vault = Vault::create(arg);
                let data = VaultData::default();
                match vault.save(&data) {
                    Ok(()) => self.start(vault, data),
                    Err(_) => self.lock_fail("Trezor sa nepodarilo uložiť"),
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
        self.cfg = data.settings.take().or_else(Settings::take_legacy).unwrap_or_default();
        let w = &self.ctx.window;
        wk::set_theme(w, theme_of(&self.cfg.theme));
        let html = include_str!("toolbar.html").to_string();
        let Some(toolbar) = self.ctx.ui_view(html, toolbar_rect(w, TABS_H + NAV_H)) else {
            return self.lock_fail("Nepodarilo sa vytvoriť okno");
        };
        self.lock = None;
        let store = wk::new_store();
        let p = self.ctx.proxy.clone();
        wk::import_cookies(&store, &data.cookies, move || {
            let _ = p.send_event(UserEvent::CookiesRestored);
        });
        // If WebKit never answers, the session still opens (see `timer`).
        self.restore_by = Some(Instant::now() + RESTORE_TIMEOUT);
        self.s = Some(Session {
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
            ready: false,
            cookies_ok: false,
            closed: Vec::new(),
            asks: Vec::new(),
            site: None,
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
        let urls = if restore { s.data.session.clone() } else { vec![self.cfg.homepage.clone()] };
        let active = if restore { s.data.active.min(urls.len() - 1) } else { 0 };
        // Only the active tab is loaded; the rest load on first click.
        for url in urls {
            s.tabs.push(Tab::new(self.ctx.next(), url, None));
        }
        s.active = active;
        self.activate(active);
        if self.cfg.updates && !self.update_checked {
            self.update_checked = true;
            update::check(self.ctx.proxy.clone(), false);
        }
    }

    fn activate(&mut self, idx: usize) {
        let Some(s) = self.s.as_mut() else { return };
        if idx >= s.tabs.len() {
            return;
        }
        s.close_panel();
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
            let config = wk::config(&s.store, self.cfg.adblock);
            tab.view = self.ctx.build(tab.id, Some(&tab.url), config, rect).ok();
            if let Some(v) = &tab.view {
                wk::set_rules(v, self.cfg.adblock);
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
        let (Some(view), Some(s)) = (view, self.s.as_mut()) else { return };
        wk::set_rules(&view, self.cfg.adblock);
        let _ = view.zoom(s.zoom);
        let at = (s.active + 1).min(s.tabs.len());
        s.tabs.insert(at, Tab::new(id, String::new(), Some(view)));
        self.activate(at);
    }

    fn close_tab(&mut self, idx: usize) {
        let Some(s) = self.s.as_mut() else { return };
        if idx >= s.tabs.len() {
            return;
        }
        let tab = s.tabs.remove(idx);
        if tab.url.starts_with("http") {
            s.closed.push(tab.url.clone());
        }
        drop(tab);
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
        if started {
            // Anti-spoofing: the address bar keeps the last finished URL until this load
            // completes; only the spinner changes.
            t.loading = true;
            if i == s.active {
                s.site = None; // describes the page being left
            }
            let js = format!("setLoading({id},true)");
            return s.eval(&js);
        }
        t.loading = false;
        if !url.is_empty() {
            t.url = url.clone();
        }
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
                h.push(HistoryEntry { url, title, time: vault::now_ms() });
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

    fn icon(&mut self, id: u32, res: String) {
        let Some(s) = self.s.as_mut() else { return };
        let Some(i) = s.tab_idx(id) else { return };
        let data: String = serde_json::from_str(&res).unwrap_or_default();
        if data.is_empty() {
            return; // still loading, polled again
        }
        let t = &mut s.tabs[i];
        t.fav_tries = 0;
        let valid = data.strip_prefix("data:image/png;base64,").is_some_and(|b| {
            b.len() <= FAV_MAX && b.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'/' | b'='))
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
            Ok(None) if manual => s.toast(&format!("Crow {} je najnovšia verzia", update::CURRENT)),
            Err(e) if manual => s.toast(&format!("Kontrola aktualizácií zlyhala: {e}")),
            _ => {}
        }
        if let Some(s) = &self.s {
            s.sync(&self.cfg);
            s.refresh_panel(&self.cfg);
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
            let (site, rest): (Vec<_>, Vec<_>) = cookies.into_iter().partition(|c| cookie_for(&host, &c.domain));
            s.dirty = true;
            let p = self.ctx.proxy.clone();
            let rest = RefCell::new(Some(rest));
            return wk::delete_cookies(&s.store, &site, move || {
                if let Some(cookies) = rest.take() {
                    let _ = p.send_event(UserEvent::SiteCookies { cookies, clear: false });
                }
            });
        }
        let (certs, errors) = s
            .view()
            .filter(|_| url.starts_with("https:"))
            .and_then(wk::cert_chain)
            .map(|(chain, errors)| (chain.iter().filter_map(|d| cert::summarize(d)).collect(), errors))
            .unwrap_or((Vec::new(), Vec::new()));
        let data = json!({
            "url": url,
            "host": host,
            "scheme": url.split(':').next().unwrap_or(""),
            "certs": certs,
            "errors": errors,
            "cookies": cookies.iter().filter(|c| cookie_for(&host, &c.domain)).count(),
            "adblock": self.cfg.adblock,
        });
        if let Some(v) = &s.site {
            let _ = v.evaluate_script(&format!("update({data})"));
            return;
        }
        let html = include_str!("siteinfo.html").replace("/*DATA*/", &data.to_string().replace("</", "<\\/"));
        let h = if s.site_h > 0.0 { s.site_h } else { 260.0 };
        s.site = self.ctx.ui_view(html, site_rect(&self.ctx.window, s.site_x, h));
    }

    fn download_ask(&mut self, key: u32, tab: Option<u32>, url: String, name: String) {
        let Some(s) = self.s.as_mut() else {
            return wk::decide_download(key, false, &url, None);
        };
        let id = self.ctx.next();
        s.asks.push(Ask { id, key, tab, url, name });
        if s.asks.len() == 1 {
            self.resize();
        }
        if let Some(s) = &self.s {
            s.sync(&self.cfg);
        }
    }

    fn download_started(&mut self, url: String, path: PathBuf) {
        let Some(s) = self.s.as_mut() else { return };
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        s.data.downloads.push(Download {
            url,
            path: path.to_string_lossy().into_owned(),
            state: "active".into(),
            time: vault::now_ms(),
        });
        s.dirty = true;
        s.toast(&format!("Sťahuje sa: {name}"));
        s.sync(&self.cfg);
        s.refresh_panel(&self.cfg);
    }

    fn download_done(&mut self, url: String, ok: bool) {
        let Some(s) = self.s.as_mut() else { return };
        let Some(d) = s.data.downloads.iter_mut().rev().find(|d| d.url == url && d.state == "active") else {
            return;
        };
        d.state = if ok { "done" } else { "failed" }.into();
        let name = d.path.rsplit('/').next().unwrap_or("").to_string();
        s.dirty = true;
        s.toast(&if ok { format!("Stiahnuté: {name}") } else { format!("Sťahovanie zlyhalo: {name}") });
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
        if s.saving && after == After::Nothing {
            return;
        }
        s.saving = true;
        let p = self.ctx.proxy.clone();
        wk::export_cookies(&s.store, move |c| {
            let _ = p.send_event(UserEvent::Cookies(c, after));
        });
    }

    fn saved(&mut self, cookies: Vec<CookieRec>, after: After) {
        let Some(s) = self.s.as_mut() else { return };
        if !s.cookies_ok {
            // Import was never confirmed: keep vault cookies WebKit may not have received yet.
            let mut cookies = cookies;
            let missing: Vec<CookieRec> = s
                .data
                .cookies
                .drain(..)
                .filter(|o| !cookies.iter().any(|c| c.name == o.name && c.domain == o.domain && c.path == o.path))
                .collect();
            cookies.extend(missing);
            s.data.cookies = cookies;
        } else {
            s.data.cookies = cookies;
        }
        s.save_now(&self.cfg);
        s.saving = false;
        self.after_save(after);
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
                let (Some(from), Ok(to)) = (id.parse().ok().and_then(|id| s.tab_idx(id)), to.parse::<usize>()) else {
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
                let Some(asset) = s.update.as_ref().and_then(|u| u.asset.clone()) else { return };
                if asset.sha256.is_none() {
                    return s.toast("Inštalátor nemá kontrolný súčet, stiahni ho z GitHubu ručne");
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
                let i = if n >= 9 { s.tabs.len().saturating_sub(1) } else { n.saturating_sub(1) };
                self.activate(i);
            }
            "next-tab" | "prev-tab" => {
                let len = s.tabs.len().max(1);
                let i = if name == "next-tab" { (s.active + 1) % len } else { (s.active + len - 1) % len };
                self.activate(i);
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
                let url = to_url(arg.trim(), self.cfg.search_prefix());
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
                    "zoom-in" => ZOOM_STEPS.iter().copied().find(|x| *x > z + 0.001).unwrap_or(z),
                    "zoom-out" => ZOOM_STEPS.iter().rev().copied().find(|x| *x < z - 0.001).unwrap_or(z),
                    _ => self.cfg.zoom,
                };
                for v in s.loaded() {
                    let _ = v.zoom(s.zoom);
                }
                s.toast(&format!("Priblíženie {} %", (s.zoom * 100.0).round()));
            }
            "find" if arg.is_empty() => self.set_find(true),
            "find" => {
                #[derive(Deserialize)]
                struct Req {
                    q: String,
                    dir: String,
                }
                let (Ok(r), Some(v)) = (serde_json::from_str::<Req>(arg), s.view()) else { return };
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
                let Some(t) = s.cur().filter(|t| t.url.starts_with("http")) else { return };
                let (url, title) = (t.url.clone(), t.label());
                let bms = &mut s.data.bookmarks;
                if let Some(i) = bms.iter().position(|b| b.url == url) {
                    bms.remove(i);
                    s.toast("Odstránené z obľúbených");
                } else {
                    bms.push(Bookmark { url, title });
                    s.toast("Pridané do obľúbených");
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
                s.site = None;
                if s.panel.is_some() && s.panel_section == arg {
                    s.close_panel();
                } else if let Some(p) = &s.panel {
                    let _ = p.evaluate_script(&format!("show({})", js(arg)));
                    s.panel_section = arg.to_string();
                } else {
                    let data = s.panel_data(&self.cfg, arg).replace("</", "<\\/");
                    let html = include_str!("panel.html").replace("/*DATA*/", &data);
                    s.panel = self.ctx.ui_view(html, content_rect(&self.ctx.window, s.top()));
                    s.panel_section = arg.to_string();
                }
                s.sync(&self.cfg);
            }
            "panel-section" => {
                s.panel_section = arg.to_string();
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
                        for v in s.loaded() {
                            wk::set_rules(v, self.cfg.adblock);
                        }
                    }
                    _ => {}
                }
                s.sync(&self.cfg);
                self.persist(After::Nothing);
            }
            "clear" => {
                match arg {
                    "history" => s.data.history.clear(),
                    "downloads" => s.data.downloads.retain(|d| d.state == "active"),
                    "cookies" => {
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
                if s.data.downloads.iter().any(|d| d.path == arg && d.state == "done") {
                    wk::reveal(arg);
                }
            }
            "dl-allow" | "dl-deny" => {
                let Some(i) = arg.parse().ok().and_then(|id: u32| s.asks.iter().position(|a| a.id == id)) else {
                    return;
                };
                let a = s.asks.remove(i);
                let view = a.tab.and_then(|t| s.tab_idx(t)).and_then(|i| s.tabs[i].view.as_ref());
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
                let Ok(mut r) = serde_json::from_str::<Req>(arg) else { return };
                let (ok, msg) = if !s.vault.verify(&r.old) {
                    (false, "Nesprávne aktuálne heslo")
                } else if r.new.chars().count() < 8 {
                    (false, "Nové heslo musí mať aspoň 8 znakov")
                } else {
                    // Transactional: the new vault must be on disk (tmp + fsync + rename)
                    // before it replaces the in-memory key or the UI reports success.
                    let vault = Vault::create(&r.new);
                    s.snapshot();
                    s.data.settings = Some(self.cfg.clone());
                    match vault.save(&s.data) {
                        Ok(()) => {
                            s.vault = vault;
                            s.dirty = false;
                            (true, "Heslo bolo zmenené")
                        }
                        Err(e) => {
                            eprintln!("password change: vault save failed: {e}");
                            (false, "Heslo sa nepodarilo uložiť, platí pôvodné")
                        }
                    }
                };
                r.old.zeroize();
                r.new.zeroize();
                if let Some(p) = &s.panel {
                    let _ = p.evaluate_script(&format!("pwResult({ok},{})", js(msg)));
                }
            }
            "site-info" => {
                // A click on the lock first blurs (closes) an open popover: treat as toggle.
                if s.site.take().is_some() || s.site_closed.elapsed() < Duration::from_millis(300) {
                    return;
                }
                s.site_x = arg.parse().unwrap_or(150.0);
                self.request_site_cookies(false);
            }
            "site-clear" => self.request_site_cookies(true),
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
    if wk::is_light(window) { BG_LIGHT } else { BG_DARK }
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

fn to_url(input: &str, search_prefix: &str) -> String {
    // Explicit schemes are navigated as typed, never sent to the search engine.
    const SCHEMES: &[&str] = &["http://", "https://", "file:", "about:", "data:", "blob:", "mailto:"];
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
    let local = matches!(host.as_str(), "localhost" | "127.0.0.1" | "[::1]") || host.ends_with(".local");
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
