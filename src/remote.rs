//! Remote execution client: routes tab content through a `crow-server` node. One runtime thread
//! carries every connection; all of them are dropped with the session (lock, quit).
//!
//! - Eco Reader: the node returns a sanitized `ReaderDoc`, rendered here to static HTML.
//! - Air-Gap RBI: the node's frames reach the tab over a loopback WebSocket (one-time ticket), so
//!   tab views keep having no IPC and frames never pass through the UI thread.
//! - Secure Tunnel: a loopback SOCKS5 port for the engines, one encrypted connection per stream.
//!
//! Failures are never retried directly: without the node, content does not load.

use crate::{app::UserEvent, vault::RemoteServer};
use chacha20poly1305::aead::{OsRng, rand_core::RngCore};
use crow_proto::{
    Block, Kind, Open, RbiCommand, RbiEvent, ReaderDoc, Reply, Rx, Span, Tx, connect, derive_psk,
    relay, socks5,
};
use futures_util::{SinkExt, StreamExt};
use std::{
    collections::HashMap,
    fmt::Write as _,
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};
use tao::event_loop::EventLoopProxy;
use tokio::{
    net::{TcpListener, TcpStream},
    runtime::Runtime,
    sync::{OnceCell, mpsc},
    task::JoinHandle,
};
use tokio_tungstenite::{
    WebSocketStream, accept_hdr_async,
    tungstenite::{
        Message,
        handshake::server::{ErrorResponse, Request, Response},
    },
};
use zeroize::Zeroizing;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Allows for a page with images, or the node starting a browser.
const REPLY_TIMEOUT: Duration = Duration::from_secs(45);
const PAGE_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RemoteMode {
    Direct,
    Eco,
    Rbi,
    Tunnel,
}

impl RemoteMode {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "direct" => Some(Self::Direct),
            "eco" => Some(Self::Eco),
            "rbi" => Some(Self::Rbi),
            "tunnel" => Some(Self::Tunnel),
            _ => None,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Eco => "eco",
            Self::Rbi => "rbi",
            Self::Tunnel => "tunnel",
        }
    }

    /// Top-level navigations are taken over instead of being loaded by the tab's engine.
    pub fn intercepts(self) -> bool {
        matches!(self, Self::Eco | Self::Rbi)
    }
}

/// Finished Eco page, ready to show.
pub struct ReaderPage {
    pub url: String,
    pub title: String,
    pub html: Zeroizing<String>,
}

/// What a tab must load to show an RBI session. Events of other sessions are stale.
pub struct RbiPage {
    pub session: u64,
    pub html: String,
}

struct Pending {
    tab: u32,
    session: u64,
    url: Zeroizing<String>,
    commands: mpsc::Receiver<RbiCommand>,
}

type Tickets = Arc<Mutex<HashMap<String, Pending>>>;

#[derive(Clone)]
struct Link {
    authority: Arc<str>,
    token: Arc<Zeroizing<String>>,
    psk: Arc<OnceCell<Zeroizing<[u8; 32]>>>,
}

impl Link {
    async fn open(&self) -> Result<(Tx<TcpStream>, Rx<TcpStream>), &'static str> {
        let psk = self
            .psk
            .get_or_try_init(|| {
                let token = self.token.clone();
                async move {
                    tokio::task::spawn_blocking(move || derive_psk(&token))
                        .await
                        .map_err(|_| "remote_err_connect")
                }
            })
            .await?;
        let attempt = async {
            let stream = TcpStream::connect(&*self.authority).await.ok()?;
            let _ = stream.set_nodelay(true);
            connect(stream, &self.authority, psk).await.ok()
        };
        tokio::time::timeout(CONNECT_TIMEOUT, attempt)
            .await
            .ok()
            .flatten()
            .ok_or("remote_err_connect")
    }

    /// Connects and asks for `open`. Node-side refusals come back as their error code.
    async fn request(&self, open: &Open) -> Result<(Tx<TcpStream>, Rx<TcpStream>), String> {
        let (mut tx, mut rx) = self.open().await.map_err(str::to_string)?;
        tx.send_json(open)
            .await
            .map_err(|_| "remote_err_connect".to_string())?;
        match tokio::time::timeout(REPLY_TIMEOUT, rx.recv_json::<Reply>()).await {
            Ok(Some(Reply::Ok)) => Ok((tx, rx)),
            Ok(Some(Reply::Err { code })) => Err(code),
            _ => Err("remote_err_connect".into()),
        }
    }
}

