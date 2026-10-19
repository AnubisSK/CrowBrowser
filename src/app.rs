use crate::{
    adblock,
    settings::{ENGINES, Settings},
    vault::{self, Bookmark, CookieRec, Download, HistoryEntry, MAX_HISTORY, Vault, VaultData},
    wk,
};
use objc2::rc::Retained;
use objc2_web_kit::{WKWebViewConfiguration, WKWebsiteDataStore};
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
    window::{Theme, Window},
};
use wry::{
    NewWindowFeatures, NewWindowResponse, PageLoadEvent, RGBA, Rect, WebView, WebViewBuilder,
    WebViewBuilderExtMacos, WebViewExtMacOS, dpi,
};
use zeroize::Zeroize;

const TABS_H: f64 = 36.0;
const NAV_H: f64 = 40.0;
const FIND_H: f64 = 36.0;
const BG_DARK: RGBA = (0x1f, 0x1f, 0x23, 0xff);
const BG_LIGHT: RGBA = (0xf4, 0xf4, 0xf6, 0xff);
const ZOOM_STEPS: &[f64] = &[0.5, 0.67, 0.75, 0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0];
const TICK: Duration = Duration::from_secs(30);
const QUIT_TIMEOUT: Duration = Duration::from_secs(3);

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

pub enum UserEvent {
    Menu(String),
    Ipc(String),
    PageLoad { tab: u32, started: bool, url: String },
    Title { tab: u32, title: String },
    NewTab(String),
    AdoptTab(u32),
    DownloadStarted { url: String, path: PathBuf },
    DownloadDone { url: String, ok: bool },
    FindResult(String),
    Cookies(Vec<CookieRec>, After),
    CookiesRestored,
    RulesReady,
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
        config: Retained<WKWebViewConfiguration>,
        rect: Rect,
    ) -> wry::Result<WebView> {
        let (p1, p2, p3, p4) = (self.proxy.clone(), self.proxy.clone(), self.proxy.clone(), self.proxy.clone());
        let ctx = self.clone();
        let mut b = WebViewBuilder::new()
            .with_webview_configuration(config)
            .with_background_color(bg_of(&self.window))
            .with_bounds(rect)
            .with_on_page_load_handler(move |ev, url| {
                let started = matches!(ev, PageLoadEvent::Started);
                let _ = p1.send_event(UserEvent::PageLoad { tab: id, started, url });
            })
            .with_document_title_changed_handler(move |title| {
                let _ = p2.send_event(UserEvent::Title { tab: id, title });
            })
            .with_download_started_handler(move |url, path| {
                let _ = p3.send_event(UserEvent::DownloadStarted { url, path: path.clone() });
                true
            })
            .with_download_completed_handler(move |url, _, ok| {
                let _ = p4.send_event(UserEvent::DownloadDone { url, ok });
            })
            .with_new_window_req_handler(move |url, features| ctx.popup(url, features));
        if let Some(url) = url {
            b = b.with_url(url);
        }
        b.build_as_child(&*self.window)
    }

    /// window.open / target=_blank: a real WKWebView keeping `window.opener`
    /// (required by OAuth login popups), adopted as a new tab.
    fn popup(&self, url: String, features: NewWindowFeatures) -> NewWindowResponse {
        let id = self.next();
        let rect = content_rect(&self.window, TABS_H + NAV_H);
        match self.build(id, None, features.opener.target_configuration, rect) {
            Ok(view) => {
                let webview = Retained::into_super(view.webview());
                PENDING.with(|p| p.borrow_mut().push((id, view)));
                self.send(UserEvent::AdoptTab(id));
                NewWindowResponse::Create { webview }
            }
            Err(_) => {
                self.send(UserEvent::NewTab(url));
                NewWindowResponse::Deny
            }
        }
    }

    fn ui_view(&self, html: String, rect: Rect) -> Option<WebView> {
        let p = self.proxy.clone();
        WebViewBuilder::new()
            .with_html(html)
            .with_background_color(bg_of(&self.window))
            .with_bounds(rect)
            .with_focused(true)
            .with_ipc_handler(move |req| {
                let _ = p.send_event(UserEvent::Ipc(req.into_body()));
            })
            .build_as_child(&*self.window)
            .ok()
    }
}

struct Tab {
    id: u32,
    url: String,
    title: String,
    view: Option<WebView>, // None = unloaded to save memory
    hidden_since: Instant,
    loading: bool,
}

