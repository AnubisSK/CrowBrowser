//! Engine 2: Remote Browser Isolation. One headless browser per session, driven over the DevTools
//! protocol: its screen goes to the client as JPEG frames, the client's input comes back. Page
//! code, cookies and cache exist only on the node, on tmpfs, and are wiped at disconnect.

use crate::{Config, policy, ram::RamDir};
use base64::{Engine, engine::general_purpose::STANDARD};
use crow_proto::{Frame, Kind, KeyKind, MouseButton, MouseKind, RbiCommand, RbiEvent, Reply, Rx, Tx};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::mpsc,
};
use url::Url;
use zeroize::Zeroizing;

const START_TIMEOUT: Duration = Duration::from_secs(20);
const CALL_TIMEOUT: Duration = Duration::from_secs(15);
const JPEG_QUALITY: u32 = 55;
const MAX_WIDTH: u32 = 3840;
const MAX_HEIGHT: u32 = 2160;
const MAX_KEY_FIELD: usize = 32;

/// The browser and its profile. Dropping kills the process tree first, so nothing can write
/// after the wipe.
struct Browser {
    child: Child,
    dir: Option<RamDir>,
}

impl Browser {
    fn kill(&mut self) {
        #[cfg(unix)]
        if let Ok(pid) = i32::try_from(self.child.id()) {
            // Its own process group (`spawn`): renderers and helpers go with it.
            unsafe { libc::killpg(pid, libc::SIGKILL) };
        }
        #[cfg(windows)]
        let _ = Command::new("taskkill")
            .args(["/T", "/F", "/PID", &self.child.id().to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        self.kill();
        self.dir.take();
    }
}

#[cfg(target_os = "windows")]
fn known_paths() -> Vec<PathBuf> {
    ["ProgramFiles", "ProgramFiles(x86)", "LocalAppData"]
        .iter()
        .filter_map(std::env::var_os)
        .flat_map(|root| {
            let root = PathBuf::from(root);
            [root.join("Google/Chrome/Application/chrome.exe"), root.join("Microsoft/Edge/Application/msedge.exe")]
        })
        .collect()
}

#[cfg(target_os = "macos")]
fn known_paths() -> Vec<PathBuf> {
    vec![PathBuf::from("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"), PathBuf::from("/Applications/Chromium.app/Contents/MacOS/Chromium")]
}

#[cfg(target_os = "linux")]
fn known_paths() -> Vec<PathBuf> {
    ["chromium", "chromium-browser", "google-chrome-stable", "google-chrome", "microsoft-edge"]
        .iter()
        .flat_map(|n| ["/usr/bin", "/usr/local/bin", "/snap/bin"].map(|d| Path::new(d).join(n)))
        .collect()
}

fn find_browser(cfg: &Config) -> Option<PathBuf> {
    cfg.chromium.clone().or_else(|| known_paths().into_iter().find(|p| p.is_file()))
}

fn base_command(exe: &Path, dir: &Path, socks_port: u16, w: u32, h: u32) -> Command {
    let mut c = Command::new(exe);
    c.args([
        "--headless=new",
        "--incognito",
        "--no-first-run",
        "--no-default-browser-check",
        "--disable-background-networking",
        "--disable-breakpad",
        "--disable-component-update",
        "--disable-crash-reporter",
        "--disable-default-apps",
        "--disable-domain-reliability",
        "--disable-extensions",
        "--disable-sync",
        "--metrics-recording-only",
        "--mute-audio",
        "--no-pings",
        "--safebrowsing-disable-auto-update",
        "--force-webrtc-ip-handling-policy=disable_non_proxied_udp",
        "--proxy-bypass-list=<-loopback>",
    ]);
    c.arg(format!("--proxy-server=socks5://127.0.0.1:{socks_port}"));
    c.arg("--host-resolver-rules=MAP * ~NOTFOUND , EXCLUDE 127.0.0.1");
    c.arg(format!("--user-data-dir={}", dir.join("profile").display()));
    c.arg(format!("--disk-cache-dir={}", dir.join("cache").display()));
    c.arg(format!("--window-size={w},{h}"));
    // Everything the browser's libraries put in a temp or config directory stays in the session's.
    for var in ["HOME", "TMPDIR", "TMP", "TEMP", "XDG_CACHE_HOME", "XDG_CONFIG_HOME", "XDG_DATA_HOME"] {
        c.env(var, dir);
    }
    c.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    c
}

type Wire = (mpsc::Sender<Zeroizing<String>>, mpsc::Receiver<Zeroizing<String>>);

/// POSIX: DevTools over a private pipe (fds 3 and 4), so no other process on the node can reach
/// the session's browser. The sandbox stays on, hence no running as root.
#[cfg(unix)]
fn spawn(mut c: Command, _dir: &Path) -> Result<(Child, Wire), &'static str> {
    use std::{
        fs::File,
        io::{Read, Write},
        os::{
            fd::{FromRawFd, RawFd},
            unix::process::CommandExt,
        },
    };
    fn pipe_cloexec() -> Option<[RawFd; 2]> {
        let mut fds = [0; 2];
        if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
            return None;
        }
        for fd in fds {
            unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
        }
        Some(fds)
    }
    if unsafe { libc::geteuid() } == 0 {
        eprintln!("rbi: refusing to start the browser as root (its sandbox would have to be off)");
        return Err("remote_err_rbi");
    }
    let to_browser = pipe_cloexec().ok_or("remote_err_rbi")?;
    let from_browser = pipe_cloexec().ok_or("remote_err_rbi")?;
    // Above the target numbers, so the `dup2`s below cannot clobber each other.
    let child_read = unsafe { libc::fcntl(to_browser[0], libc::F_DUPFD_CLOEXEC, 10) };
    let child_write = unsafe { libc::fcntl(from_browser[1], libc::F_DUPFD_CLOEXEC, 10) };
    unsafe {
        libc::close(to_browser[0]);
        libc::close(from_browser[1]);
    }
    if child_read < 0 || child_write < 0 {
        return Err("remote_err_rbi");
    }
    c.arg("--remote-debugging-pipe").arg("about:blank").process_group(0);
    // SAFETY: only async-signal-safe calls between fork and exec.
    unsafe {
        c.pre_exec(move || {
            if libc::dup2(child_read, 3) < 0 || libc::dup2(child_write, 4) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            #[cfg(target_os = "linux")]
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
            Ok(())
        });
    }
    let spawned = c.spawn();
    unsafe {
        libc::close(child_read);
        libc::close(child_write);
    }
    let child = spawned.map_err(|_| "remote_err_rbi")?;