/// Error codes come from the node: only known `locales.json` keys reach the UI.
pub fn known_code(code: &str) -> &'static str {
    const KNOWN: &[&str] = &[
        "remote_err_connect",
        "remote_err_blocked",
        "remote_err_fetch",
        "remote_err_not_html",
        "remote_err_too_large",
        "remote_err_mode",
        "remote_err_rbi",
        "remote_err_busy",
        "remote_err_ram",
    ];
    KNOWN
        .iter()
        .find(|k| **k == code)
        .copied()
        .unwrap_or("remote_err_connect")
}

/// Connection to the configured node, alive for the session (or until the settings change).
pub struct Remote {
    rt: Option<Runtime>,
    link: Link,
    proxy: EventLoopProxy<UserEvent>,
    surface_port: u16,
    tickets: Tickets,
    sessions: HashMap<u32, (u64, mpsc::Sender<RbiCommand>)>,
    next_session: u64,
    socks: Option<(u16, JoinHandle<()>)>,
}

impl Drop for Remote {
    fn drop(&mut self) {
        // Closes every connection now; the node wipes each session as its socket goes.
        if let Some(rt) = self.rt.take() {
            rt.shutdown_background();
        }
    }
}

impl Remote {
    pub fn new(server: &RemoteServer, proxy: EventLoopProxy<UserEvent>) -> Option<Self> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .max_blocking_threads(2)
            .enable_all()
            .build()
            .ok()?;
        let authority = if server.host.contains(':') {
            format!("[{}]:{}", server.host, server.port)
        } else {
            format!("{}:{}", server.host, server.port)
        };
        let link = Link {
            authority: authority.into(),
            token: Arc::new(Zeroizing::new(server.token.clone())),
            psk: Arc::new(OnceCell::new()),
        };
        let tickets: Tickets = Arc::default();
        let listener = rt.block_on(TcpListener::bind(("127.0.0.1", 0))).ok()?;
        let surface_port = listener.local_addr().ok()?.port();
        rt.spawn(surface_listener(
            listener,
            tickets.clone(),
            link.clone(),
            proxy.clone(),
        ));
        Some(Self {
            rt: Some(rt),
            link,
            proxy,
            surface_port,
            tickets,
            sessions: HashMap::new(),
            next_session: 1,
            socks: None,
        })
    }

    fn spawn(&self, f: impl Future<Output = ()> + Send + 'static) -> Option<JoinHandle<()>> {
        self.rt.as_ref().map(|rt| rt.spawn(f))
    }

    /// Reachability and token check (toolbar status, settings test).
    pub fn ping(&self) {
        let (link, proxy) = (self.link.clone(), self.proxy.clone());
        self.spawn(async move {
            let result = link
                .request(&Open::Ping)
                .await
                .map(|_| ())
                .map_err(|c| known_code(&c));
            let _ = proxy.send_event(UserEvent::RemotePing(result));
        });
    }

    /// Fetches `url` through the Eco engine; the answer arrives as `UserEvent::RemoteReader`.
    pub fn reader(&self, tab: u32, seq: u32, url: String) {
        let (link, proxy) = (self.link.clone(), self.proxy.clone());
        self.spawn(async move {
            let result = eco(&link, url).await;
            let _ = proxy.send_event(UserEvent::RemoteReader { tab, seq, result });
        });
    }

    /// New RBI session for `tab` starting at `url`; replaces any earlier one of the tab.
    pub fn rbi_open(&mut self, tab: u32, url: String) -> RbiPage {
        let session = self.next_session;
        self.next_session += 1;
        let mut raw = [0u8; 32];
        OsRng.fill_bytes(&mut raw);
        let ticket = raw.iter().fold(String::with_capacity(64), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        });
        let (commands_tx, commands) = mpsc::channel(32);
        let mut tickets = self.tickets.lock().unwrap_or_else(PoisonError::into_inner);
        tickets.retain(|_, p| p.tab != tab);
        tickets.insert(
            ticket.clone(),
            Pending {
                tab,
                session,
                url: Zeroizing::new(url),
                commands,
            },
        );
        drop(tickets);
        self.sessions.insert(tab, (session, commands_tx));
        let html = include_str!("rbi.html")
            .replace("__PORT__", &self.surface_port.to_string())
            .replace("__TICKET__", &ticket);
        RbiPage { session, html }
    }

    /// Queues `cmd` for the tab's session; a full queue drops it (navigation is user-repeatable).
    pub fn rbi_command(&self, tab: u32, cmd: RbiCommand) {
        if let Some((_, tx)) = self.sessions.get(&tab) {
            let _ = tx.try_send(cmd);
        }
    }

    /// Ends the tab's session (mode change, tab closed); the node wipes it.
    pub fn rbi_close(&mut self, tab: u32) {
        self.sessions.remove(&tab);
        self.tickets
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|_, p| p.tab != tab);
    }

    /// The session ended on its own. A newer one of the same tab stays.
    pub fn rbi_ended(&mut self, tab: u32, session: u64) {
        if self.sessions.get(&tab).is_some_and(|(s, _)| *s == session) {
            self.sessions.remove(&tab);
        }
    }

    /// Starts the loopback SOCKS5 port (idempotent).
    pub fn tunnel_start(&mut self) -> Option<u16> {
        if let Some((port, _)) = &self.socks {
            return Some(*port);
        }
        let listener = self
            .rt
            .as_ref()?
            .block_on(TcpListener::bind(("127.0.0.1", 0)))
            .ok()?;
        let port = listener.local_addr().ok()?.port();
        let handle = self.spawn(socks_listener(listener, self.link.clone()))?;
        self.socks = Some((port, handle));
        Some(port)
    }

    pub fn tunnel_stop(&mut self) {
        if let Some((_, h)) = self.socks.take() {
            h.abort();
        }
    }
}