impl Tab {
    fn new(id: u32, url: String, view: Option<WebView>) -> Self {
        Self { id, url, title: String::new(), view, hidden_since: Instant::now(), loading: false }
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
    store: Retained<WKWebsiteDataStore>,
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
    closed: Vec<String>,
}

impl Session {
    fn top(&self) -> f64 {
        TABS_H + NAV_H + if self.find_open { FIND_H } else { 0.0 }
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

    fn save_now(&mut self) {
        self.snapshot();
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
            .map(|t| json!({ "id": t.id, "title": t.label(), "loading": t.loading, "asleep": t.view.is_none() }))
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
}

impl App {
    pub fn new(window: Window, proxy: EventLoopProxy<UserEvent>, cfg: Settings) -> Self {
        let p = proxy.clone();
        wk::compile_rules(adblock::rules_json(), move || {
            let _ = p.send_event(UserEvent::RulesReady);
        });
        let ctx = TabCtx { window: Rc::new(window), proxy, next_id: Rc::new(Cell::new(1)) };
        let mut app = Self {
            ctx,
            cfg,
            lock: None,
            s: None,
            exit: false,
            next_tick: Instant::now() + TICK,
            quit_at: None,
        };
        app.show_lock();
        app
    }

    fn show_lock(&mut self) {
        let mode = if Vault::exists() { "unlock" } else { "create" };
        let html = include_str!("lock.html").replace("/*MODE*/", &js(mode));
        self.lock = self.ctx.ui_view(html, full_rect(&self.ctx.window));
    }

    fn lock_fail(&self, msg: &str) {
        if let Some(l) = &self.lock {
            let _ = l.evaluate_script(&format!("fail({})", js(msg)));
        }
    }

    pub fn wake(&self) -> Instant {
        self.quit_at.map_or(self.next_tick, |q| q.min(self.next_tick))
    }

    pub fn resize(&self) {
        let w = &self.ctx.window;
        if let Some(l) = &self.lock {
            let _ = l.set_bounds(full_rect(w));
        }
        if let Some(s) = &self.s {
            let _ = s.toolbar.set_bounds(toolbar_rect(w, s.top()));
            let c = content_rect(w, s.top());
            for v in s.loaded() {
                let _ = v.set_bounds(c);
            }
            if let Some(p) = &s.panel {
                let _ = p.set_bounds(c);
            }
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
            UserEvent::DownloadStarted { url, path } => self.download_started(url, path),
            UserEvent::DownloadDone { url, ok } => self.download_done(url, ok),
            UserEvent::FindResult(r) => {
                if let Some(s) = &self.s {
                    s.eval(&format!("findCount({})", r.trim().parse::<u32>().unwrap_or(0)));
                }
            }
            UserEvent::Cookies(c, after) => self.saved(c, after),
            UserEvent::CookiesRestored => self.restored(),
            UserEvent::RulesReady => {
                if let (Some(s), true) = (&self.s, self.cfg.adblock) {
                    for v in s.loaded() {
                        wk::set_rules(&v.manager(), true);
                    }
                }
            }
        }
    }

    pub fn timer(&mut self) {
        let now = Instant::now();
        if self.quit_at.is_some_and(|q| now >= q) {
            // Cookie export never answered; save what we have.
            if let Some(s) = &mut self.s {
                s.save_now();
            }
            self.exit = true;
            return;
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
            s.save_now();
        }
    }

    fn command(&mut self, cmd: &str) {
        let (name, arg) = cmd.split_once(':').unwrap_or((cmd, ""));
        match name {
            "drag" => {
                let _ = self.ctx.window.drag_window();
            }
            "quit" => self.quit(),
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

    fn start(&mut self, vault: Vault, data: VaultData) {
        let w = &self.ctx.window;
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
            dirty: false,
            saving: false,
            ready: false,
            closed: Vec::new(),
        });
    }

    fn restored(&mut self) {
        let Some(s) = self.s.as_mut() else { return };
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
                let _ = v.zoom(s.zoom);
            }
        }
        if let Some(v) = &tab.view {
            let _ = v.set_bounds(rect);
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
            t.url = url;
            if let Some(v) = &t.view {
                let _ = v.load_url(&t.url);
                let _ = v.focus();
            }
        }
        s.sync(&self.cfg);
    }

    fn page_load(&mut self, id: u32, started: bool, url: String) {
        let Some(s) = self.s.as_mut() else { return };
        let Some(i) = s.tab_idx(id) else { return };
        let t = &mut s.tabs[i];
        t.loading = started;
        if !url.is_empty() {
            t.url = url.clone();
        }
        if !started && url.starts_with("http") {
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
            s.save_now();
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
        s.data.cookies = cookies;
        s.save_now();
        s.saving = false;
        self.after_save(after);
    }

    fn after_save(&mut self, after: After) {
        match after {
            After::Nothing => {}
            After::Quit => self.exit = true,
            After::Lock => {
                if let Some(s) = self.s.take() {
                    wk::clear_store(&s.store);
                }
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
                self.cfg.save();
                let w = &self.ctx.window;
                match k {
                    "theme" => {
                        w.set_theme(theme_of(&self.cfg.theme));
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
                            wk::set_rules(&v.manager(), self.cfg.adblock);
                        }
                    }
                    _ => {}
                }
                s.sync(&self.cfg);
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
            "open-file" | "reveal" => {
                if s.data.downloads.iter().any(|d| d.path == arg) {
                    let mut cmd = std::process::Command::new("open");
                    if name == "reveal" {
                        cmd.arg("-R");
                    }
                    let _ = cmd.arg(arg).spawn();
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
                    s.vault = Vault::create(&r.new);
                    (true, "Heslo bolo zmenené")
                };
                r.old.zeroize();
                r.new.zeroize();
                if let Some(p) = &s.panel {
                    let _ = p.evaluate_script(&format!("pwResult({ok},{})", js(msg)));
                }
                if ok {
                    s.dirty = true;
                    self.persist(After::Nothing);
                }
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
    if window.theme() == Theme::Light { BG_LIGHT } else { BG_DARK }
}

fn rect(window: &Window, y: f64, h: Option<f64>) -> Rect {
    let size = window.inner_size().to_logical::<f64>(window.scale_factor());
    let h = h.unwrap_or(size.height - y).max(0.0);
    Rect {
        position: dpi::LogicalPosition::new(0.0, y).into(),
        size: dpi::LogicalSize::new(size.width, h).into(),
    }
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
    let has_scheme = ["http://", "https://", "file://", "about:"]
        .iter()
        .any(|s| input.starts_with(s));
    if has_scheme {
        input.to_string()
    } else if !input.contains(' ') && (input.contains('.') || input.starts_with("localhost")) {
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
