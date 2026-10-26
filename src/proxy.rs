//! Network route of web content: direct, the system proxy, a custom SOCKS5 / HTTP proxy (with
//! optional login) or a local Tor client. Lives in RAM only (never in the vault or on disk):
//! every start is direct; credentials are zeroized when a mode is replaced or dropped.
//! Applied per engine by `wk::set_proxy`; the browser UI never uses the network.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fmt::Write as _,
    io::Write as _,
    process::{Command, Stdio},
};
use zeroize::{Zeroize, Zeroizing};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProxyProtocol {
    Socks5,
    Http,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub enum ProxyMode {
    Direct,
    System,
    Custom {
        protocol: ProxyProtocol,
        host: String,
        port: u16,
        auth: Option<(String, String)>,
    },
    /// Local Tor client: SOCKS5 on 127.0.0.1:9050.
    Tor,
}

/// Debug output (logs, panics) never shows the password.
impl std::fmt::Debug for ProxyMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Custom { protocol, host, port, auth } => f
                .debug_struct("Custom")
                .field("protocol", protocol)
                .field("host", host)
                .field("port", port)
                .field("auth", &auth.as_ref().map(|(u, _)| (u, "***")))
                .finish(),
            Self::Direct => f.write_str("Direct"),
            Self::System => f.write_str("System"),
            Self::Tor => f.write_str("Tor"),
        }
    }
}

pub const TOR_HOST: &str = "127.0.0.1";
pub const TOR_PORT: u16 = 9050;
/// SOCKS5 limits user name and password to 255 bytes each (RFC 1929).
const MAX_AUTH: usize = 255;

/// Credentials and the proxy host leave no copy in freed memory.
impl Drop for ProxyMode {
    fn drop(&mut self) {
        if let Self::Custom { host, auth, .. } = self {
            host.zeroize();
            if let Some((u, p)) = auth {
                u.zeroize();
                p.zeroize();
            }
        }
    }
}

/// Proxy endpoint as the engines need it.
pub struct Endpoint<'a> {
    pub protocol: ProxyProtocol,
    pub host: &'a str,
    pub port: u16,
    pub auth: Option<(&'a str, &'a str)>,
}

/// `set-proxy-mode` request of the settings panel.
#[derive(Deserialize)]
struct Req {
    mode: String,
    #[serde(default)]
    protocol: String,
    #[serde(default)]
    host: String,
    #[serde(default)]
    port: u32,
    #[serde(default)]
    user: String,
    #[serde(default)]
    pass: String,
    /// Password field left empty: keep the current one (the panel never receives it).
    #[serde(default)]
    keep_pass: bool,
}

impl Drop for Req {
    fn drop(&mut self) {
        self.host.zeroize();
        self.user.zeroize();
        self.pass.zeroize();
    }
}

/// Host name, IPv4 or bracket-less IPv6 literal; nothing that could change the meaning of a
/// proxy URI or a browser command line (spaces, quotes, `@`, `/`, `,`, `;`).
fn valid_host(h: &str) -> bool {
    !h.is_empty()
        && h.len() <= 253
        && !h.starts_with(['.', '-'])
        && h.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b':'))
}

fn valid_auth(s: &str) -> bool {
    s.len() <= MAX_AUTH && !s.chars().any(char::is_control)
}

/// RFC 3986 userinfo / host component encoding.
fn pct(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            o.push(b as char);
        } else {
            let _ = write!(o, "%{b:02X}");
        }
    }
    o
}