async fn eco(link: &Link, url: String) -> Result<ReaderPage, &'static str> {
    let open = Open::Eco { url };
    let (_tx, mut rx) = link.request(&open).await.map_err(|c| known_code(&c))?;
    let frame = tokio::time::timeout(REPLY_TIMEOUT, rx.recv())
        .await
        .ok()
        .flatten()
        .ok_or("remote_err_connect")?;
    if frame.kind != Kind::Json {
        return Err("remote_err_connect");
    }
    let doc: ReaderDoc = serde_json::from_slice(&frame.data).map_err(|_| "remote_err_connect")?;
    if !(doc.url.starts_with("https://") || doc.url.starts_with("http://")) {
        return Err("remote_err_connect");
    }
    Ok(ReaderPage {
        url: doc.url.clone(),
        title: doc.title.clone(),
        html: reader_html(&doc),
    })
}

async fn socks_listener(listener: TcpListener, link: Link) {
    while let Ok((stream, _)) = listener.accept().await {
        let link = link.clone();
        tokio::spawn(async move {
            let _ = stream.set_nodelay(true);
            tunnel_stream(stream, link).await;
        });
    }
}

async fn tunnel_stream(mut local: TcpStream, link: Link) {
    let Ok((host, port)) = socks5::handshake(&mut local).await else {
        return;
    };
    let open = Open::Tunnel { host, port };
    match link.request(&open).await {
        Ok((mut tx, mut rx)) => {
            if socks5::reply(&mut local, socks5::SUCCEEDED).await.is_ok() {
                relay(local, &mut tx, &mut rx).await;
            }
        }
        Err(code) => {
            let reason = if code == "remote_err_blocked" {
                socks5::NOT_ALLOWED
            } else {
                socks5::UNREACHABLE
            };
            let _ = socks5::reply(&mut local, reason).await;
        }
    }
}

async fn surface_listener(
    listener: TcpListener,
    tickets: Tickets,
    link: Link,
    proxy: EventLoopProxy<UserEvent>,
) {
    while let Ok((stream, _)) = listener.accept().await {
        let (tickets, link, proxy) = (tickets.clone(), link.clone(), proxy.clone());
        tokio::spawn(async move {
            let found = Arc::new(Mutex::new(None));
            let slot = found.clone();
            // The ticket is the only credential, spent on first use. The return type is
            // tungstenite's `Callback` signature, so the large `Err` cannot be boxed.
            #[allow(clippy::result_large_err)]
            let check = move |req: &Request, resp: Response| -> Result<Response, ErrorResponse> {
                let ticket = req.uri().path().strip_prefix("/rbi/").unwrap_or("");
                match tickets
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .remove(ticket)
                {
                    Some(p) => {
                        *slot.lock().unwrap_or_else(PoisonError::into_inner) = Some(p);
                        Ok(resp)
                    }
                    None => Err(ErrorResponse::new(None)),
                }
            };
            let Ok(ws) = accept_hdr_async(stream, check).await else {
                return;
            };
            let Some(p) = found.lock().unwrap_or_else(PoisonError::into_inner).take() else {
                return;
            };
            surface(ws, p, link, proxy).await;
        });
    }
}

