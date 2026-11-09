//! What the node may reach, enforced for all three engines: no internal addresses (the node must
//! not become a door into its own network), no tracker hosts, web ports only.

use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use std::{
    io::{self, ErrorKind},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    time::Duration,
};
use tokio::net::{TcpStream, lookup_host};
use url::{Host, Url};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Ad and telemetry domains, matched with their subdomains.
const TRACKERS: &[&str] = &[
    "2mdn.net", "adnxs.com", "adroll.com", "adsafeprotected.com", "adservice.google.com", "adsrvr.org",
    "amazon-adsystem.com", "appsflyer.com", "bluekai.com", "branch.io", "casalemedia.com", "chartbeat.com",
    "chartbeat.net", "clarity.ms", "criteo.com", "criteo.net", "demdex.net", "doubleclick.net", "everesttech.net",
    "facebook.net", "fullstory.com", "google-analytics.com", "googleadservices.com", "googlesyndication.com",
    "googletagmanager.com", "googletagservices.com", "hotjar.com", "krxd.net", "mixpanel.com", "moatads.com",
    "mouseflow.com", "nr-data.net", "omtrdc.net", "openx.net", "outbrain.com", "pubmatic.com", "quantserve.com",
    "rlcdn.com", "rubiconproject.com", "scorecardresearch.com", "segment.io", "taboola.com", "tiqcdn.com",
    "yieldmo.com", "zedo.com",
];

fn v4_allowed(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_documentation()
        || ip.is_multicast()
        || a == 0
        || a >= 240
        || (a == 100 && (64..128).contains(&b))
        || (a == 192 && b == 0 && c == 0)
        || (a == 198 && (b == 18 || b == 19)))
}

fn v6_allowed(ip: Ipv6Addr) -> bool {
    let s = ip.segments();
    let embedded = |hi: u16, lo: u16| v4_allowed(Ipv4Addr::from((u32::from(hi) << 16) | u32::from(lo)));
    if let Some(v4) = ip.to_ipv4_mapped() {
        return v4_allowed(v4);
    }
    match s[0] {
        // ::/96 (IPv4-compatible, loopback, unspecified) and the rest of 0000::/16.
        0 => false,
        0x64 if s[1] == 0xff9b => embedded(s[6], s[7]),
        0x2002 => embedded(s[1], s[2]),
        0x2001 if s[1] == 0xdb8 => false,
        x if x & 0xfe00 == 0xfc00 || x & 0xffc0 == 0xfe80 || x & 0xff00 == 0xff00 => false,
        _ => true,
    }
}

pub fn ip_allowed(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => v4_allowed(v),
        IpAddr::V6(v) => v6_allowed(v),
    }
}

/// Web ports only: the node is no mail or SSH relay.
pub fn port_allowed(port: u16) -> bool {
    port == 80 || port == 443 || port >= 1024
}

pub fn tracker(host: &str) -> bool {
    let h = host.trim_end_matches('.').to_ascii_lowercase();
    TRACKERS.iter().any(|d| h == *d || h.strip_suffix(d).is_some_and(|r| r.ends_with('.')))
}

fn internal_name(host: &str) -> bool {
    let h = host.trim_end_matches('.').to_ascii_lowercase();
    !h.contains('.') || [".localhost", ".local", ".internal", ".intranet", ".lan"].iter().any(|s| h.ends_with(s))
}

/// http(s) address the node may fetch; DNS answers are checked separately (`Dns`, `connect`).
pub fn check_url(u: &Url) -> Result<(), &'static str> {
    if !matches!(u.scheme(), "http" | "https") || !port_allowed(u.port_or_known_default().unwrap_or(0)) {
        return Err("remote_err_blocked");
    }
    let ok = match u.host() {
        Some(Host::Domain(d)) => !tracker(d) && !internal_name(d),
        Some(Host::Ipv4(ip)) => v4_allowed(ip),
        Some(Host::Ipv6(ip)) => v6_allowed(ip),
        None => false,
    };
    if ok { Ok(()) } else { Err("remote_err_blocked") }
}

/// Resolver for `reqwest`: internal answers are dropped, so a public name cannot point the node
/// at its own network, not even after a redirect or a rebinding.
pub struct Dns;

impl Resolve for Dns {
    fn resolve(&self, name: Name) -> Resolving {
        Box::pin(async move {
            let found = lookup_host((name.as_str(), 0)).await?;
            let good: Vec<SocketAddr> = found.filter(|a| ip_allowed(a.ip())).collect();
            if good.is_empty() {
                return Err(Box::new(io::Error::from(ErrorKind::PermissionDenied)) as Box<dyn std::error::Error + Send + Sync>);
            }
            Ok(Box::new(good.into_iter()) as Addrs)
        })
    }
}

/// TCP to `host:port` through the policy. Connects to the very addresses that were checked.
pub async fn connect(host: &str, port: u16) -> io::Result<TcpStream> {
    let denied = || io::Error::from(ErrorKind::PermissionDenied);
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    if !port_allowed(port) || bare.len() > 253 {
        return Err(denied());
    }
    let addrs: Vec<SocketAddr> = if let Ok(ip) = bare.parse::<IpAddr>() {
        vec![SocketAddr::new(ip, port)]
    } else {
        if tracker(bare) || internal_name(bare) {
            return Err(denied());
        }
        lookup_host((bare, port)).await?.collect()
    };
    let mut last = denied();
    for a in addrs.into_iter().filter(|a| ip_allowed(a.ip())) {
        match tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(a)).await {
            Ok(Ok(s)) => {
                let _ = s.set_nodelay(true);
                return Ok(s);
            }
            Ok(Err(e)) => last = e,
            Err(_) => last = ErrorKind::TimedOut.into(),
        }
    }
    Err(last)
}