    // SAFETY: the parent's pipe ends, owned by nothing else.
    let mut w = unsafe { File::from_raw_fd(to_browser[1]) };
    let mut r = unsafe { File::from_raw_fd(from_browser[0]) };
    let (out_tx, mut out_rx) = mpsc::channel::<Zeroizing<String>>(64);
    let (in_tx, in_rx) = mpsc::channel::<Zeroizing<String>>(256);
    std::thread::spawn(move || {
        while let Some(m) = out_rx.blocking_recv() {
            if w.write_all(m.as_bytes()).is_err() || w.write_all(&[0]).is_err() {
                return;
            }
        }
    });
    std::thread::spawn(move || {
        let mut pending = Zeroizing::new(Vec::new());
        let mut buf = Zeroizing::new(vec![0u8; 64 * 1024]);
        loop {
            match r.read(&mut buf) {
                Ok(n) if n > 0 => pending.extend_from_slice(&buf[..n]),
                _ => return,
            }
            while let Some(end) = pending.iter().position(|b| *b == 0) {
                let text = String::from_utf8_lossy(&pending[..end]).into_owned();
                pending.drain(..=end);
                if in_tx.blocking_send(Zeroizing::new(text)).is_err() {
                    return;
                }
            }
        }
    });
    Ok((child, (out_tx, in_rx)))
}