/// Only what a page surface legitimately produces is forwarded from it.
fn page_command(text: &str) -> Option<RbiCommand> {
    let cmd: RbiCommand = serde_json::from_str(text).ok()?;
    matches!(
        cmd,
        RbiCommand::Resize { .. }
            | RbiCommand::Visible { .. }
            | RbiCommand::Mouse { .. }
            | RbiCommand::Wheel { .. }
            | RbiCommand::Key { .. }
    )
    .then_some(cmd)
}

async fn surface(
    ws: WebSocketStream<TcpStream>,
    mut p: Pending,
    link: Link,
    proxy: EventLoopProxy<UserEvent>,
) {
    let (tab, session) = (p.tab, p.session);
    let error = pump(ws, &mut p, link, &proxy).await.err();
    let _ = proxy.send_event(UserEvent::RemoteClosed {
        tab,
        session,
        error,
    });
}

async fn pump(
    ws: WebSocketStream<TcpStream>,
    p: &mut Pending,
    link: Link,
    proxy: &EventLoopProxy<UserEvent>,
) -> Result<(), &'static str> {
    let (tab, session) = (p.tab, p.session);
    let (mut page_tx, mut page_rx) = ws.split();
    // The surface reports its size first: the remote viewport starts out matching it.
    let first = tokio::time::timeout(PAGE_TIMEOUT, page_rx.next())
        .await
        .ok()
        .flatten();
    let Some(Ok(Message::Text(first))) = first else {
        return Ok(());
    };
    let Some(RbiCommand::Resize { width, height }) = page_command(&first) else {
        return Ok(());
    };

    let open = Open::Rbi {
        url: p.url.to_string(),
        width,
        height,
    };
    let (mut tx, mut rx) = link.request(&open).await.map_err(|c| known_code(&c))?;
    let _ = proxy.send_event(UserEvent::RemoteRbi {
        tab,
        session,
        event: RbiEvent::Loading { on: true },
    });
    let gone = "remote_err_connect";
    loop {
        tokio::select! {
            m = page_rx.next() => match m {
                Some(Ok(Message::Text(t))) => {
                    let t = Zeroizing::new(t.to_string());
                    if let Some(cmd) = page_command(&t) {
                        tx.send_json(&cmd).await.map_err(|_| gone)?;
                    }
                }
                Some(Ok(Message::Close(_)) | Err(_)) | None => break,
                Some(Ok(_)) => {}
            },
            c = p.commands.recv() => match c {
                Some(cmd) => tx.send_json(&cmd).await.map_err(|_| gone)?,
                None => break,
            },
            f = rx.recv() => {
                let f = f.ok_or(gone)?;
                match f.kind {
                    Kind::Binary => {
                        if page_tx.send(Message::binary(f.data.to_vec())).await.is_err() {
                            break;
                        }
                    }
                    Kind::Json => {
                        if let Ok(event) = serde_json::from_slice::<RbiEvent>(&f.data) {
                            let _ = proxy.send_event(UserEvent::RemoteRbi { tab, session, event });
                        }
                    }
                }
            }
        }
    }
    tx.close().await;
    Ok(())
}

fn esc(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
}

fn http_link(href: &str) -> bool {
    let l = href.get(..8).unwrap_or(href).to_ascii_lowercase();
    l.starts_with("https://") || l.starts_with("http://")
}

fn spans_html(spans: &[Span], out: &mut String) {
    for s in spans {
        let link = s.href.as_deref().filter(|h| http_link(h));
        if let Some(h) = link {
            out.push_str("<a href=\"");
            esc(h, out);
            out.push_str("\">");
        }
        let tags: Vec<&str> = [(s.strong, "strong"), (s.em, "em"), (s.code, "code")]
            .iter()
            .filter(|(on, _)| *on)
            .map(|(_, t)| *t)
            .collect();
        for t in &tags {
            let _ = write!(out, "<{t}>");
        }
        for (i, line) in s.text.split('\n').enumerate() {
            if i > 0 {
                out.push_str("<br>");
            }
            esc(line, out);
        }
        for t in tags.iter().rev() {
            let _ = write!(out, "</{t}>");
        }
        if link.is_some() {
            out.push_str("</a>");
        }
    }
}

