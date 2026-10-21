//! Ad/tracker blocking rules. Filter lists (EasyList, EasyPrivacy, StevenBlack hosts, the
//! built-in list and `data_dir()/blocklist.txt`) are parsed with Brave's `adblock` crate and
//! converted to WebKit content-blocker JSON, so requests are still blocked inside WebKit's network
//! layer before any connection is made (WKContentRuleList / WebKitUserContentFilter).
//! Two lists, because WebKit allows at most 150 000 rules per list:
//! `crow-adblock` (EasyList + EasyPrivacy + built-in + blocklist.txt) and `crow-hosts`.
//! Parsing runs only when an input changes (input hash); otherwise WebKit's compiled copy is reused.

use crate::{settings::data_dir, update};
use adblock::lists::{FilterFormat, FilterSet, ParseOptions};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
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

/// Anti-adblock defuser, injected at document start into every frame of every tab. WebKit's
/// content blockers abort blocked loads (no empty 200/204 responses possible), so the page-side
/// symptoms are neutralized instead: ad resource errors become `load`, ad fetches resolve with an
/// empty 200, bait globals are set and hidden bait elements report a non-zero size.
pub const DEFUSER: &str = include_str!("defuser.js");

/// WebKit's content-blocker limit per rule list.
const MAX_RULES: usize = 150_000;
/// Bump when the conversion changes, so cached compiled lists are rebuilt.
const FORMAT: &str = "adblock-0.13/1";
const MAX_LIST_BYTES: u64 = 16 * 1024 * 1024;
const MAX_AGE: Duration = Duration::from_secs(7 * 24 * 3600);

pub const MAIN: &str = "crow-adblock";
pub const HOSTS: &str = "crow-hosts";
pub const LISTS: [&str; 2] = [MAIN, HOSTS];

/// Downloadable filter list: (name, file in `data_dir()`, URL, format, rule list it goes into).
pub struct Source {
    pub name: &'static str,
    pub file: &'static str,
    pub url: &'static str,
    hosts: bool,
}

impl Source {
    pub fn enabled(&self, easylist: bool, hosts: bool) -> bool {
        if self.hosts { hosts } else { easylist }
    }
}

pub const SOURCES: &[Source] = &[
    Source { name: "EasyList", file: "easylist.txt", url: "https://easylist.to/easylist/easylist.txt", hosts: false },
    Source { name: "EasyPrivacy", file: "easyprivacy.txt", url: "https://easylist.to/easylist/easyprivacy.txt", hosts: false },
    Source {
        name: "StevenBlack hosts",
        file: "hosts-stevenblack.txt",
        url: "https://raw.githubusercontent.com/StevenBlack/hosts/master/hosts",
        hosts: true,
    },
];

/// Result of the last conversion of one rule list, kept next to its input hash.
#[derive(Serialize, Deserialize, Clone, Default)]
pub struct ListStats {
    pub filters: usize,
    pub rules: usize,
    pub truncated: bool,
}

#[derive(Serialize, Deserialize)]
struct Marker {
    hash: String,
    stats: ListStats,
}

static STATS: Mutex<Option<HashMap<String, ListStats>>> = Mutex::new(None);

fn set_stats(id: &str, s: Option<ListStats>) {
    if let Ok(mut m) = STATS.lock() {
        let m = m.get_or_insert_with(HashMap::new);
        match s {
            Some(s) => m.insert(id.to_string(), s),
            None => m.remove(id),
        };
    }
}

fn stats_of(id: &str) -> Option<ListStats> {
    STATS.lock().ok()?.as_ref()?.get(id).cloned()
}

fn path(file: &str) -> PathBuf {
    data_dir().join(file)
}

fn marker_path(id: &str) -> PathBuf {
    data_dir().join(format!("{id}.json"))
}

fn modified_ms(p: &PathBuf) -> Option<u64> {
    let t = fs::metadata(p).and_then(|m| m.modified()).ok()?;
    Some(t.duration_since(UNIX_EPOCH).ok()?.as_millis() as u64)
}

/// Panel data: sources (downloaded or not, when) and per-list rule counts.
pub fn status() -> Value {
    let sources: Vec<Value> = SOURCES
        .iter()
        .map(|s| json!({ "name": s.name, "file": s.file, "time": modified_ms(&path(s.file)) }))
        .collect();
    json!({
        "builtin": DOMAINS.len(),
        "sources": sources,
        "main": stats_of(MAIN),
        "hosts": stats_of(HOSTS),
    })
}

/// blocklist.txt: ABP rules as-is; plain domains and hosts-file lines become `||domain^`.
fn custom_rules() -> Vec<String> {
    let Ok(text) = fs::read_to_string(path("blocklist.txt")) else { return Vec::new() };
    let is_domain = |d: &str| d.contains('.') && d.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-');
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with(['#', '!']))
        .map(|l| {
            let last = l.split_whitespace().last().unwrap_or(l);
            let ip_line = l.split_whitespace().count() == 2 && l.starts_with(|c: char| c.is_ascii_digit() || c == ':');
            if (ip_line || !l.contains(' ')) && is_domain(last) { format!("||{}^", last.to_ascii_lowercase()) } else { l.to_string() }
        })
        .collect()
}

