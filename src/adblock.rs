//! Builds a WebKit content-blocker list. Requests are blocked inside WebKit's network layer,
//! before any connection is made. Extra domains: Application Support/Crow/blocklist.txt
//! (plain domains, hosts-file or `||domain^` lines).

use crate::settings::data_dir;
use serde_json::{Value, json};

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

const MAX_RULES: usize = 50_000;

fn block_rule(domain: &str) -> Value {
    json!({
        "trigger": { "url-filter": format!(r"^[^:]+://([^/:]+\.)?{}[:/]", domain.replace('.', r"\.")) },
        "action": { "type": "block" }
    })
}

fn parse_line(line: &str) -> Option<&str> {
    let line = line.trim();
    if line.is_empty() || line.starts_with(['#', '!']) {
        return None;
    }
    let d = line.split_whitespace().last()?;
    let d = d.trim_start_matches("||").trim_end_matches('^');
    let valid = d.contains('.')
        && d.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        && d != "0.0.0.0";
    valid.then_some(d)
}

pub fn rules_json() -> String {
    let mut rules: Vec<Value> = DOMAINS.iter().map(|d| block_rule(d)).collect();
    if let Ok(text) = std::fs::read_to_string(data_dir().join("blocklist.txt")) {
        let extra = text.lines().filter_map(parse_line).map(|d| block_rule(&d.to_ascii_lowercase()));
        rules.extend(extra.take(MAX_RULES.saturating_sub(rules.len())));
    }
    rules.push(json!({
        "trigger": { "url-filter": ".*" },
        "action": { "type": "css-display-none", "selector": HIDE }
    }));
    Value::Array(rules).to_string()
}
