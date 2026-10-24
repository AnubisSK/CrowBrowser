//! Ad/tracker blocking rules. Filter lists (EasyList, EasyPrivacy, StevenBlack hosts, the
//! built-in list and `data_dir()/blocklist.txt`) are parsed with Brave's `adblock` crate and
//! converted to WebKit content-blocker JSON, so requests are still blocked inside WebKit's network
//! layer before any connection is made (WKContentRuleList / WebKitUserContentFilter).
//! One group of WebKit lists per source (built-in + blocklist.txt, EasyList, EasyPrivacy, hosts),
//! because WebKit allows at most 150 000 rules per list (see `Group`, `pack`).
//! Parsing runs only when an input changes (input hash); otherwise WebKit's compiled copy is reused.

use crate::{settings::data_dir, update};
use adblock::{
    content_blocking::{CbAction, CbRule, CbTrigger, CbType},
    lists::{FilterFormat, FilterSet, ParseOptions},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fmt::Write as _,
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
/// Also the page-side privacy shim: gamepads fed by `gamepad::Bridge` (global `gamepad`, flipped
/// for `gamepad_flip` sites), WebGL off for `webgl_off` sites, and `event`: per-session event name for Crow's
/// isolated world. Runs in every frame of every tab.
pub fn defuser(gamepad: bool, gamepad_flip: &[String], webgl_off: &[String], event: &str) -> String {
    fn json<T: Serialize + ?Sized>(v: &T) -> String {
        serde_json::to_string(v).unwrap_or_default()
    }
    include_str!("defuser.js")
        .replace("/*GAMEPAD*/false", if gamepad { "true" } else { "false" })
        .replace("/*GAMEPAD_FLIP*/[]", &json(gamepad_flip))
        .replace("/*WEBGL_OFF*/[]", &json(webgl_off))
        .replace("/*PAD_EVENT*/''", &json(event))
}

/// The only gamepad id pages ever see: one common standard-mapping controller.
pub const PAD_ID: &str = "Xbox 360 Controller (STANDARD GAMEPAD)";

/// WebKit's content-blocker limit per rule list.
const MAX_RULES: usize = 150_000;
/// Bump when the conversion changes, so cached compiled lists are rebuilt.
const FORMAT: &str = "adblock-0.13/3";
const MAX_LIST_BYTES: u64 = 16 * 1024 * 1024;
const MAX_AGE: Duration = Duration::from_hours(168);

/// One logical filter list. WebKit takes at most 150 000 rules per list and exceptions only
/// cancel rules of their own list, so each source gets its own group; a group that is still too
/// large is split into several lists (`parts`), each carrying all of the group's exceptions.
pub struct Group {
    key: &'static str,
    pub parts: &'static [&'static str],
}

pub const GROUPS: &[Group] = &[
    Group { key: "custom", parts: &["crow-custom", "crow-custom-2"] },
    Group { key: "easylist", parts: &["crow-easylist", "crow-easylist-2", "crow-easylist-3"] },
    Group { key: "easyprivacy", parts: &["crow-easyprivacy", "crow-easyprivacy-2"] },
    Group { key: "hosts", parts: &["crow-hosts", "crow-hosts-2"] },
];

/// Rule lists of older versions, removed from WebKit's store.
pub const LEGACY: &[&str] = &["crow-adblock"];

/// Downloadable filter list: name, file in `data_dir()`, URL, group it belongs to.
pub struct Source {
    pub name: &'static str,
    pub file: &'static str,
    pub url: &'static str,
    group: &'static str,
}

impl Source {
    pub fn enabled(&self, easylist: bool, hosts: bool) -> bool {
        if self.group == "hosts" { hosts } else { easylist }
    }
}

pub const SOURCES: &[Source] = &[
    Source { name: "EasyList", file: "easylist.txt", url: "https://easylist.to/easylist/easylist.txt", group: "easylist" },
    Source { name: "EasyPrivacy", file: "easyprivacy.txt", url: "https://easylist.to/easylist/easyprivacy.txt", group: "easyprivacy" },
    Source {
        name: "StevenBlack hosts",
        file: "hosts-stevenblack.txt",
        url: "https://raw.githubusercontent.com/StevenBlack/hosts/master/hosts",
        group: "hosts",
    },
];

/// Result of the last conversion of one group, kept next to its input hash.
#[derive(Serialize, Deserialize, Clone, Default)]
pub struct ListStats {
    pub filters: usize,
    pub rules: usize,
    pub parts: usize,
    /// Rules that did not fit even into all parts (cosmetic ones go first; normally 0).
    pub dropped: usize,
}

#[derive(Serialize, Deserialize)]
struct Marker {
    hash: String,
    parts: usize,
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
    u64::try_from(t.duration_since(UNIX_EPOCH).ok()?.as_millis()).ok()
}