/// Windows has no inheritable numbered descriptors for `--remote-debugging-pipe`: DevTools on a
/// loopback port chosen by the browser.
#[cfg(windows)]
fn spawn(mut c: Command, dir: &Path) -> Result<(Child, Wire), &'static str> {
    c.arg("--remote-debugging-port=0").arg("about:blank");
    let child = c.spawn().map_err(|_| "remote_err_rbi")?;
    let (out_tx, out_rx) = mpsc::channel::<Zeroizing<String>>(64);
    let (in_tx, in_rx) = mpsc::channel::<Zeroizing<String>>(256);
    let file = dir.join("profile").join("DevToolsActivePort");
    tokio::spawn(pump_websocket(file, out_rx, in_tx));
    Ok((child, (out_tx, in_rx)))
}

#[cfg(windows)]
async fn pump_websocket(file: PathBuf, mut out_rx: mpsc::Receiver<Zeroizing<String>>, in_tx: mpsc::Sender<Zeroizing<String>>) {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::{connect_async_with_config, tungstenite::{Message, protocol::WebSocketConfig}};
    let start = std::time::Instant::now();
    let url = loop {
        if let Ok(s) = std::fs::read_to_string(&file)
            && let [port, path, ..] = s.lines().collect::<Vec<_>>()[..]
        {
            break format!("ws://127.0.0.1:{port}{path}");
        }
        if start.elapsed() > START_TIMEOUT {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    let cfg = WebSocketConfig::default().max_message_size(Some(256 * 1024 * 1024)).max_frame_size(None);
    let Ok((ws, _)) = connect_async_with_config(url, Some(cfg), true).await else { return };
    let (mut sink, mut stream) = ws.split();
    loop {
        tokio::select! {
            m = out_rx.recv() => {
                let Some(m) = m else { return };
                if sink.send(Message::text(m.as_str())).await.is_err() {
                    return;
                }
            }
            m = stream.next() => match m {
                Some(Ok(Message::Text(t))) => {
                    if in_tx.send(Zeroizing::new(t.to_string())).await.is_err() {
                        return;
                    }
                }
                Some(Ok(_)) => {}
                _ => return,
            }
        }
    }
}

struct Cdp {
    out: mpsc::Sender<Zeroizing<String>>,
    input: mpsc::Receiver<Zeroizing<String>>,
    next: u64,
    session: Option<String>,
}

impl Cdp {
    async fn send_to(&mut self, session: Option<String>, method: &str, params: Value) -> Result<u64, &'static str> {
        self.next += 1;
        let mut msg = json!({ "id": self.next, "method": method, "params": params });
        if let Some(s) = session {
            msg["sessionId"] = json!(s);
        }
        self.out.send(Zeroizing::new(msg.to_string())).await.map_err(|_| "remote_err_rbi")?;
        Ok(self.next)
    }

    async fn page(&mut self, method: &str, params: Value) -> Result<(), &'static str> {
        let s = self.session.clone();
        self.send_to(s, method, params).await.map(|_| ())
    }

    async fn browser(&mut self, method: &str, params: Value) -> Result<(), &'static str> {
        self.send_to(None, method, params).await.map(|_| ())
    }

    /// Setup only: waits for the answer, events before it are not needed.
    async fn call(&mut self, session: Option<String>, method: &str, params: Value, limit: Duration) -> Result<Value, &'static str> {
        let id = self.send_to(session, method, params).await?;
        let wait = async {
            while let Some(line) = self.input.recv().await {
                let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
                if v["id"].as_u64() == Some(id) {
                    return if v.get("error").is_some() { None } else { Some(v["result"].clone()) };
                }
            }
            None
        };
        tokio::time::timeout(limit, wait).await.ok().flatten().ok_or("remote_err_rbi")
    }
}

fn checked_url(url: &str) -> Result<String, &'static str> {
    let u = Url::parse(url).map_err(|_| "remote_err_blocked")?;
    policy::check_url(&u)?;
    Ok(u.to_string())
}