impl ProxyMode {
    /// Parses and validates the panel's JSON request; errors are `locales.json` keys. `current`:
    /// the active mode, whose password a request with `keep_pass` and the same user reuses.
    pub fn from_request(current: &Self, json: &str) -> Result<Self, &'static str> {
        let r: Req = serde_json::from_str(json).map_err(|_| "proxy_err_invalid")?;
        Ok(match r.mode.as_str() {
            "direct" => Self::Direct,
            "system" => Self::System,
            "tor" => Self::Tor,
            "custom" => {
                let protocol = match r.protocol.as_str() {
                    "socks5" => ProxyProtocol::Socks5,
                    "http" => ProxyProtocol::Http,
                    _ => return Err("proxy_err_invalid"),
                };
                let host = r.host.trim().trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase();
                if !valid_host(&host) {
                    return Err("proxy_err_host");
                }
                let port = u16::try_from(r.port).ok().filter(|p| *p > 0).ok_or("proxy_err_port")?;
                let kept = match current {
                    Self::Custom { auth: Some((u, p)), .. } if r.keep_pass && r.pass.is_empty() && *u == r.user => Some(p.clone()),
                    _ => None,
                };
                let pass = kept.as_ref().unwrap_or(&r.pass);
                let auth = if r.user.is_empty() && pass.is_empty() {
                    None
                } else if valid_auth(&r.user) && valid_auth(pass) && !r.user.is_empty() {
                    Some((r.user.clone(), pass.clone()))
                } else {
                    return Err("proxy_err_auth");
                };
                Self::Custom { protocol, host, port, auth }
            }
            _ => return Err("proxy_err_invalid"),
        })
    }

    /// Web content goes through an explicit proxy (Custom or Tor): WebRTC is cut off and DNS is
    /// resolved by the proxy.
    pub fn proxied(&self) -> bool {
        matches!(self, Self::Custom { .. } | Self::Tor)
    }

    pub fn endpoint(&self) -> Option<Endpoint<'_>> {
        match self {
            Self::Custom { protocol, host, port, auth } => Some(Endpoint {
                protocol: *protocol,
                host,
                port: *port,
                auth: auth.as_ref().map(|(u, p)| (u.as_str(), p.as_str())),
            }),
            Self::Tor => Some(Endpoint { protocol: ProxyProtocol::Socks5, host: TOR_HOST, port: TOR_PORT, auth: None }),
            Self::Direct | Self::System => None,
        }
    }

    /// Panel state; the password itself never goes back to the UI.
    pub fn state(&self) -> Value {
        let mode = match self {
            Self::Direct => "direct",
            Self::System => "system",
            Self::Custom { .. } => "custom",
            Self::Tor => "tor",
        };
        match self {
            Self::Custom { protocol, host, port, auth } => json!({
                "mode": mode,
                "protocol": if *protocol == ProxyProtocol::Socks5 { "socks5" } else { "http" },
                "host": host, "port": port,
                "user": auth.as_ref().map_or("", |(u, _)| u.as_str()),
                "hasPass": auth.as_ref().is_some_and(|(_, p)| !p.is_empty()),
            }),
            _ => json!({ "mode": mode }),
        }
    }
}

impl Endpoint<'_> {
    /// `host:port`, IPv6 literals in brackets.
    pub fn authority(&self) -> String {
        if self.host.contains(':') { format!("[{}]:{}", self.host, self.port) } else { format!("{}:{}", self.host, self.port) }
    }

    /// Proxy URI; `socks5h` = host names resolved by the proxy (curl's spelling; the engines'
    /// SOCKS5 implementations always send host names to the proxy).
    pub fn uri(&self, scheme_socks: &str, with_auth: bool) -> Zeroizing<String> {
        let scheme = if self.protocol == ProxyProtocol::Socks5 { scheme_socks } else { "http" };
        let auth = match self.auth.filter(|_| with_auth) {
            Some((u, p)) => format!("{}:{}@", pct(u), pct(p)),
            None => String::new(),
        };
        Zeroizing::new(format!("{scheme}://{auth}{}", self.authority()))
    }
}

/// Result of `probe`: exit address and whether it is a Tor exit.
pub struct Probe {
    pub ip: String,
    pub tor: bool,
}

/// Connectivity test through `mode`, outside the browser: a separate curl process without
/// cookies, cache, referrer or browser headers, so the probe carries nothing of the session.
/// Asks the Tor Project's check service for the exit address. Credentials go to curl on stdin
/// (never on the command line, visible to other processes). Blocking: run off the main thread.
pub fn probe(mode: &ProxyMode) -> Result<Probe, &'static str> {
    let mut c = crate::update::curl();
    c.args(["--max-time", "25", "-H", "Accept: application/json"]);
    // `update::curl` bypasses environment proxies; set the route explicitly.
    let mut config = Zeroizing::new(String::new());
    match (mode, mode.endpoint()) {
        (_, Some(e)) => {
            c.arg("--proxy").arg(e.uri("socks5h", false).as_str()).args(["--noproxy", ""]);
            if let Some((u, p)) = e.auth {
                let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
                let (u, p) = (Zeroizing::new(esc(u)), Zeroizing::new(esc(p)));
                // Written in place: no unwiped temporary copy of the login.
                let _ = writeln!(*config, "proxy-user = \"{}:{}\"", u.as_str(), p.as_str());
            }
        }
        (ProxyMode::System, None) => {
            if let Some(p) = system_proxy() {
                c.arg("--proxy").arg(p).args(["--noproxy", ""]);
            }
        }
        _ => {}
    }
    c.args(["-K", "-", "https://check.torproject.org/api/ip"]);
    c.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    let mut child = c.spawn().map_err(|_| "update_err_curl")?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(config.as_bytes());
    }
    let out = child.wait_with_output().map_err(|_| "update_err_curl")?;
    if !out.status.success() {
        return Err("proxy_test_failed");
    }
    let v: Value = serde_json::from_slice(&out.stdout).map_err(|_| "proxy_test_failed")?;
    let ip = v["IP"].as_str().filter(|s| s.len() <= 64).ok_or("proxy_test_failed")?.to_string();
    Ok(Probe { ip, tor: v["IsTor"].as_bool().unwrap_or(false) })
}

