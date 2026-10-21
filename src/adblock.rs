//! Builds a WebKit content-blocker list. Requests are blocked inside WebKit's network layer,
//! before any connection is made (WKContentRuleList on macOS, WebKitUserContentFilter on Linux;
//! same JSON format). Sources: built-in domains, the StevenBlack hosts list (downloaded to
//! `data_dir()`, refreshed weekly) and `data_dir()/blocklist.txt`
//! (plain domains, hosts-file or `||domain^` lines).

use crate::{settings::data_dir, update};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs,
    path::PathBuf,
    sync::Mutex,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const DOMAINS: &[&str] = &[
    // Google ads & analytics
    "doubleclick.net", "googlesyndication.com", "googleadservices.com", "google-analytics.com",
    "googletagmanager.com", "googletagservices.com", "adservice.google.com", "imasdk.googleapis.com",
    // Ad exchanges / SSPs
    "adnxs.com", "adsrvr.org", "advertising.com", "amazon-adsystem.com", "criteo.com", "criteo.net",
    "taboola.com", "outbrain.com", "rubiconproject.com", "pubmatic.com", "openx.net",
    "casalemedia.com", "smartadserver.com", "adform.net", "bidswitch.net", "yieldmo.com", "teads.tv",
    "sharethrough.com", "3lift.com", "indexww.com", "media.net", "contextweb.com", "lijit.com",
    "gumgum.com", "spotxchange.com", "springserve.com", "adcolony.com", "applovin.com",
    "moatads.com", "serving-sys.com", "zedo.com", "revcontent.com", "mgid.com", "propellerads.com",
    "popads.net", "adsterra.com", "exoclick.com", "onclickads.net", "adroll.com", "quantcount.com",
    "33across.com", "connatix.com", "primis.tech", "seedtag.com", "adition.com", "adocean.pl",
    "etarget.sk", "etarget.cz", "r2b2.cz", "r2b2.io", "ssp.seznam.cz",
    // Trackers / telemetry
    "scorecardresearch.com", "quantserve.com", "hotjar.com", "mouseflow.com", "fullstory.com",
    "crazyegg.com", "mixpanel.com", "cdn.segment.com", "api.segment.io", "nr-data.net",
    "bat.bing.com", "clarity.ms", "px.ads.linkedin.com", "ads.linkedin.com", "analytics.twitter.com",
    "ads-twitter.com", "analytics.tiktok.com", "ct.pinterest.com", "tr.snapchat.com",
    "mc.yandex.ru", "an.yandex.ru", "gemius.pl", "hit.gemius.pl", "chartbeat.com", "chartbeat.net",
    "parsely.com", "branch.io", "app-measurement.com", "adjust.com", "appsflyer.com", "kochava.com",
    "omtrdc.net", "demdex.net", "everesttech.net", "krxd.net", "bluekai.com", "exelator.com",
    "rlcdn.com", "tapad.com", "agkn.com", "mathtag.com", "eyeota.net", "id5-sync.com",
    "liadm.com", "permutive.com", "onetag-sys.com",
];

const HIDE: &str = "ins.adsbygoogle, .adsbygoogle, [id^='google_ads_iframe'], [id^='div-gpt-ad'], \
    iframe[src*='doubleclick.net'], .ad-banner, .advertisement, .adsbox, [data-ad-slot]";

/// WebKit's content-blocker limit is 150 000 rules (one is the cosmetic rule).
const MAX_RULES: usize = 150_000;
pub const HOSTS_URL: &str = "https://raw.githubusercontent.com/StevenBlack/hosts/master/hosts";
const HOSTS_FILE: &str = "hosts-stevenblack.txt";
const HOSTS_MAX_BYTES: u64 = 16 * 1024 * 1024;
const HOSTS_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 3600);
/// SHA-256 of the last successfully compiled rule JSON: same hash = reuse WebKit's compiled list.
const HASH_FILE: &str = "adblock.sha256";

/// Hosts-file entries that are not ad domains.
const NOT_DOMAINS: &[&str] = &["localhost.localdomain", "local", "broadcasthost", "ip6-localhost", "ip6-loopback"];

#[derive(Serialize, Clone, Default)]
pub struct Stats {
    pub builtin: usize,
    pub hosts: usize,
    pub custom: usize,
    /// Block rules after dropping subdomains of already blocked domains.
    pub rules: usize,
    pub truncated: bool,
    /// Download time of the hosts list (ms since epoch).
    pub hosts_time: Option<u64>,
}

pub struct Rules {
    pub json: String,
    pub hash: String,
}

static STATS: Mutex<Option<Stats>> = Mutex::new(None);