fn clamp_size(w: u32, h: u32) -> (u32, u32) {
    (w.clamp(64, MAX_WIDTH), h.clamp(64, MAX_HEIGHT))
}

async fn metrics(cdp: &mut Cdp, w: u32, h: u32) -> Result<(), &'static str> {
    cdp.page("Emulation.setDeviceMetricsOverride", json!({ "width": w, "height": h, "deviceScaleFactor": 1, "mobile": false })).await
}

async fn screencast(cdp: &mut Cdp, w: u32, h: u32) -> Result<(), &'static str> {
    cdp.page("Page.startScreencast", json!({ "format": "jpeg", "quality": JPEG_QUALITY, "maxWidth": w, "maxHeight": h, "everyNthFrame": 1 })).await
}

fn finite(v: f64) -> f64 {
    if v.is_finite() { v.clamp(-1e6, 1e6) } else { 0.0 }
}

fn button_name(b: MouseButton) -> &'static str {
    match b {
        MouseButton::None => "none",
        MouseButton::Left => "left",
        MouseButton::Middle => "middle",
        MouseButton::Right => "right",
    }
}

/// Applies one client message.
async fn command(cdp: &mut Cdp, cmd: RbiCommand, size: &mut (u32, u32), tx: &mut impl EventSink) -> Result<(), &'static str> {
    match cmd {
        RbiCommand::Navigate { url } => match checked_url(&url) {
            Ok(u) => cdp.page("Page.navigate", json!({ "url": u })).await?,
            Err(code) => tx.event(&RbiEvent::Error { code: code.into() }).await?,
        },
        RbiCommand::Back => cdp.page("Runtime.evaluate", json!({ "expression": "history.back()" })).await?,
        RbiCommand::Forward => cdp.page("Runtime.evaluate", json!({ "expression": "history.forward()" })).await?,
        RbiCommand::Reload => cdp.page("Page.reload", json!({})).await?,
        RbiCommand::Stop => cdp.page("Page.stopLoading", json!({})).await?,
        RbiCommand::Resize { width, height } => {
            let next = clamp_size(width, height);
            if next != *size {
                *size = next;
                metrics(cdp, next.0, next.1).await?;
                cdp.page("Page.stopScreencast", json!({})).await?;
                screencast(cdp, next.0, next.1).await?;
            }
        }
        RbiCommand::Visible { on } => {
            if on {
                screencast(cdp, size.0, size.1).await?;
            } else {
                cdp.page("Page.stopScreencast", json!({})).await?;
            }
        }
        RbiCommand::Mouse { kind, x, y, button, buttons, clicks, mods } => {
            let t = match kind {
                MouseKind::Move => "mouseMoved",
                MouseKind::Down => "mousePressed",
                MouseKind::Up => "mouseReleased",
            };
            let params = json!({ "type": t, "x": finite(x), "y": finite(y), "button": button_name(button), "buttons": buttons & 31, "clickCount": clicks.min(3), "modifiers": mods & 15 });
            cdp.page("Input.dispatchMouseEvent", params).await?;
        }
        RbiCommand::Wheel { x, y, dx, dy, mods } => {
            let params = json!({ "type": "mouseWheel", "x": finite(x), "y": finite(y), "deltaX": finite(dx), "deltaY": finite(dy), "modifiers": mods & 15 });
            cdp.page("Input.dispatchMouseEvent", params).await?;
        }
        RbiCommand::Key { kind, key, code, text, vk, mods } => {
            if key.len() > MAX_KEY_FIELD || code.len() > MAX_KEY_FIELD || text.len() > MAX_KEY_FIELD {
                return Ok(());
            }
            let t = match (kind, text.is_empty()) {
                (KeyKind::Up, _) => "keyUp",
                (KeyKind::Down, true) => "rawKeyDown",
                (KeyKind::Down, false) => "keyDown",
            };
            let params = json!({ "type": t, "key": key, "code": code, "text": text, "unmodifiedText": text, "windowsVirtualKeyCode": vk & 255, "nativeVirtualKeyCode": vk & 255, "modifiers": mods & 15 });
            cdp.page("Input.dispatchKeyEvent", params).await?;
        }
    }
    Ok(())
}