const READER_STYLE: &str = "<meta charset=\"utf-8\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; img-src data:; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'\"><style>:root{color-scheme:light dark}body{margin:0;background:Canvas;color:CanvasText;font:18px/1.65 Georgia,'Noto Serif',serif}main{max-width:42em;margin:0 auto;padding:2.5em 1.25em 5em}h1{font:700 1.9em/1.2 system-ui,sans-serif;margin:0 0 .4em}h2,h3,h4,h5,h6{font-family:system-ui,sans-serif;line-height:1.3;margin:1.6em 0 .5em}.m{font:.8em system-ui,sans-serif;opacity:.65;margin-bottom:2em}a{color:#5b8def}img{display:block;max-width:100%;height:auto;margin:1.2em auto;border-radius:4px}figcaption,.c{font:.8em system-ui,sans-serif;opacity:.65;text-align:center}blockquote{margin:1em 0;padding-left:1em;border-left:3px solid #8884}pre{overflow:auto;padding:1em;background:#8882;border-radius:6px;font:.8em ui-monospace,monospace}code{font-family:ui-monospace,monospace;font-size:.88em}</style>";

/// Static page without any script (CSP `default-src 'none'`). Everything from the node is
/// escaped; links are http(s) only and come back through the tab's navigation handler.
pub fn reader_html(doc: &ReaderDoc) -> Zeroizing<String> {
    let mut o = Zeroizing::new(String::with_capacity(16 * 1024));
    let lang: String = doc
        .lang
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .take(16)
        .collect();
    let _ = write!(
        *o,
        "<!doctype html><html lang=\"{lang}\"><head>{READER_STYLE}<title>"
    );
    esc(&doc.title, &mut o);
    o.push_str("</title></head><body><main><h1>");
    esc(&doc.title, &mut o);
    o.push_str("</h1><div class=\"m\">");
    let meta: Vec<&str> = [&doc.site, &doc.byline, &doc.published]
        .iter()
        .map(|s| s.as_str())
        .filter(|s| !s.is_empty())
        .collect();
    esc(&meta.join(" · "), &mut o);
    o.push_str("</div>");
    for b in &doc.blocks {
        match b {
            Block::Heading { level, text } => {
                let l = (*level).clamp(2, 6);
                let _ = write!(*o, "<h{l}>");
                esc(text, &mut o);
                let _ = write!(*o, "</h{l}>");
            }
            Block::Paragraph { spans } => {
                o.push_str("<p>");
                spans_html(spans, &mut o);
                o.push_str("</p>");
            }
            Block::Quote { spans } => {
                o.push_str("<blockquote>");
                spans_html(spans, &mut o);
                o.push_str("</blockquote>");
            }
            Block::List { ordered, items } => {
                let tag = if *ordered { "ol" } else { "ul" };
                let _ = write!(*o, "<{tag}>");
                for item in items {
                    o.push_str("<li>");
                    spans_html(item, &mut o);
                    o.push_str("</li>");
                }
                let _ = write!(*o, "</{tag}>");
            }
            Block::Code { text } => {
                o.push_str("<pre>");
                esc(text, &mut o);
                o.push_str("</pre>");
            }
            Block::Image {
                data,
                alt,
                width,
                height,
            } => {
                if data
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'='))
                {
                    let _ = write!(*o, "<img width=\"{width}\" height=\"{height}\" alt=\"");
                    esc(alt, &mut o);
                    let _ = write!(*o, "\" src=\"data:image/webp;base64,{data}\">");
                }
            }
        }
    }
    o.push_str("</main></body></html>");
    o
}

/// Static error page of a remote mode, with a retry link when `url` is http(s).
pub fn error_html(title: &str, message: &str, url: &str, retry: &str) -> String {
    let mut o = String::with_capacity(2048);
    let _ = write!(o, "<!doctype html><html><head>{READER_STYLE}<title>");
    esc(title, &mut o);
    o.push_str("</title></head><body><main><h1>");
    esc(title, &mut o);
    o.push_str("</h1><p>");
    esc(message, &mut o);
    o.push_str("</p>");
    if http_link(url) {
        o.push_str("<p class=\"m\">");
        esc(url, &mut o);
        o.push_str("</p><p><a href=\"");
        esc(url, &mut o);
        o.push_str("\">");
        esc(retry, &mut o);
        o.push_str("</a></p>");
    }
    o.push_str("</main></body></html>");
    o
}