/// The system's HTTP proxy for the probe (the engines read it themselves).
#[cfg(target_os = "windows")]
fn system_proxy() -> Option<String> {
    let out = Command::new("reg")
        .args(["query", r"HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let value = |name: &str| {
        text.lines().find_map(|l| {
            let mut it = l.split_whitespace();
            (it.next() == Some(name)).then(|| it.nth(1).map(str::to_string)).flatten()
        })
    };
    if value("ProxyEnable").as_deref() != Some("0x1") {
        return None;
    }
    let server = value("ProxyServer")?;
    // "host:port" or "http=host:port;https=host:port".
    let https = server.split(';').find_map(|p| p.strip_prefix("https=")).unwrap_or(&server);
    valid_host(https.rsplit_once(':').map_or(https, |(h, _)| h)).then(|| format!("http://{https}"))
}

#[cfg(target_os = "macos")]
fn system_proxy() -> Option<String> {
    let out = Command::new("scutil").arg("--proxy").stdin(Stdio::null()).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let value = |k: &str| {
        text.lines().find_map(|l| l.trim().strip_prefix(k)?.trim().strip_prefix(':').map(|v| v.trim().to_string()))
    };
    if value("HTTPSEnable").as_deref() != Some("1") {
        return None;
    }
    let (host, port) = (value("HTTPSProxy")?, value("HTTPSPort")?);
    (valid_host(&host) && port.parse::<u16>().is_ok()).then(|| format!("http://{host}:{port}"))
}

/// GNOME's proxy settings (other desktops: environment, which curl then reads itself).
#[cfg(target_os = "linux")]
fn system_proxy() -> Option<String> {
    let get = |k: &str, schema: &str| {
        let out = Command::new("gsettings").args(["get", schema, k]).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
        Some(String::from_utf8_lossy(&out.stdout).trim().trim_matches('\'').to_string())
    };
    if get("mode", "org.gnome.system.proxy")? != "manual" {
        return std::env::var("https_proxy").or_else(|_| std::env::var("HTTPS_PROXY")).ok();
    }
    let (host, port) = (get("host", "org.gnome.system.proxy.https")?, get("port", "org.gnome.system.proxy.https")?);
    (valid_host(&host) && port.parse::<u16>().is_ok_and(|p| p > 0)).then(|| format!("http://{host}:{port}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_request_is_validated() {
        let m = ProxyMode::from_request(&ProxyMode::Direct, r#"{"mode":"custom","protocol":"socks5","host":"Proxy.Example","port":1080}"#);
        assert_eq!(
            m,
            Ok(ProxyMode::Custom { protocol: ProxyProtocol::Socks5, host: "proxy.example".into(), port: 1080, auth: None })
        );
        for bad in ["a b", "h\"x", "x@y", "x/y", "x,y", ""] {
            let j = json!({ "mode": "custom", "protocol": "http", "host": bad, "port": 8080 }).to_string();
            assert_eq!(ProxyMode::from_request(&ProxyMode::Direct, &j), Err("proxy_err_host"));
        }
        assert_eq!(ProxyMode::from_request(&ProxyMode::Direct, r#"{"mode":"custom","protocol":"http","host":"h","port":0}"#), Err("proxy_err_port"));
        assert_eq!(ProxyMode::from_request(&ProxyMode::Direct, r#"{"mode":"custom","protocol":"http","host":"h","port":70000}"#), Err("proxy_err_port"));
    }

    #[test]
    fn uris_encode_credentials_and_resolve_remotely() {
        let m = ProxyMode::Custom {
            protocol: ProxyProtocol::Socks5,
            host: "::1".into(),
            port: 1080,
            auth: Some(("me".into(), "p@ss:/".into())),
        };
        let e = m.endpoint().unwrap();
        assert_eq!(e.uri("socks5h", true).as_str(), "socks5h://me:p%40ss%3A%2F@[::1]:1080");
        assert_eq!(e.uri("socks5", false).as_str(), "socks5://[::1]:1080");
        let tor = ProxyMode::Tor;
        assert_eq!(tor.endpoint().unwrap().uri("socks5h", true).as_str(), "socks5h://127.0.0.1:9050");
        assert!(tor.proxied() && !ProxyMode::System.proxied());
    }

    #[test]
    fn state_never_contains_the_password() {
        let m = ProxyMode::Custom {
            protocol: ProxyProtocol::Http,
            host: "h".into(),
            port: 3128,
            auth: Some(("u".into(), "secret".into())),
        };
        let s = m.state().to_string();
        assert!(!s.contains("secret") && s.contains("\"hasPass\":true"));
    }
}
