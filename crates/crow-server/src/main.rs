//! `crow-server`: remote execution node of CrowBrowser. Keeps no logs of addresses or pages.

mod eco;
mod policy;
mod ram;
mod rbi;
mod tunnel;

use crow_proto::{Open, Reply, Rx, Tx, accept, derive_psk, parse_token};
use mimalloc::MiMalloc;
use std::{
    alloc::{GlobalAlloc, Layout},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::Semaphore,
};
use zeroize::{Zeroize, Zeroizing};

/// Every heap block is overwritten when freed, whichever library owned it: pages, DOM trees,
/// TLS and socket buffers of a session do not outlive it in RAM.
struct WipingAlloc;

// SAFETY: forwards to mimalloc with the caller's layout; `dealloc` only writes inside the block
// it is about to free.
unsafe impl GlobalAlloc for WipingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        unsafe { MiMalloc.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        unsafe { MiMalloc.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe {
            std::slice::from_raw_parts_mut(ptr, layout.size()).zeroize();
            MiMalloc.dealloc(ptr, layout);
        }
    }
}

#[global_allocator]
static GLOBAL: WipingAlloc = WipingAlloc;

const MAX_CONNECTIONS: usize = 2048;
const MAX_RBI: usize = 8;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

const USAGE: &str = "usage: crow-server --bind <ADDR:PORT> --token <SECRET|argon2:HEX> [--mode eco|rbi|tunnel|all] [--ram-only] [--chromium <PATH>]
       crow-server --print-token-hash <SECRET>
The token may also come from the CROW_TOKEN environment variable.";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Eco,
    Rbi,
    Tunnel,
    All,
}

pub struct Config {
    psk: Zeroizing<[u8; 32]>,
    mode: Mode,
    pub ram_only: bool,
    pub chromium: Option<PathBuf>,
    pub socks_port: u16,
    pub rbi_slots: Semaphore,
    pub http: reqwest::Client,
}

impl Config {
    fn serves(&self, m: Mode) -> bool {
        self.mode == Mode::All || self.mode == m
    }
}

struct Args {
    bind: String,
    token: Zeroizing<String>,
    mode: Mode,
    ram_only: bool,
    chromium: Option<PathBuf>,
}

fn parse_args() -> Result<Args, String> {
    let mut it = std::env::args().skip(1);
    let (mut bind, mut token, mut mode, mut ram_only, mut chromium) = (None, None, Mode::All, false, None);
    while let Some(a) = it.next() {
        let mut value = || it.next().ok_or_else(|| format!("{a} needs a value"));
        match a.as_str() {
            "--bind" => bind = Some(value()?),
            "--token" => token = Some(Zeroizing::new(value()?)),
            "--mode" => {
                mode = match value()?.as_str() {
                    "eco" => Mode::Eco,
                    "rbi" => Mode::Rbi,
                    "tunnel" => Mode::Tunnel,
                    "all" => Mode::All,
                    other => return Err(format!("unknown mode: {other}")),
                };
            }
            "--ram-only" => ram_only = true,
            "--chromium" => chromium = Some(PathBuf::from(value()?)),
            "--print-token-hash" => {
                let secret = Zeroizing::new(value()?);
                let hex: String = derive_psk(&secret).iter().map(|b| format!("{b:02x}")).collect();
                println!("argon2:{hex}");
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    let token = token.or_else(|| std::env::var("CROW_TOKEN").ok().map(Zeroizing::new)).ok_or("--token is required")?;
    Ok(Args { bind: bind.ok_or("--bind is required")?, token, mode, ram_only, chromium })
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut term) => {
                tokio::select! {
                    _ = term.recv() => {}
                    _ = tokio::signal::ctrl_c() => {}
                }
            }
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

async fn serve(stream: TcpStream, cfg: Arc<Config>) {
    let _ = stream.set_nodelay(true);
    let Ok(Ok((tx, mut rx))) = tokio::time::timeout(HANDSHAKE_TIMEOUT, accept(stream, &cfg.psk)).await else { return };
    let Ok(Some(open)) = tokio::time::timeout(HANDSHAKE_TIMEOUT, rx.recv_json::<Open>()).await else { return };
    dispatch(open, tx, rx, cfg).await;
}

async fn refuse<S>(mut tx: Tx<S>, code: &str)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let _ = tx.send_json(&Reply::Err { code: code.into() }).await;
}

async fn dispatch<S>(open: Open, mut tx: Tx<S>, rx: Rx<S>, cfg: Arc<Config>)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    match &open {
        Open::Ping => {
            let _ = tx.send_json(&Reply::Ok).await;
        }
        Open::Eco { url } if cfg.serves(Mode::Eco) => eco::run(tx, url, &cfg.http).await,
        Open::Rbi { url, width, height } if cfg.serves(Mode::Rbi) => {
            if let Err(code) = rbi::run(&mut tx, rx, url, *width, *height, &cfg).await {
                refuse(tx, code).await;
            }
        }
        Open::Tunnel { host, port } if cfg.serves(Mode::Tunnel) => tunnel::run(tx, rx, host, *port).await,
        _ => refuse(tx, "remote_err_mode").await,
    }
}

#[tokio::main]
async fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}\n{USAGE}");
            std::process::exit(2);
        }
    };
    let fail = |msg: String| -> ! {
        eprintln!("crow-server: {msg}");
        std::process::exit(1);
    };
    let token = args.token;
    let psk = match tokio::task::spawn_blocking(move || parse_token(&token)).await {
        Ok(Ok(k)) => k,
        Ok(Err(e)) => fail(e.to_string()),
        Err(e) => fail(e.to_string()),
    };

    ram::sweep_stale();
    if args.ram_only {
        ram::harden();
        if let (true, Some(problem)) = (args.mode == Mode::Rbi || args.mode == Mode::All, ram::ram_only_problem()) {
            if args.mode == Mode::Rbi {
                fail(format!("--ram-only: {problem}"));
            }
            eprintln!("warning: --ram-only: RBI sessions will be refused: {problem}");
        }
    }

    let http = match eco::client() {
        Ok(c) => c,
        Err(e) => fail(e),
    };
    let socks_port = match tunnel::local_socks().await {
        Ok(p) => p,
        Err(e) => fail(e.to_string()),
    };
    let listener = match TcpListener::bind(&args.bind).await {
        Ok(l) => l,
        Err(e) => fail(format!("{}: {e}", args.bind)),
    };
    let cfg = Arc::new(Config {
        psk,
        mode: args.mode,
        ram_only: args.ram_only,
        chromium: args.chromium,
        socks_port,
        rbi_slots: Semaphore::new(MAX_RBI),
        http,
    });
    let slots = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    eprintln!("crow-server listening on {}", args.bind);

    let shutdown = shutdown_signal();
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            () = &mut shutdown => break,
            r = listener.accept() => {
                let Ok((stream, _)) = r else { continue };
                let Ok(permit) = slots.clone().try_acquire_owned() else { continue };
                let cfg = cfg.clone();
                tokio::spawn(async move {
                    serve(stream, cfg).await;
                    drop(permit);
                });
            }
        }
    }
    // Returning drops the runtime and with it every session task: their guards kill the browsers
    // and wipe the session directories.
}