pub fn stats() -> Stats {
    STATS.lock().ok().and_then(|s| s.clone()).unwrap_or_default()
}

fn hosts_path() -> PathBuf {
    data_dir().join(HOSTS_FILE)
}

fn block_rule(domain: &str) -> Value {
    json!({
        "trigger": { "url-filter": format!(r"^[^:]+://([^/:]+\.)?{}[:/]", domain.replace('.', r"\.")) },
        "action": { "type": "block" }
    })
}

fn parse_line(line: &str) -> Option<&str> {
    let line = line.split('#').next()?.trim();
    if line.is_empty() || line.starts_with('!') {
        return None;
    }
    let d = line.split_whitespace().last()?;
    let d = d.trim_start_matches("||").trim_end_matches('^');
    let valid = d.contains('.')
        && d.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        && !d.bytes().all(|b| b.is_ascii_digit() || b == b'.')
        && !NOT_DOMAINS.contains(&d);
    valid.then_some(d)
}

fn read_list(path: &PathBuf) -> Vec<String> {
    fs::read_to_string(path)
        .map(|t| t.lines().filter_map(parse_line).map(|d| d.to_ascii_lowercase()).collect())
        .unwrap_or_default()
}

/// Reads all sources and builds the rule JSON. Runs off the main thread (~100k domains).
pub fn build() -> Rules {
    let mut seen = HashSet::new();
    let mut order: Vec<String> = Vec::new();
    let mut add = |list: Vec<String>| {
        let before = order.len();
        for d in list {
            if seen.insert(d.clone()) {
                order.push(d);
            }
        }
        order.len() - before
    };
    // Priority on truncation: built-in, then the user's list, then the downloaded one.
    let builtin = add(DOMAINS.iter().map(|d| d.to_string()).collect());
    let custom = add(read_list(&data_dir().join("blocklist.txt")));
    let hosts = add(read_list(&hosts_path()));
    // `a.b.com` is redundant when `b.com` is blocked (rules cover subdomains).
    let covered = |d: &str| d.match_indices('.').any(|(i, _)| seen.contains(&d[i + 1..]));
    let mut domains: Vec<&String> = order.iter().filter(|d| !covered(d)).collect();
    let truncated = domains.len() > MAX_RULES - 1;
    domains.truncate(MAX_RULES - 1);

    let mut rules: Vec<Value> = domains.iter().map(|d| block_rule(d)).collect();
    rules.push(json!({
        "trigger": { "url-filter": ".*" },
        "action": { "type": "css-display-none", "selector": HIDE }
    }));
    let hosts_time = fs::metadata(hosts_path())
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64);
    let stats = Stats { builtin, hosts, custom, rules: domains.len(), truncated, hosts_time };
    if let Ok(mut s) = STATS.lock() {
        *s = Some(stats);
    }
    let json = Value::Array(rules).to_string();
    let hash = Sha256::digest(json.as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
    Rules { json, hash }
}

/// WebKit already holds a compiled list for exactly these rules.
pub fn is_compiled(hash: &str) -> bool {
    fs::read_to_string(data_dir().join(HASH_FILE)).is_ok_and(|h| h.trim() == hash)
}

pub fn mark_compiled(hash: &str) {
    let _ = fs::create_dir_all(data_dir());
    let _ = fs::write(data_dir().join(HASH_FILE), hash);
}

/// Hosts list missing or older than a week.
pub fn hosts_stale() -> bool {
    fs::metadata(hosts_path())
        .and_then(|m| m.modified())
        .map_or(true, |t| SystemTime::now().duration_since(t).map_or(true, |age| age > HOSTS_MAX_AGE))
}

pub fn remove_hosts() {
    let _ = fs::remove_file(hosts_path());
}

/// Downloads the StevenBlack list (blocking); replaces the cached copy only if it looks valid.
pub fn fetch_hosts() -> Result<usize, String> {
    let dir = data_dir();
    let _ = fs::create_dir_all(&dir);
    let part = dir.join(format!("{HOSTS_FILE}.part"));
    let ok = update::curl()
        .args(["--max-time", "120", "--max-filesize"])
        .arg(HOSTS_MAX_BYTES.to_string())
        .arg("-o")
        .arg(&part)
        .arg(HOSTS_URL)
        .status()
        .is_ok_and(|s| s.success());
    let count = if ok { read_list(&part).len() } else { 0 };
    if count < 1000 {
        let _ = fs::remove_file(&part);
        return Err(if ok { "neplatný zoznam".into() } else { "zoznam sa nepodarilo stiahnuť".into() });
    }
    fs::rename(&part, hosts_path()).map_err(|_| "zoznam sa nepodarilo uložiť".to_string())?;
    Ok(count)
}
