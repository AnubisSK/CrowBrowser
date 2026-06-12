namespace CrowBrowser {

    public class AdBlockManager : Object {

        private static AdBlockManager? _instance = null;

        public static AdBlockManager get_instance () {
            if (_instance == null) _instance = new AdBlockManager ();
            return _instance;
        }

        public signal void filter_loaded ();

        private WebKit.UserContentFilterStore? store = null;
        private WebKit.UserContentFilter?      _filter = null;
        private bool _enabled;
        private bool _loading = false;

        public WebKit.UserContentFilter? filter { get { return _filter; } }

        public bool enabled {
            get { return _enabled; }
            set {
                if (_enabled == value) return;
                _enabled = value;
                if (_enabled && _filter == null && !_loading)
                    load_or_build.begin ();
            }
        }

        construct {
            _enabled = SettingsManager.get_instance ().get_adblock_enabled ();
            string dir = GLib.Path.build_filename (
                GLib.Environment.get_user_data_dir (), "crow-browser", "content-filters"
            );
            GLib.DirUtils.create_with_parents (dir, 0755);
            store = new WebKit.UserContentFilterStore (dir);
            if (_enabled) load_or_build.begin ();
        }

        // Apply all blocking layers to a UCM.
        public void apply_to (WebKit.UserContentManager ucm) {
            if (!_enabled) return;

            // ── Popup blocker at START — runs before any page script ──────────
            // Overrides window.open so pop-unders and pop-ups are blocked.
            var popup_script = new WebKit.UserScript (
                POPUP_BLOCK_JS,
                WebKit.UserContentInjectedFrames.ALL_FRAMES,
                WebKit.UserScriptInjectionTime.START,
                null, null
            );
            ucm.add_script (popup_script);

            // ── Cosmetic CSS — hides ad elements without needing network rules ──
            var css = new WebKit.UserStyleSheet (
                COSMETIC_CSS,
                WebKit.UserContentInjectedFrames.ALL_FRAMES,
                WebKit.UserStyleLevel.USER,
                null, null
            );
            ucm.add_style_sheet (css);

            // ── Overlay remover + YT ad-skip at END ───────────────────────────
            var skip = new WebKit.UserScript (
                AD_CLEANUP_JS,
                WebKit.UserContentInjectedFrames.ALL_FRAMES,
                WebKit.UserScriptInjectionTime.END,
                null, null
            );
            ucm.add_script (skip);

            // ── Network filter — blocks requests to known ad domains ─────────
            if (_filter != null) {
                ucm.add_filter (_filter);
            } else {
                weak WebKit.UserContentManager w = ucm;
                ulong sid = 0;
                sid = filter_loaded.connect (() => {
                    if (w != null && _filter != null) w.add_filter (_filter);
                    if (sid != 0) { disconnect (sid); sid = 0; }
                });
            }
        }

        // ── Async filter loader ────────────────────────────────────────────────

        private async void load_or_build () {
            _loading = true;
            const string FILTER_ID = "crow-adblock-v2";
            try {
                _filter = yield store.load (FILTER_ID, null);
            } catch {
                _filter = null;
            }

            if (_filter == null) {
                string json = build_filter_json ();
                var bytes = new GLib.Bytes (json.data);
                try {
                    _filter = yield store.save (FILTER_ID, bytes, null);
                } catch (GLib.Error e) {
                    warning ("AdBlock: failed to compile filter: %s", e.message);
                }
            }

            _loading = false;
            if (_filter != null) filter_loaded ();
        }

        // ── Popup blocker (injected at START, before any page script) ────────────

        private const string POPUP_BLOCK_JS = """
(function () {
  if (window.__crowPopBlock) return;
  window.__crowPopBlock = true;

  /* Block window.open — stops pop-unders and pop-ups.
     Only blocks if triggered without a direct user gesture (mousedown/keydown
     within last 600 ms), which lets legit login popups through. */
  var _origOpen = window.open;
  var _lastGesture = 0;
  document.addEventListener('mousedown', function () { _lastGesture = Date.now(); }, true);
  document.addEventListener('keydown',   function () { _lastGesture = Date.now(); }, true);
  document.addEventListener('touchstart', function () { _lastGesture = Date.now(); }, true);

  window.open = function (url, name, features) {
    if (!url || url === '' || url === 'about:blank')
      return _origOpen.call(this, url, name, features);
    /* Allow same-origin (e.g. OAuth popups on same domain) */
    try {
      var u = new URL(url, window.location.href);
      if (u.origin === window.location.origin)
        return _origOpen.call(this, url, name, features);
    } catch (e) {}
    /* Allow if opened within 600 ms of a real user gesture */
    if (Date.now() - _lastGesture < 600)
      return _origOpen.call(this, url, name, features);
    /* Block everything else (pop-unders, auto-popups) */
    return { closed: true, close: function(){}, focus: function(){},
             blur: function(){}, postMessage: function(){} };
  };
})();
""";

        // ── CSS cosmetic filter ────────────────────────────────────────────────

        private const string COSMETIC_CSS = """
/* ── YouTube ── */
ytd-ad-slot-renderer,
ytd-action-companion-ad-renderer,
ytd-banner-promo-renderer,
ytd-in-feed-ad-layout-renderer,
ytd-promoted-sparkles-web-renderer,
ytd-promoted-video-renderer,
ytd-statement-banner-renderer,
ytd-carousel-ad-renderer,
ytd-display-ad-renderer,
ytd-promoted-sparkles-text-search-renderer,
ytd-rich-item-renderer:has(ytd-ad-slot-renderer),
yt-mealbar-promo-renderer,
yt-about-this-ad-renderer,
#masthead-ad,
#player-ads,
.ytp-ad-overlay-container,
.ytp-ad-text-overlay,
.ytp-ad-image-overlay,
.ytp-suggested-action,
.video-ads.ytp-ad-module {
  display: none !important;
  pointer-events: none !important;
}

/* ── Generic ad selectors ── */
.ad-slot, .ad-unit, .ad-banner, .ad-block, .ad-block-outer, .ad-wrap,
.ad-container, .ad-wrapper, .ad-frame, .adslot, .ad_unit,
.adsbygoogle, ins.adsbygoogle,
[id^="div-gpt-ad"], [id^="google_ads_iframe"], [id^="adngin-"],
iframe[id^="google_ads"], [class*="GoogleAd"],
.dfp-ad, .dfp-slot, .adsense-ad, .adsense_ad,
.widget-ad, .widget-ads, .sidebar-ad,
.sponsored-post, .sponsored-content, .sponsored-label,
.native-ad, .nativead, .native_ad,
.promo-ad, .promotional-ad, .advertisement, .advertise,
.advert, .adverts, .advertising, .advertisment,
#adblock-notification, .adblock-warning, .adblock-message,
[class*="ad-leaderboard"], [class*="ad-rectangle"],
[class*="banner-ad"], [class*="banner_ad"],
[data-ad], [data-google-query-id],
.commercial-unit-mobile-top,
.right-rail > section:first-child[class*="ad"] { display: none !important; }

/* ── Twitch ── */
.tw-interstitial-ad,
[data-test-selector="ad-banner-default-container"] { display: none !important; }

/* ── Reddit ── */
.promotedlink, [data-promoted] { display: none !important; }

/* ── Facebook ── */
[data-pagelet*="FeedUnit"][class*="sponsored"],
[data-ad-preview] { display: none !important; }

/* ── Streaming / piracy sites — exact known class names only ── */
.ad-overlay, .ads-overlay,
.ad-popup, .ads-popup, .popup-ad, .popunder-ad,
.preroll-ad, .pre-roll, #preroll, .preroll-overlay,
/* iframes from specific known ad networks */
iframe[src*="popads.net"],
iframe[src*="popcash.net"],
iframe[src*="propellerads.com"],
iframe[src*="adsterra.com"],
iframe[src*="exoclick.com"],
iframe[src*="hilltopads.net"],
iframe[src*="juicyads.com"],
iframe[src*="trafficjunky.net"],
iframe[src*="clickadu.com"] { display: none !important; }
""";

        // ── Overlay remover + ad-skip (injected at END) ───────────────────────

        private const string AD_CLEANUP_JS = """
(function () {
  if (window.__crowAdClean) return;
  window.__crowAdClean = true;

  /* Remove click-redirect handlers on <body> — safe, only affects the body element */
  function cleanBody () {
    if (!document.body) return;
    document.body.removeAttribute('onclick');
    document.body.removeAttribute('onmousedown');
    document.body.removeAttribute('onmouseup');
    document.body.removeAttribute('onpointerdown');
  }
  cleanBody();
  document.addEventListener('DOMContentLoaded', cleanBody);

  /* ── YouTube ad skipper ── */
  function handleYT () {
    var btn = document.querySelector(
      '.ytp-skip-ad-button, .ytp-ad-skip-button, .ytp-ad-skip-button-modern');
    if (btn && btn.offsetParent !== null) { btn.click(); return; }
    var adVid = document.querySelector('.ad-showing video');
    if (adVid && adVid.duration > 0 && isFinite(adVid.duration))
      adVid.currentTime = adVid.duration;
    var dismiss = document.querySelector('.ytp-ad-overlay-close-button');
    if (dismiss) dismiss.click();
  }
  if (window.location.hostname.indexOf('youtube.com') !== -1)
    setInterval(handleYT, 300);

  /* ── Streaming-site specific overlay removal ──
     Only removes elements matching EXACT known ad class/id patterns.
     Does NOT touch elements based on z-index/coverage to avoid breaking
     Cloudflare challenges, Google sign-in, video players, etc. */
  var EXACT_AD_SELECTORS = [
    /* pop-under / pre-roll overlay wrappers */
    '.ad-overlay', '.ads-overlay',
    '#ad-overlay', '#ads-overlay',
    '.popup-ad', '.popunder-ad', '.ad-popup',
    '.preroll-ad', '.pre-roll', '#preroll',
    '.preroll-overlay', '#preroll-overlay',
    /* iframes from specific ad networks */
    'iframe[src*="popads.net"]',
    'iframe[src*="popcash.net"]',
    'iframe[src*="propellerads.com"]',
    'iframe[src*="adsterra.com"]',
    'iframe[src*="exoclick.com"]',
    'iframe[src*="hilltopads.net"]',
    'iframe[src*="juicyads.com"]',
    'iframe[src*="trafficjunky.net"]',
    'iframe[src*="clickadu.com"]',
    'iframe[src*="adnxs.com"]',
  ];

  function removeKnownAdEls () {
    cleanBody();
    try {
      document.querySelectorAll(EXACT_AD_SELECTORS.join(',')).forEach(
        function (el) { el.remove(); }
      );
    } catch (e) {}
  }

  removeKnownAdEls();

  /* Re-run once after a short delay to catch lazy-loaded ad injections */
  setTimeout(removeKnownAdEls, 2000);
  setTimeout(removeKnownAdEls, 5000);
})();
""";

        // ── Network filter JSON builder ────────────────────────────────────────

        private static string build_filter_json () {
            string[] domains = {
                // Ad networks
                "doubleclick.net",
                "googleadservices.com",
                "googlesyndication.com",
                "adnxs.com",
                "advertising.com",
                "adsrvr.org",
                "openx.net",
                "rubiconproject.com",
                "pubmatic.com",
                "casalemedia.com",
                "appnexus.com",
                "criteo.com",
                "criteo.net",
                "outbrain.com",
                "taboola.com",
                "media.net",
                "a9.com",
                "amazon-adsystem.com",
                "2mdn.net",
                "admixer.net",
                "smartadserver.com",
                "yieldmo.com",
                "conversantmedia.com",
                "bidswitch.net",
                "contextweb.com",
                "indexww.com",
                "sharethrough.com",
                "triplelift.com",
                "sovrn.com",
                "lijit.com",
                "spotxchange.com",
                "spotx.tv",
                "teads.tv",
                "teads.net",
                "turn.com",
                "undertone.com",
                "yieldlab.net",
                "sonobi.com",
                "emxdgt.com",
                "sekindo.com",
                "stickyadstv.com",
                "magnite.com",
                "freewheel.tv",
                "loopme.com",
                "revcontent.com",
                "33across.com",
                "zedo.com",
                "adform.net",
                "adform.io",
                "doubleverify.com",
                "adsafeprotected.com",
                "springserve.com",
                "semasio.net",
                "connectad.io",
                "adkernel.com",
                "exoclick.com",
                "trafficjunky.net",
                "propellerads.com",
                "clickadu.com",
                "popcash.net",
                "popads.net",
                "adsterra.com",
                "mgid.com",
                "buysellads.com",
                "adition.com",
                "ads.yahoo.com",
                // Tracking & analytics
                "google-analytics.com",
                "googletagmanager.com",
                "googletagservices.com",
                "scorecardresearch.com",
                "omtrdc.net",
                "demdex.net",
                "quantserve.com",
                "bluekai.com",
                "addthis.com",
                "segment.io",
                "segment.com",
                "amplitude.com",
                "heapanalytics.com",
                "fullstory.com",
                "mouseflow.com",
                "hotjar.com",
                "mixpanel.com",
                "statcounter.com",
                "clicky.com",
                "chartbeat.com",
                "chartbeat.net",
                "newrelic.com",
                "nr-data.net",
                "bat.bing.com",
                "ads.twitter.com",
                "tr.snapchat.com",
                "analytics.tiktok.com",
                "ads.tiktok.com",
                "advertising.linkedin.com",
                "px.ads.linkedin.com",
                "dotomi.com",
                "adjust.com",
                "kochava.com",
                "appsflyer.com",
                "branch.io",
                "moatads.com",
                "moat.com",
                "braze.com",
                "appboy.com",
                "smartlook.com",
                "luckyorange.com",
                "crazyegg.com",
                "kissmetrics.com",
                "sessioncam.com",
                "clicktale.net",
                "logrocket.io",
                "hs-scripts.com",
                "hsforms.com",
                "hsforms.net",
                "drift.com",
                "driftt.com",
                "intercom.io",
            };

            // Path-based rules for sites that serve ads from their own domain
            string[] path_rules = {
                // YouTube ad endpoints
                "^https?://([^/]*\\.)?youtube\\.com/pagead/",
                "^https?://([^/]*\\.)?youtube\\.com/api/stats/ads",
                "^https?://([^/]*\\.)?youtube\\.com/api/stats/qoe.*adformat",
                "^https?://([^/]*\\.)?youtube\\.com/youtubei/.*/ad_break",
                // Google ad endpoints
                "^https?://([^/]*\\.)?google\\.com/pagead/",
                "^https?://pagead2\\.googlesyndication\\.com/",
                "^https?://([^/]*\\.)?googleadapis\\.com/",
            };

            var sb = new GLib.StringBuilder ();
            sb.append ("[");
            bool first = true;

            // Domain rules
            foreach (string domain in domains) {
                if (!first) sb.append_c (',');
                first = false;

                var re = new GLib.StringBuilder ();
                int dlen = domain.length;
                for (int i = 0; i < dlen; i++) {
                    char c = domain[i];
                    if (c == '.') re.append ("\\\\.");
                    else re.append_c (c);
                }
                sb.append ("{\"trigger\":{\"url-filter\":\"^https?://([^/]*\\\\.)?" );
                sb.append (re.str);
                sb.append ("[/?]\"},\"action\":{\"type\":\"block\"}}");
            }

            // Path-based rules (already full regex strings)
            foreach (string rule in path_rules) {
                sb.append_c (',');
                // Escape the rule for JSON (backslashes → \\, quotes → \")
                var escaped = new GLib.StringBuilder ();
                int rlen = rule.length;
                for (int i = 0; i < rlen; i++) {
                    char c = rule[i];
                    if (c == '\\') escaped.append ("\\\\");
                    else if (c == '"') escaped.append ("\\\"");
                    else escaped.append_c (c);
                }
                sb.append ("{\"trigger\":{\"url-filter\":\"");
                sb.append (escaped.str);
                sb.append ("\"},\"action\":{\"type\":\"block\"}}");
            }

            sb.append ("]");
            return sb.str;
        }
    }
}