trait EventSink {
    async fn event(&mut self, e: &RbiEvent) -> Result<(), &'static str>;
    async fn frame(&mut self, data: &[u8]) -> Result<(), &'static str>;
}

impl<S: AsyncRead + AsyncWrite + Unpin> EventSink for Tx<S> {
    async fn event(&mut self, e: &RbiEvent) -> Result<(), &'static str> {
        self.send_json(e).await.map_err(|_| "gone")
    }

    async fn frame(&mut self, data: &[u8]) -> Result<(), &'static str> {
        self.send(Kind::Binary, data).await.map_err(|_| "gone")
    }
}

fn shown_url(frame: &Value) -> String {
    format!("{}{}", frame["url"].as_str().unwrap_or(""), frame["urlFragment"].as_str().unwrap_or(""))
}

async fn committed(tx: &mut impl EventSink, url: String) -> Result<(), &'static str> {
    if url.starts_with("chrome-error:") {
        return tx.event(&RbiEvent::Error { code: "remote_err_fetch".into() }).await;
    }
    if url.starts_with("http://") || url.starts_with("https://") {
        return tx.event(&RbiEvent::Committed { url }).await;
    }
    Ok(())
}

async fn ask_title(cdp: &mut Cdp) -> Result<u64, &'static str> {
    let session = cdp.session.clone();
    cdp.send_to(session, "Runtime.evaluate", json!({ "expression": "document.title", "returnByValue": true })).await
}

/// One DevTools message. `Ok(false)` ends the session.
async fn browser_event(cdp: &mut Cdp, line: &str, main: &str, title_query: &mut u64, tx: &mut impl EventSink) -> Result<bool, &'static str> {
    let Ok(v) = serde_json::from_str::<Value>(line) else { return Ok(true) };
    if *title_query != 0 && v["id"].as_u64() == Some(*title_query) {
        let title = v["result"]["result"]["value"].as_str().unwrap_or("").to_string();
        tx.event(&RbiEvent::Title { title }).await?;
        return Ok(true);
    }
    let Some(method) = v["method"].as_str() else { return Ok(true) };
    let p = &v["params"];
    match method {
        "Page.screencastFrame" => {
            let Some(id) = p["sessionId"].as_u64() else { return Ok(true) };
            if let Some(data) = p["data"].as_str().and_then(|d| STANDARD.decode(d).ok()) {
                let data = Zeroizing::new(data);
                tx.frame(&data).await?;
            }
            // The next frame is produced after this one was handed to the client (back-pressure).
            cdp.page("Page.screencastFrameAck", json!({ "sessionId": id })).await?;
        }
        "Page.frameNavigated" if p["frame"].get("parentId").is_none() => committed(tx, shown_url(&p["frame"])).await?,
        "Page.navigatedWithinDocument" if p["frameId"].as_str() == Some(main) => {
            committed(tx, p["url"].as_str().unwrap_or("").to_string()).await?;
            *title_query = ask_title(cdp).await?;
        }
        "Page.frameStartedLoading" if p["frameId"].as_str() == Some(main) => tx.event(&RbiEvent::Loading { on: true }).await?,
        "Page.frameStoppedLoading" if p["frameId"].as_str() == Some(main) => {
            tx.event(&RbiEvent::Loading { on: false }).await?;
            *title_query = ask_title(cdp).await?;
        }
        "Page.windowOpen" => {
            if let Ok(u) = checked_url(p["url"].as_str().unwrap_or("")) {
                cdp.page("Page.navigate", json!({ "url": u })).await?;
            }
        }
        // One tab per session: pop-ups are closed, `Page.windowOpen` moves the tab instead.
        "Target.targetCreated" if p["targetInfo"]["type"] == "page" && p["targetInfo"]["targetId"].as_str() != Some(main) => {
            let id = p["targetInfo"]["targetId"].clone();
            cdp.browser("Target.closeTarget", json!({ "targetId": id })).await?;
        }
        "Target.targetCrashed" | "Inspector.targetCrashed" | "Target.detachedFromTarget" => {
            tx.event(&RbiEvent::Error { code: "remote_err_rbi".into() }).await?;
            return Ok(false);
        }
        _ => {}
    }
    Ok(true)
}