/// Hosts entries, with subdomains of already listed domains dropped (`||d^` covers them).
fn hosts_rules(text: &str) -> Vec<String> {
    const NOT: &[&str] = &["localhost.localdomain", "local", "broadcasthost", "ip6-localhost", "ip6-loopback"];
    let domains: Vec<&str> = text
        .lines()
        .filter_map(|l| {
            let l = l.split('#').next()?.trim();
            let mut it = l.split_whitespace();
            let (_ip, d) = (it.next()?, it.next()?);
            let ok = d.contains('.') && !NOT.contains(&d) && d.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-');
            ok.then_some(d)
        })
        .collect();
    let set: HashSet<&str> = domains.iter().copied().collect();
    let covered = |d: &str| d.match_indices('.').any(|(i, _)| set.contains(&d[i + 1..]));
    let mut seen = HashSet::new();
    domains
        .into_iter()
        .filter(|d| !covered(d) && seen.insert(*d))
        .map(|d| format!("||{}^", d.to_ascii_lowercase()))
        .collect()
}

/// Raw inputs of a rule list (`None` = list has no sources, i.e. disabled).
fn inputs(id: &str) -> Option<Vec<(String, FilterFormat)>> {
    let read = |hosts: bool| -> Vec<String> {
        SOURCES
            .iter()
            .filter(|s| s.hosts == hosts)
            .filter_map(|s| fs::read_to_string(path(s.file)).ok())
            .collect()
    };
    if id == HOSTS {
        let lists = read(true);
        return (!lists.is_empty()).then(|| lists.iter().map(|t| (hosts_rules(t).join("\n"), FilterFormat::Standard)).collect());
    }
    let builtin: Vec<String> = DOMAINS.iter().map(|d| format!("||{d}^")).chain([format!("##{HIDE}")]).collect();
    let mut out = vec![(builtin.join("\n"), FilterFormat::Standard), (custom_rules().join("\n"), FilterFormat::Standard)];
    out.extend(read(false).into_iter().map(|t| (t, FilterFormat::Standard)));
    Some(out)
}

/// Hash of the list's inputs; `None` = list disabled.
pub fn input_hash(id: &str) -> Option<String> {
    let inputs = inputs(id)?;
    let mut h = Sha256::new();
    h.update(FORMAT);
    h.update(id);
    for (text, _) in &inputs {
        h.update((text.len() as u64).to_le_bytes());
        h.update(text);
    }
    Some(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// WebKit already compiled this exact input; restores its stats for the panel.
pub fn is_compiled(id: &str, hash: &str) -> bool {
    let Ok(text) = fs::read_to_string(marker_path(id)) else { return false };
    let Ok(m) = serde_json::from_str::<Marker>(&text) else { return false };
    let ok = m.hash == hash;
    if ok {
        set_stats(id, Some(m.stats));
    }
    ok
}

pub fn mark_compiled(id: &str, hash: &str) {
    let m = Marker { hash: hash.to_string(), stats: stats_of(id).unwrap_or_default() };
    let _ = fs::create_dir_all(data_dir());
    let _ = fs::write(marker_path(id), serde_json::to_string(&m).unwrap_or_default());
}

pub fn forget(id: &str) {
    set_stats(id, None);
    let _ = fs::remove_file(marker_path(id));
}

/// Parses the list's filters and converts them to content-blocker JSON (off the main thread;
/// large transient allocation, freed when done).
pub fn build(id: &str) -> Option<String> {
    let mut set = FilterSet::new(true); // debug = keep raw lines, required by the conversion
    for (text, format) in inputs(id)? {
        set.add_filter_list(text, ParseOptions { format, ..Default::default() });
    }
    let (mut rules, used) = set.into_content_blocking().ok()?;
    let truncated = rules.len() > MAX_RULES;
    if truncated {
        // Exceptions (ignore-previous-rules) are last and must stay; drop generic cosmetic
        // rules first, then the oldest block rules.
        rules.retain(|r| !(r.action.selector.is_some() && r.trigger.url_filter == ".*" && r.trigger.if_domain.is_none()));
        let over = rules.len().saturating_sub(MAX_RULES);
        rules.drain(..over);
    }
    set_stats(id, Some(ListStats { filters: used.len(), rules: rules.len(), truncated }));
    serde_json::to_string(&rules).ok()
}

/// Downloadable lists for the enabled options that are missing or older than a week.
pub fn stale(easylist: bool, hosts: bool) -> Vec<&'static Source> {
    let old = |s: &Source| {
        fs::metadata(path(s.file))
            .and_then(|m| m.modified())
            .map_or(true, |t| SystemTime::now().duration_since(t).map_or(true, |a| a > MAX_AGE))
    };
    SOURCES.iter().filter(|s| s.enabled(easylist, hosts) && old(s)).collect()
}

pub fn remove(hosts: bool) {
    for s in SOURCES.iter().filter(|s| s.hosts == hosts) {
        let _ = fs::remove_file(path(s.file));
    }
}

/// Downloads one list (blocking); replaces the cached copy only if it looks valid.
pub fn fetch(s: &Source) -> Result<(), String> {
    let _ = fs::create_dir_all(data_dir());
    let part = path(&format!("{}.part", s.file));
    let ok = update::curl()
        .args(["--max-time", "120", "--max-filesize"])
        .arg(MAX_LIST_BYTES.to_string())
        .arg("-o")
        .arg(&part)
        .arg(s.url)
        .status()
        .is_ok_and(|st| st.success());
    let lines = if ok { fs::read_to_string(&part).map(|t| t.lines().count()).unwrap_or(0) } else { 0 };
    if lines < 1000 {
        let _ = fs::remove_file(&part);
        return Err(format!("{} sa nepodarilo stiahnuť", s.name));
    }
    fs::rename(&part, path(s.file)).map_err(|_| format!("{} sa nepodarilo uložiť", s.name))
}
