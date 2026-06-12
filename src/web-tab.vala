namespace CrowBrowser {

    // Injected at START — installs intercepts before any page script runs
    private const string VIDEO_HOOK_JS = """
(function () {
  if (window.__crowHooksInstalled) return;
  window.__crowHooksInstalled = true;

  var reported = new Set();

  window.__crowSend = function (url) {
    if (!url || typeof url !== 'string') return;
    url = url.trim();
    if (!url || reported.has(url)) return;
    if (url.startsWith('data:') || url.startsWith('javascript:')) return;
    if (url.startsWith('//')) url = window.location.protocol + url;
    if (!url.startsWith('http') && !url.startsWith('blob:')) {
      try { url = new URL(url, window.location.href).href; } catch (_) { return; }
    }
    reported.add(url);
    try { window.webkit.messageHandlers.crowVideoDetected.postMessage(url); } catch (_) {}
  };
  var send = window.__crowSend;

  // XHR — capture actual m3u8/mpd stream URL
  var origOpen = XMLHttpRequest.prototype.open;
  XMLHttpRequest.prototype.open = function (method, url) {
    if (typeof url === 'string') {
      var u = url.toLowerCase();
      if (u.indexOf('.m3u8') !== -1 || u.indexOf('.mpd') !== -1)
        send(url.startsWith('http') ? url : new URL(url, window.location.href).href);
    }
    return origOpen.apply(this, arguments);
  };

  // Fetch — capture actual m3u8/mpd stream URL
  var origFetch = window.fetch;
  if (origFetch) {
    window.fetch = function (input) {
      var url = typeof input === 'string' ? input
              : (input && typeof input === 'object' && input.url) ? input.url : '';
      if (url) {
        var u = url.toLowerCase();
        if (u.indexOf('.m3u8') !== -1 || u.indexOf('.mpd') !== -1)
          send(url.startsWith('http') ? url : new URL(url, window.location.href).href);
      }
      return origFetch.apply(this, arguments);
    };
  }

  // JW Player 8 config extractor
  function extractJWConfig(config) {
    if (!config || typeof config !== 'object') return;
    if (typeof config.file === 'string') send(config.file);
    if (Array.isArray(config.sources))
      config.sources.forEach(function (s) { if (s && typeof s.file === 'string') send(s.file); });
    if (Array.isArray(config.playlist))
      config.playlist.forEach(function (item) {
        if (!item) return;
        if (typeof item.file === 'string') send(item.file);
        if (Array.isArray(item.sources))
          item.sources.forEach(function (s) { if (s && typeof s.file === 'string') send(s.file); });
      });
  }
  window.__crowExtractJWConfig = extractJWConfig;

  // JW Player wrapper — intercepts setup() and ready event
  function wrapJWPlayer(jw) {
    if (!jw || typeof jw !== 'function' || jw.__crowWrapped) return jw;
    var wrapped = function () {
      var inst = jw.apply(this, arguments);
      if (inst && typeof inst === 'object') {
        if (typeof inst.setup === 'function') {
          var origSetup = inst.setup;
          inst.setup = function (config) {
            extractJWConfig(config);
            return origSetup.call(this, config);
          };
        }
        try {
          inst.on && inst.on('ready', function () {
            try { var item = inst.getPlaylistItem && inst.getPlaylistItem(); if (item) extractJWConfig(item); } catch (_) {}
          });
          inst.on && inst.on('playlistItem', function () {
            try { var item = inst.getPlaylistItem && inst.getPlaylistItem(); if (item) extractJWConfig(item); } catch (_) {}
          });
        } catch (_) {}
      }
      return inst;
    };
    wrapped.__crowWrapped = true;
    try { Object.keys(jw).forEach(function (k) { try { wrapped[k] = jw[k]; } catch (_) {} }); } catch (_) {}
    return wrapped;
  }
  window.__crowWrapJWPlayer = wrapJWPlayer;

  if (typeof window.jwplayer !== 'undefined') window.jwplayer = wrapJWPlayer(window.jwplayer);

  // Watch for JW Player loaded asynchronously
  try {
    var __jw = window.jwplayer;
    Object.defineProperty(window, 'jwplayer', {
      configurable: true, enumerable: true,
      get: function () { return __jw; },
      set: function (v) { __jw = wrapJWPlayer(v); }
    });
  } catch (_) {}

  // Video.js
  function wrapVideoJS (vjs) {
    if (!vjs || vjs.__crowWrapped) return vjs;
    var orig = vjs;
    var wrapped = function () {
      var p = orig.apply(this, arguments);
      if (p && typeof p.on === 'function') {
        p.on('loadeddata', function () { try { var s = p.currentSrc(); if (s) send(s); } catch (_) {} });
        p.on('sourceset',  function (e) { try { if (e && e.src) send(e.src); } catch (_) {} });
      }
      return p;
    };
    try { Object.keys(orig).forEach(function (k) { try { wrapped[k] = orig[k]; } catch (_) {} }); } catch (_) {}
    wrapped.__crowWrapped = true;
    return wrapped;
  }
  try {
    var __vjs = window.videojs;
    Object.defineProperty(window, 'videojs', { configurable: true, enumerable: true,
      get: function () { return __vjs; }, set: function (v) { __vjs = wrapVideoJS(v); } });
    if (typeof window.videojs !== 'undefined') window.videojs = wrapVideoJS(window.videojs);
  } catch (_) {}

  // HLS.js
  function patchHLSClass (Hls) {
    if (!Hls || !Hls.prototype || Hls.prototype.__crowPatched) return Hls;
    var origLoad = Hls.prototype.loadSource;
    if (origLoad) Hls.prototype.loadSource = function (src) {
      if (typeof src === 'string') send(src); return origLoad.apply(this, arguments);
    };
    Hls.prototype.__crowPatched = true;
    return Hls;
  }
  try {
    var __Hls = window.Hls;
    Object.defineProperty(window, 'Hls', { configurable: true, enumerable: true,
      get: function () { return __Hls; }, set: function (v) { __Hls = patchHLSClass(v); } });
    if (typeof window.Hls !== 'undefined') patchHLSClass(window.Hls);
  } catch (_) {}

  // Shaka Player
  function patchShaka (shaka) {
    if (!shaka || !shaka.Player || !shaka.Player.prototype || shaka.Player.prototype.__crowPatched) return;
    var origLoad = shaka.Player.prototype.load;
    if (origLoad) shaka.Player.prototype.load = function (uri) {
      if (typeof uri === 'string') send(uri); return origLoad.apply(this, arguments);
    };
    shaka.Player.prototype.__crowPatched = true;
  }
  try {
    var __shaka = window.shaka;
    Object.defineProperty(window, 'shaka', { configurable: true, enumerable: true,
      get: function () { return __shaka; }, set: function (v) { __shaka = v; patchShaka(v); } });
    if (typeof window.shaka !== 'undefined') patchShaka(window.shaka);
  } catch (_) {}

  // dash.js
  function patchDashJS (dashjs) {
    if (!dashjs || !dashjs.MediaPlayer || dashjs.MediaPlayer.__crowWrapped) return;
    var factory = dashjs.MediaPlayer;
    dashjs.MediaPlayer = function () {
      var player = factory.apply(this, arguments);
      if (player && typeof player.initialize === 'function') {
        var origInit = player.initialize.bind(player);
        player.initialize = function (view, src) {
          if (typeof src === 'string') send(src); return origInit.apply(this, arguments);
        };
      }
      return player;
    };
    dashjs.MediaPlayer.__crowWrapped = true;
  }
  try {
    var __dashjs = window.dashjs;
    Object.defineProperty(window, 'dashjs', { configurable: true, enumerable: true,
      get: function () { return __dashjs; }, set: function (v) { __dashjs = v; patchDashJS(v); } });
    if (typeof window.dashjs !== 'undefined') patchDashJS(window.dashjs);
  } catch (_) {}
})();
""";

    // Injected at END — scans DOM for videos and known video-hoster embeds
    private const string VIDEO_SCAN_JS = """
(function () {
  if (window.__crowScanInstalled) return;
  window.__crowScanInstalled = true;

  var send = window.__crowSend || function () {};
  var extractJWConfig = window.__crowExtractJWConfig || function () {};

  var HOSTER_PATTERNS = [
    'filemoon', 'streamtape', 'doodstream', 'mixdrop', 'upstream',
    'vidcloud', 'vidstreaming', 'goload', 'fembed', 'fplayer',
    'mp4upload', 'voe.sx', 'sbplay', 'sbvideo', 'vidhide', 'vidmoly',
    'streamlare', 'streamvid', 'streamzz', 'filelions', 'moonplayer',
    'emturbovid', 'turbovid', 'embedsito', 'jetload', 'vidoza',
    'uqload', 'supervideo', 'streamwish', 'wishfast', 'speedostream',
    'filebee', 'streamsb', 'watchsb', 'sbthe', 'sblongvu',
    'sendvid', 'okru', 'ok.ru', 'dailymotion', 'rumble',
    'vidlox', 'vidhd', 'vidfast', 'vidbin', 'vidplay', 'hotlinking',
  ];

  function isKnownHoster(url) {
    var lo = url.toLowerCase();
    for (var i = 0; i < HOSTER_PATTERNS.length; i++)
      if (lo.indexOf(HOSTER_PATTERNS[i]) !== -1) return true;
    return false;
  }

  function scanVideos() {
    document.querySelectorAll('video').forEach(function (v) {
      var src = v.currentSrc || v.src || '';
      if (src && !src.startsWith('blob:') && !src.startsWith('data:')) {
        send(src);
      } else {
        // blob or no src means a streaming player — report page URL for yt-dlp
        if (document.querySelector('video[src], video > source[src]') || src.startsWith('blob:'))
          send(window.location.href);
      }
      v.querySelectorAll('source').forEach(function (s) {
        if (s.src && !s.src.startsWith('blob:') && !s.src.startsWith('data:')) send(s.src);
      });
    });
  }

  // Parent-side: detect video-hosting iframes
  // Reports if: (a) URL matches known hoster, OR (b) iframe has fullscreen/autoplay
  // permission — virtually every real video embed has allowfullscreen.
  function scanIframes() {
    document.querySelectorAll('iframe[src], iframe[data-src]').forEach(function (iframe) {
      var src = (iframe.getAttribute('src') || iframe.getAttribute('data-src') || '').trim();
      if (!src || src === 'about:blank' || src.startsWith('javascript:') || src.startsWith('data:')) return;
      if (isKnownHoster(src)) { send(src); return; }
      var allow = (iframe.getAttribute('allow') || '').toLowerCase();
      var fsAttr = iframe.hasAttribute('allowfullscreen') || iframe.hasAttribute('allowFullScreen');
      var hasFull = fsAttr || allow.indexOf('fullscreen') !== -1;
      var hasAuto = allow.indexOf('autoplay') !== -1;
      if (hasFull || hasAuto) send(src);
    });
  }

  // Child-side: if WE are inside an iframe and have video content, report our own URL.
  // This fires for any video iframe regardless of domain — no whitelist needed.
  function reportSelfIfVideoFrame() {
    var inIframe = false;
    try { inIframe = window.self !== window.top; } catch (_) { inIframe = true; }
    if (!inIframe) return;
    var hasVideo = !!document.querySelector('video');
    var hasJWP   = typeof window.jwplayer !== 'undefined';
    if (hasVideo || hasJWP) send(window.location.href);
  }

  // If we are directly on a known video-hosting page, also offer the page URL
  // so yt-dlp's dedicated extractor can handle auth/tokens properly.
  function reportPageIfKnownHoster() {
    if (isKnownHoster(window.location.href)) send(window.location.href);
  }

  // Scan inline <script> tags for m3u8 / mp4 / mpd URL literals
  function scanScripts() {
    var re = /https?:\/\/[^\s"'`\\<>\]}\)]+\.(?:m3u8|mpd|mp4|webm|mkv|mov)(?:[?#][^\s"'`\\<>\]}\)]*)?/g;
    document.querySelectorAll('script:not([src])').forEach(function (s) {
      var text = s.textContent || '';
      var m;
      while ((m = re.exec(text)) !== null) send(m[0]);
      re.lastIndex = 0;
    });
  }

  function probeJWPlayer() {
    if (typeof window.jwplayer !== 'function') return;
    try {
      var inst = window.jwplayer();
      if (!inst) return;
      try { var item = inst.getPlaylistItem && inst.getPlaylistItem(); if (item) extractJWConfig(item); } catch (_) {}
      try { var list = inst.getPlaylist && inst.getPlaylist(); if (Array.isArray(list)) list.forEach(function (it) { extractJWConfig(it); }); } catch (_) {}
    } catch (_) {}
  }

  function probeVideoJS() {
    if (typeof window.videojs !== 'function') return;
    try {
      var all = window.videojs.getAllPlayers ? window.videojs.getAllPlayers() : [];
      all.forEach(function (p) { try { var s = p.currentSrc(); if (s) send(s); } catch (_) {} });
    } catch (_) {}
  }

  function fullScan() {
    scanVideos();
    scanIframes();
    scanScripts();
    reportSelfIfVideoFrame();
    reportPageIfKnownHoster();
    probeJWPlayer();
    probeVideoJS();
  }

  fullScan();

  // Poll for players that initialise after our scan (async loaders)
  var polls = 0;
  function pollJW() { probeJWPlayer(); probeVideoJS(); reportSelfIfVideoFrame(); if (++polls < 8) setTimeout(pollJW, 700); }
  setTimeout(pollJW, 400);

  window.addEventListener('load', fullScan);

  var observer = new MutationObserver(function (mutations) {
    var needVideo = false, needIframe = false;
    for (var i = 0; i < mutations.length; i++) {
      var nodes = mutations[i].addedNodes;
      for (var j = 0; j < nodes.length; j++) {
        var n = nodes[j];
        if (n.nodeType !== 1) continue;
        if (n.tagName === 'VIDEO' || (n.querySelector && n.querySelector('video'))) needVideo = true;
        if (n.tagName === 'IFRAME' || (n.querySelector && n.querySelector('iframe'))) needIframe = true;
      }
    }
    if (needVideo) scanVideos();
    if (needIframe) { scanIframes(); setTimeout(probeJWPlayer, 600); setTimeout(probeVideoJS, 600); setTimeout(reportSelfIfVideoFrame, 600); }
  });
  observer.observe(document.documentElement, { childList: true, subtree: true });
})();
""";

    public class WebTab : Gtk.Box {

        private static WebKit.NetworkSession? _shared_session = null;

        public static unowned WebKit.NetworkSession get_shared_session () {
            if (_shared_session == null) {
                string data_dir = GLib.Path.build_filename (
                    GLib.Environment.get_user_data_dir (), "crow-browser"
                );
                string cache_dir = GLib.Path.build_filename (
                    GLib.Environment.get_user_cache_dir (), "crow-browser"
                );
                GLib.DirUtils.create_with_parents (data_dir, 0755);
                GLib.DirUtils.create_with_parents (cache_dir, 0755);
                _shared_session = new WebKit.NetworkSession (data_dir, cache_dir);
                // Without explicit persistent storage, WebKit stores cookies only in memory.
                var cm = _shared_session.get_cookie_manager ();
                cm.set_accept_policy (WebKit.CookieAcceptPolicy.ALWAYS);
                cm.set_persistent_storage (
                    GLib.Path.build_filename (data_dir, "cookies.db"),
                    WebKit.CookiePersistentStorage.SQLITE
                );
            }
            return _shared_session;
        }

        // Set at construction time to use a private/ephemeral network session
        public WebKit.NetworkSession? tab_network_session { get; construct; default = null; }

        public WebKit.WebView web_view { get; private set; }

        public string tab_title { get; private set; default = "Nová karta"; }
        public string tab_uri { get; private set; default = "about:blank"; }
        public bool loading { get; private set; default = false; }
        public double load_progress { get; private set; default = 0.0; }
        public bool nav_can_go_back { get; private set; default = false; }
        public bool nav_can_go_forward { get; private set; default = false; }

        // Detected video URLs on current page (cleared on navigation)
        public Gee.ArrayList<string> detected_video_urls { get; private set; }

        public signal void title_changed (string title);
        public signal void uri_changed (string uri);
        public signal void loading_changed (bool is_loading, double progress);
        public signal void favicon_changed (Gdk.Texture? favicon);
        public signal void drm_blocked ();
        public signal void video_detected (string url);
        public signal void fullscreen_entered ();
        public signal void fullscreen_left ();

        construct {
            orientation = Gtk.Orientation.VERTICAL;
            spacing = 0;
            detected_video_urls = new Gee.ArrayList<string> ();

            var settings = new WebKit.Settings ();
            settings.enable_javascript = true;
            settings.enable_media = true;
            settings.enable_mediasource = true;
            settings.enable_html5_local_storage = true;
            settings.hardware_acceleration_policy = WebKit.HardwareAccelerationPolicy.ALWAYS;
            settings.enable_smooth_scrolling = true;
            settings.enable_encrypted_media = true;
            settings.enable_developer_extras = true;
            // Chrome user agent — needed for DRM sites and better compatibility
            settings.set_user_agent (
                "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 " +
                "(KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36"
            );

            if (tab_network_session != null) {
                web_view = (WebKit.WebView) GLib.Object.new (typeof (WebKit.WebView),
                    "network-session", tab_network_session);
            } else {
                web_view = (WebKit.WebView) GLib.Object.new (typeof (WebKit.WebView),
                    "network-session", get_shared_session ());
            }
            web_view.settings = settings;
            web_view.vexpand = true;
            web_view.hexpand = true;
            append (web_view);

            // ── Video detection via JS message handler ─────────────────────
            var ucm = web_view.get_user_content_manager ();
            ucm.register_script_message_handler ("crowVideoDetected", "");
            ucm.script_message_received["crowVideoDetected"].connect ((jsc_val) => {
                if (!jsc_val.is_string ()) return;
                string url = jsc_val.to_string ();
                if (url.length == 0) return;
                if (!detected_video_urls.contains (url)) {
                    detected_video_urls.add (url);
                    video_detected (url);
                }
            });

            // Hook script at START catches JW Player setup() before page scripts run
            var hook_script = new WebKit.UserScript (
                VIDEO_HOOK_JS,
                WebKit.UserContentInjectedFrames.ALL_FRAMES,
                WebKit.UserScriptInjectionTime.START,
                null, null
            );
            ucm.add_script (hook_script);

            // Scan script at END scans the DOM and known iframe embeds
            var scan_script = new WebKit.UserScript (
                VIDEO_SCAN_JS,
                WebKit.UserContentInjectedFrames.ALL_FRAMES,
                WebKit.UserScriptInjectionTime.END,
                null, null
            );
            ucm.add_script (scan_script);

            // ── Ad blocking ────────────────────────────────────────────────
            AdBlockManager.get_instance ().apply_to (ucm);

            // ── Extensions ─────────────────────────────────────────────────
            ExtensionManager.get_instance ().apply_to (ucm);

            // ── Page signals ───────────────────────────────────────────────
            web_view.notify["title"].connect (() => {
                var t = web_view.title;
                tab_title = (t != null && t.length > 0) ? t : "Nová karta";
                title_changed (tab_title);
            });

            web_view.notify["uri"].connect (() => {
                tab_uri = web_view.uri ?? "about:blank";
                uri_changed (tab_uri);
            });

            web_view.notify["estimated-load-progress"].connect (() => {
                load_progress = web_view.estimated_load_progress;
                loading_changed (loading, load_progress);
            });

            web_view.notify["favicon"].connect (() => {
                favicon_changed (web_view.favicon);
            });

            web_view.load_changed.connect ((event) => {
                switch (event) {
                    case WebKit.LoadEvent.STARTED:
                        loading = true;
                        load_progress = 0.05;
                        detected_video_urls.clear ();  // reset on new navigation
                        break;
                    case WebKit.LoadEvent.REDIRECTED:
                        loading = true;
                        load_progress = 0.1;
                        break;
                    case WebKit.LoadEvent.COMMITTED:
                        loading = true;
                        break;
                    case WebKit.LoadEvent.FINISHED:
                        loading = false;
                        load_progress = 1.0;
                        // Skip history for private/tor tabs (tab_network_session is ephemeral)
                        if (tab_network_session == null &&
                            SettingsManager.get_instance ().get_save_history ())
                            HistoryManager.get_instance ().add (tab_uri, tab_title);
                        break;
                }
                nav_can_go_back = web_view.can_go_back ();
                nav_can_go_forward = web_view.can_go_forward ();
                loading_changed (loading, load_progress);
            });

            // ── DRM + device permissions ───────────────────────────────────
            web_view.permission_request.connect ((req) => {
                var sm_r = SettingsManager.get_instance ();
                if (req is WebKit.MediaKeySystemPermissionRequest) {
                    if (widevine_cdm_available ()) req.allow ();
                    else { req.deny (); drm_blocked (); }
                    return true;
                }
                if (req is WebKit.UserMediaPermissionRequest) {
                    var umreq = (WebKit.UserMediaPermissionRequest) req;
                    string perm = umreq.is_for_video_device
                        ? sm_r.get_camera_permission ()
                        : sm_r.get_microphone_permission ();
                    if (perm == "allow") req.allow (); else req.deny ();
                    return true;
                }
                if (req is WebKit.GeolocationPermissionRequest) {
                    if (sm_r.get_location_permission () == "allow") req.allow ();
                    else req.deny ();
                    return true;
                }
                if (req is WebKit.NotificationPermissionRequest) {
                    if (sm_r.get_notification_permission () == "allow") req.allow ();
                    else req.deny ();
                    return true;
                }
                return false;
            });

            // ── Fullscreen ─────────────────────────────────────────────────
            web_view.enter_fullscreen.connect (() => {
                fullscreen_entered ();
                return true;
            });
            web_view.leave_fullscreen.connect (() => {
                fullscreen_left ();
                return true;
            });

            // ── Navigation policy (HTTPS-only + unsupported MIME download) ──
            web_view.decide_policy.connect ((decision, type) => {
                if (type == WebKit.PolicyDecisionType.NAVIGATION_ACTION) {
                    if (SettingsManager.get_instance ().get_https_only ()) {
                        var nav = (WebKit.NavigationPolicyDecision) decision;
                        string req_uri = nav.navigation_action.get_request ().get_uri () ?? "";
                        if (req_uri.has_prefix ("http://") &&
                            !req_uri.has_prefix ("http://localhost") &&
                            !req_uri.has_prefix ("http://127.")) {
                            decision.ignore ();
                            web_view.load_uri ("https://" + req_uri.substring (7));
                            return true;
                        }
                    }
                }
                if (type == WebKit.PolicyDecisionType.RESPONSE) {
                    var resp = (WebKit.ResponsePolicyDecision) decision;
                    if (!resp.is_mime_type_supported ()) {
                        resp.download ();
                        return true;
                    }
                }
                return false;
            });

            web_view.get_network_session ().download_started.connect (on_download_started);

            // Apply persisted settings
            var sm = SettingsManager.get_instance ();
            settings.enable_javascript  = sm.get_javascript_enabled ();
            settings.hardware_acceleration_policy = sm.get_hardware_accel ()
                ? WebKit.HardwareAccelerationPolicy.ALWAYS
                : WebKit.HardwareAccelerationPolicy.NEVER;
            web_view.zoom_level = sm.get_default_zoom ();
            settings.enable_smooth_scrolling = sm.get_smooth_scrolling ();
            settings.default_font_size = (uint) sm.get_default_font_size ();
            settings.media_playback_requires_user_gesture = sm.get_block_autoplay ();

            // ── Network-level video stream detection ───────────────────────
            // Catches ANY player regardless of JS hooks — inspects MIME type
            // of every HTTP response as it starts loading.
            web_view.resource_load_started.connect (on_resource_load_started);
        }

        private void on_resource_load_started (WebKit.WebResource resource,
                                               WebKit.URIRequest  request) {
            string url = request.get_uri () ?? "";
            if (url.length == 0 || url.has_prefix ("data:") || url.has_prefix ("blob:")) return;

            string lo = url.down () ?? "";
            // Skip binary media segments — we only want manifests and full files
            if (lo.has_suffix (".ts")   || lo.contains (".ts?")   || lo.contains (".ts#")  ||
                lo.has_suffix (".m4s")  || lo.contains (".m4s?")  ||
                lo.has_suffix (".vtt")  || lo.has_suffix (".srt")  ||
                lo.has_suffix (".js")   || lo.has_suffix (".css")  ||
                lo.has_suffix (".jpg")  || lo.has_suffix (".png")  ||
                lo.has_suffix (".gif")  || lo.has_suffix (".svg")  ||
                lo.has_suffix (".woff") || lo.has_suffix (".woff2")) return;

            bool looks_like_stream = lo.contains (".m3u8") || lo.contains (".mpd");

            resource.notify["response"].connect (() => {
                var resp = resource.get_response ();
                if (resp == null) return;
                string mime = (resp.get_mime_type () ?? "").down () ?? "";
                bool is_video =
                    mime.has_prefix ("video/")              ||
                    mime == "application/x-mpegurl"         ||
                    mime == "application/vnd.apple.mpegurl" ||
                    mime == "application/dash+xml"          ||
                    mime == "application/x-dash+xml";

                if (!is_video && !looks_like_stream) return;

                string rurl = resp.get_uri () ?? url;
                string rlo  = rurl.down () ?? "";
                // Still skip segments even if MIME says video
                if (rlo.has_suffix (".ts")  || rlo.contains (".ts?") ||
                    rlo.has_suffix (".m4s") || rlo.contains (".m4s?")) return;

                if (!detected_video_urls.contains (rurl)) {
                    detected_video_urls.add (rurl);
                    video_detected (rurl);
                }
            });
        }

        // ── Widevine check ────────────────────────────────────────────────

        public static bool widevine_cdm_available () {
            string[] paths = {
                "/usr/lib/chromium/WidevineCdm",
                "/usr/lib64/chromium/WidevineCdm",
                "/usr/lib/chromium-browser/WidevineCdm",
                "/opt/google/chrome/WidevineCdm",
                "/opt/chromium/WidevineCdm",
                GLib.Path.build_filename (
                    GLib.Environment.get_home_dir (), ".local/lib/chromium/WidevineCdm"
                ),
            };
            foreach (var p in paths) {
                if (GLib.FileUtils.test (p, GLib.FileTest.IS_DIR)) return true;
                if (GLib.FileUtils.test (
                    GLib.Path.build_filename (p, "libwidevinecdm.so"),
                    GLib.FileTest.EXISTS)) return true;
            }
            string? env = GLib.Environment.get_variable ("WEBKIT_WEBKITGTK_CDM_SEARCH_PATH");
            return env != null && env.length > 0;
        }

        // ── File download ─────────────────────────────────────────────────

        private void on_download_started (WebKit.Download dl) {
            DownloadManager.get_instance ().track (dl);
            dl.decide_destination.connect ((suggested) => {
                string download_dir = GLib.Environment.get_user_special_dir (
                    GLib.UserDirectory.DOWNLOAD
                );
                string dest = GLib.Path.build_filename (download_dir, suggested);
                int counter = 1;
                string base_name = suggested;
                string ext = "";
                int dot = suggested.last_index_of_char ('.');
                if (dot >= 0) {
                    base_name = suggested.substring (0, dot);
                    ext = suggested.substring (dot);
                }
                while (GLib.FileUtils.test (dest, GLib.FileTest.EXISTS)) {
                    dest = GLib.Path.build_filename (
                        download_dir, @"$(base_name) ($(counter))$(ext)"
                    );
                    counter++;
                }
                dl.set_destination (dest);
                return true;
            });

            if (SettingsManager.get_instance ().get_open_after_download ()) {
                dl.finished.connect (() => {
                    string dest = dl.destination ?? "";
                    if (dest.length > 0) {
                        try {
                            GLib.AppInfo.launch_default_for_uri (
                                GLib.Filename.to_uri (dest, null), null
                            );
                        } catch {}
                    }
                });
            }
        }

        // ── Navigation ────────────────────────────────────────────────────

        public void load_url (string input) {
            web_view.load_uri (normalize_url (input));
        }

        public static string normalize_url (string input) {
            string s = input.strip ();
            if (s.has_prefix ("http://") || s.has_prefix ("https://") ||
                s.has_prefix ("file://") || s.has_prefix ("about:") ||
                s.has_prefix ("data:")) return s;
            if (looks_like_domain (s)) return "https://" + s;
            return SettingsManager.get_instance ().get_search_url (s);
        }

        private static bool looks_like_domain (string s) {
            if (s.contains (" ")) return false;
            if (s.has_prefix ("localhost")) return true;
            int dot = s.index_of_char ('.');
            if (dot <= 0) return false;
            string after = s.substring (dot + 1).split ("/")[0].split (":")[0];
            return after.length >= 2 && !after.contains (".");
        }

        public void go_back ()    { web_view.go_back (); }
        public void go_forward () { web_view.go_forward (); }
        public void reload ()     { if (loading) web_view.stop_loading (); else web_view.reload (); }
        public void stop ()       { web_view.stop_loading (); }

        public void open_inspector () {
            web_view.get_inspector ().show ();
        }
    }
}