async fn handle_client(cdp: &mut Cdp, f: &Frame, size: &mut (u32, u32), tx: &mut impl EventSink) -> Result<(), &'static str> {
    if f.kind != Kind::Json {
        return Ok(());
    }
    match serde_json::from_slice::<RbiCommand>(&f.data) {
        Ok(cmd) => command(cdp, cmd, size, tx).await,
        Err(_) => Ok(()),
    }
}

/// `Err`: refused before the session began (the caller reports the code). Once `Reply::Ok` is out
/// all errors end the session through the event stream or by closing.
pub async fn run<S>(tx: &mut Tx<S>, mut rx: Rx<S>, url: &str, w: u32, h: u32, cfg: &Config) -> Result<(), &'static str>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let start = checked_url(url)?;
    let _slot = cfg.rbi_slots.try_acquire().map_err(|_| "remote_err_busy")?;
    let exe = find_browser(cfg).ok_or_else(|| {
        eprintln!("rbi: no Chromium-based browser found (use --chromium <PATH>)");
        "remote_err_rbi"
    })?;
    let dir = RamDir::create(cfg.ram_only)?;
    let (w, h) = clamp_size(w, h);
    let command_line = base_command(&exe, dir.path(), cfg.socks_port, w, h);
    let (child, (out, input)) = spawn(command_line, dir.path())?;
    let browser = Browser { child, dir: Some(dir) };
    let mut cdp = Cdp { out, input, next: 0, session: None };

    let served = session(tx, &mut rx, &mut cdp, &start, (w, h)).await;
    // The wipe waits for the process tree to die and may retry: off the async workers.
    let _ = tokio::task::spawn_blocking(move || drop(browser)).await;
    served
}

async fn session<S>(tx: &mut Tx<S>, rx: &mut Rx<S>, cdp: &mut Cdp, start: &str, mut size: (u32, u32)) -> Result<(), &'static str>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    // Air gap: nothing the page offers for download is ever written, not even on the node.
    cdp.call(None, "Browser.setDownloadBehavior", json!({ "behavior": "deny" }), START_TIMEOUT).await?;
    cdp.call(None, "Target.setDiscoverTargets", json!({ "discover": true }), CALL_TIMEOUT).await?;
    let created = cdp.call(None, "Target.createTarget", json!({ "url": "about:blank" }), CALL_TIMEOUT).await?;
    let main = created["targetId"].as_str().ok_or("remote_err_rbi")?.to_string();
    let attached = cdp.call(None, "Target.attachToTarget", json!({ "targetId": main, "flatten": true }), CALL_TIMEOUT).await?;
    cdp.session = Some(attached["sessionId"].as_str().ok_or("remote_err_rbi")?.to_string());
    let s = cdp.session.clone();
    cdp.call(s, "Page.enable", json!({}), CALL_TIMEOUT).await?;
    metrics(cdp, size.0, size.1).await?;
    screencast(cdp, size.0, size.1).await?;
    tx.send_json(&Reply::Ok).await.map_err(|_| "gone")?;
    cdp.page("Page.navigate", json!({ "url": start })).await?;

    let mut title_query = 0;
    loop {
        tokio::select! {
            f = rx.recv() => {
                let Some(f) = f else { break };
                if handle_client(cdp, &f, &mut size, tx).await.is_err() {
                    break;
                }
            }
            line = cdp.input.recv() => {
                let Some(line) = line else {
                    let _ = tx.event(&RbiEvent::Error { code: "remote_err_rbi".into() }).await;
                    break;
                };
                if !matches!(browser_event(cdp, &line, &main, &mut title_query, tx).await, Ok(true)) {
                    break;
                }
            }
        }
    }
    tx.close().await;
    Ok(())
}