/// Panel data: sources (downloaded or not, when) and rule counts (main = all but hosts).
pub fn status() -> Value {
    let sources: Vec<Value> = SOURCES
        .iter()
        .map(|s| json!({ "name": s.name, "file": s.file, "time": modified_ms(&path(s.file)) }))
        .collect();
    let sum = |keys: &[&str]| {
        let all: Vec<ListStats> = keys.iter().filter_map(|k| stats_of(k)).collect();
        (!all.is_empty()).then(|| {
            let dropped: usize = all.iter().map(|s| s.dropped).sum();
            json!({
                "rules": all.iter().map(|s| s.rules).sum::<usize>(),
                "lists": all.iter().map(|s| s.parts).sum::<usize>(),
                "dropped": dropped,
                "truncated": dropped > 0,
            })
        })
    };
    json!({
        "builtin": DOMAINS.len(),
        "sources": sources,
        "main": sum(&["custom", "easylist", "easyprivacy"]),
        "hosts": sum(&["hosts"]),
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

/// Raw inputs of a group (`None` = no sources, i.e. disabled or not downloaded).
fn inputs(g: &Group) -> Option<Vec<(String, FilterFormat)>> {
    if g.key == "custom" {
        let builtin: Vec<String> = DOMAINS.iter().map(|d| format!("||{d}^")).chain([format!("##{HIDE}")]).collect();
        return Some(vec![(builtin.join("\n"), FilterFormat::Standard), (custom_rules().join("\n"), FilterFormat::Standard)]);
    }
    let lists: Vec<String> =
        SOURCES.iter().filter(|s| s.group == g.key).filter_map(|s| fs::read_to_string(path(s.file)).ok()).collect();
    if lists.is_empty() {
        return None;
    }
    let hosts = g.key == "hosts";
    Some(lists.into_iter().map(|t| (if hosts { hosts_rules(&t).join("\n") } else { t }, FilterFormat::Standard)).collect())
}

/// Group and part index of a WebKit rule-list id.
pub fn group_of(id: &str) -> Option<(&'static Group, usize)> {
    GROUPS.iter().find_map(|g| g.parts.iter().position(|p| *p == id).map(|i| (g, i)))
}

/// Hash of the group's inputs; `None` = group disabled.
pub fn input_hash(g: &Group) -> Option<String> {
    let inputs = inputs(g)?;
    let mut h = Sha256::new();
    h.update(FORMAT);
    h.update(g.key);
    for (text, _) in &inputs {
        h.update((text.len() as u64).to_le_bytes());
        h.update(text);
    }
    Some(h.finalize().iter().fold(String::with_capacity(64), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    }))
}

fn marker(id: &str) -> Option<Marker> {
    serde_json::from_str(&fs::read_to_string(marker_path(id)).ok()?).ok()
}

/// WebKit already compiled every part of this exact input: number of parts (restores stats).
pub fn compiled_parts(g: &Group, hash: &str) -> Option<usize> {
    let first = marker(g.parts[0]).filter(|m| m.hash == hash)?;
    let n = first.parts;
    let all = n >= 1 && n <= g.parts.len() && g.parts[1..n].iter().all(|p| marker(p).is_some_and(|m| m.hash == hash));
    all.then(|| {
        set_stats(g.key, Some(first.stats));
        n
    })
}

pub fn mark_compiled(id: &str, hash: &str, parts: usize) {
    let Some((g, _)) = group_of(id) else { return };
    let m = Marker { hash: hash.to_string(), parts, stats: stats_of(g.key).unwrap_or_default() };
    let _ = fs::create_dir_all(data_dir());
    let _ = fs::write(marker_path(id), serde_json::to_string(&m).unwrap_or_default());
}

pub fn forget(id: &str) {
    if let Some((g, 0)) = group_of(id) {
        set_stats(g.key, None);
    }
    let _ = fs::remove_file(marker_path(id));
}

/// `*domain` of every `@@||domain^$generichide` exception: the content-blocking conversion
/// drops these, so generic hiding (`##.ads`) would also hit e.g. Gmail's `div.adn.ads` message.
fn generichide_domains(text: &str) -> impl Iterator<Item = String> + '_ {
    text.lines().filter_map(|l| {
        let (pat, opts) = l.trim().strip_prefix("@@||")?.split_once('$')?;
        opts.split(',').any(|o| matches!(o, "generichide" | "ghide")).then_some(())?;
        let d = pat.strip_suffix('^').unwrap_or(pat);
        let plain = !d.is_empty() && d.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-');
        plain.then(|| format!("*{}", d.to_ascii_lowercase()))
    })
}

/// One exception cancelling generic cosmetic rules on `domains` (and their subdomains).
fn generichide_rule(mut domains: Vec<String>) -> Option<CbRule> {
    domains.sort();
    domains.dedup();
    (!domains.is_empty()).then(|| CbRule {
        action: CbAction { typ: CbType::IgnorePreviousRules, selector: None },
        trigger: CbTrigger { url_filter: ".*".into(), if_domain: Some(domains), ..Default::default() },
    })
}

/// Generic cosmetic rules first, then the generichide exception, then the rest: an exception
/// only cancels the rules before it, so network and site-specific rules stay in force there.
fn with_generichide(part: Vec<CbRule>, ghide: Option<&CbRule>) -> Vec<CbRule> {
    let Some(ghide) = ghide else { return part };
    let (mut out, rest): (Vec<_>, Vec<_>) = part
        .into_iter()
        .partition(|r| matches!(r.action.typ, CbType::CssDisplayNone) && r.trigger.if_domain.is_none());
    out.push(ghide.clone());
    out.extend(rest);
    out
}

/// Splits converted rules into WebKit lists of at most `MAX_RULES` without losing integrity:
/// every part ends with all exceptions (they only cancel rules of their own list); network
/// rules are placed before cosmetic ones, so only cosmetic rules are dropped if even
/// `max_parts` lists are not enough.
fn pack(rules: Vec<CbRule>, max_parts: usize, ghide: Option<&CbRule>) -> (Vec<Vec<CbRule>>, usize) {
    let extra = usize::from(ghide.is_some());
    if rules.len() + extra <= MAX_RULES {
        return (vec![with_generichide(rules, ghide)], 0);
    }
    let (mut ignore, mut cosmetic, mut network) = (Vec::new(), Vec::new(), Vec::new());
    for r in rules {
        match r.action.typ {
            CbType::IgnorePreviousRules => ignore.push(r),
            CbType::CssDisplayNone => cosmetic.push(r),
            _ => network.push(r),
        }
    }
    let room = MAX_RULES.saturating_sub(ignore.len() + extra).max(1);
    let total = network.len() + cosmetic.len();
    let mut parts: Vec<Vec<CbRule>> = Vec::new();
    for r in network.into_iter().chain(cosmetic) {
        if parts.last().is_none_or(|p| p.len() == room) {
            if parts.len() == max_parts {
                break;
            }
            parts.push(Vec::with_capacity(room));
        }
        if let Some(p) = parts.last_mut() {
            p.push(r);
        }
    }
    let kept: usize = parts.iter().map(Vec::len).sum();
    let parts = parts
        .into_iter()
        .map(|mut p| {
            p.extend(ignore.iter().cloned());
            with_generichide(p, ghide)
        })
        .collect();
    (parts, total - kept)
}

/// Parses the group's filters and converts them to content-blocker JSON, one string per WebKit
/// list (off the main thread; large transient allocation, freed when done).
pub fn build(g: &Group) -> Option<Vec<String>> {
    let mut set = FilterSet::new(true); // debug = keep raw lines, required by the conversion
    let mut ghide = Vec::new();
    for (text, format) in inputs(g)? {
        ghide.extend(generichide_domains(&text));
        set.add_filter_list(text, ParseOptions { format, ..Default::default() });
    }
    let (rules, used) = set.into_content_blocking().ok()?;
    let ghide = generichide_rule(ghide);
    let (parts, dropped) = pack(rules, g.parts.len(), ghide.as_ref());
    let rules = parts.iter().map(Vec::len).sum();
    set_stats(g.key, Some(ListStats { filters: used.len(), rules, parts: parts.len(), dropped }));
    parts.iter().map(|p| serde_json::to_string(p).ok()).collect()
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
    for s in SOURCES.iter().filter(|s| (s.group == "hosts") == hosts) {
        let _ = fs::remove_file(path(s.file));
    }
}

/// Downloads one list (blocking); replaces the cached copy only if it looks valid.
/// Errors are `locales.json` keys taking the list name.
pub fn fetch(s: &Source) -> Result<(), &'static str> {
    let _ = fs::create_dir_all(data_dir());
    let part = path(&format!("{}.part", s.file));
    if update::private_file(&part).is_err() {
        return Err("save_failed");
    }
    let ok = update::curl()
        .args(["--max-time", "120", "--max-filesize"])
        .arg(MAX_LIST_BYTES.to_string())
        .arg("-o")
        .arg(&part)
        .arg(s.url)
        .status()
        .is_ok_and(|st| st.success());
    let lines = if ok { fs::read_to_string(&part).map_or(0, |t| t.lines().count()) } else { 0 };
    if lines < 1000 {
        let _ = fs::remove_file(&part);
        return Err("fetch_failed");
    }
    fs::rename(&part, path(s.file)).map_err(|_| "save_failed")
}
