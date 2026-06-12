namespace CrowBrowser {

    public class Application : Adw.Application {

        private string session_file = "";
        private bool session_consumed = false;

        public Application () {
            Object (
                application_id: "org.crowbrowser.CrowBrowser",
                flags: GLib.ApplicationFlags.DEFAULT_FLAGS
            );
        }

        // Called by BrowserWindow on first startup to get saved tab URLs.
        // Returns an empty array after the first call so subsequent windows
        // don't attempt to restore the same session.
        public string[] consume_session_urls () {
            if (session_consumed || session_file == "") return new string[0];
            session_consumed = true;
            string content = "";
            try { GLib.FileUtils.get_contents (session_file, out content, null); }
            catch { return new string[0]; }
            var urls = new Gee.ArrayList<string> ();
            foreach (string line in content.strip ().split ("\n")) {
                string url = line.strip ();
                if (url.length > 0) urls.add (url);
            }
            return urls.to_array ();
        }

        protected override void shutdown () {
            HistoryManager.get_instance ().save_now ();
            save_session ();
            base.shutdown ();
        }

        private void save_session () {
            if (session_file == "") return;
            var sb = new GLib.StringBuilder ();
            foreach (var win in get_windows ()) {
                var bw = win as BrowserWindow;
                if (bw != null) {
                    foreach (string url in bw.get_open_urls ())
                        sb.append (url + "\n");
                }
            }
            try { GLib.FileUtils.set_contents (session_file, sb.str); } catch {}
        }

        protected override void activate () {
            var win = get_active_window ();
            if (win == null) {
                win = new BrowserWindow (this);
            }
            win.present ();
        }

        protected override void startup () {
            base.startup ();
            Adw.init ();

            // Force dark mode – Arc is always dark
            Adw.StyleManager.get_default ().color_scheme = Adw.ColorScheme.FORCE_DARK;

            // Register bundled icons so crow-video-dl-symbolic is findable by name
            Gtk.IconTheme.get_for_display (Gdk.Display.get_default ())
                .add_resource_path ("/org/crowbrowser/CrowBrowser/icons");

            // Eagerly init singletons so they load from disk before first window
            SettingsManager.get_instance ();
            HistoryManager.get_instance ();
            DownloadManager.get_instance ();
            AdBlockManager.get_instance ();

            string data_dir = GLib.Path.build_filename (
                GLib.Environment.get_user_data_dir (), "crow-browser"
            );
            session_file = GLib.Path.build_filename (data_dir, "session");

            setup_widevine_cdm_path ();
            setup_actions ();
            load_styles ();
        }

        // Point WebKit to Widevine CDM if installed
        private void setup_widevine_cdm_path () {
            string[] cdm_dirs = {
                "/usr/lib/chromium/WidevineCdm",
                "/usr/lib64/chromium/WidevineCdm",
                "/usr/lib/chromium-browser/WidevineCdm",
                "/opt/google/chrome/WidevineCdm",
                "/opt/chromium/WidevineCdm",
                GLib.Path.build_filename (
                    GLib.Environment.get_home_dir (), ".local/lib/chromium/WidevineCdm"
                ),
            };

            var found = new GLib.StringBuilder ();
            foreach (var dir in cdm_dirs) {
                if (GLib.FileUtils.test (dir, GLib.FileTest.IS_DIR)) {
                    if (found.len > 0) found.append (":");
                    found.append (dir);
                }
            }

            if (found.len > 0) {
                // WEBKIT_WEBKITGTK_CDM_SEARCH_PATH tells WebKit where to look for CDMs
                GLib.Environment.set_variable (
                    "WEBKIT_WEBKITGTK_CDM_SEARCH_PATH", found.str, true
                );
            }
        }

        private void setup_actions () {
            var new_window = new GLib.SimpleAction ("new-window", null);
            new_window.activate.connect (() => {
                var win = new BrowserWindow (this);
                win.present ();
            });
            add_action (new_window);

            var new_priv = new GLib.SimpleAction ("new-private-window", null);
            new_priv.activate.connect (() => {
                var win = new BrowserWindow.with_mode (this, "private");
                win.present ();
            });
            add_action (new_priv);

            var new_tor = new GLib.SimpleAction ("new-tor-window", null);
            new_tor.activate.connect (() => {
                var win = new BrowserWindow.with_mode (this, "tor");
                win.present ();
            });
            add_action (new_tor);

            var quit = new GLib.SimpleAction ("quit", null);
            quit.activate.connect (() => this.quit ());
            add_action (quit);
            set_accels_for_action ("app.quit", {"<primary>q"});
        }

        private void load_styles () {
            var css = new Gtk.CssProvider ();
            css.load_from_string ("""
/* ═══════════════════════════════════════════════════════
   CrowBrowser – Arc.net inspired dark theme
   ═══════════════════════════════════════════════════════ */

/* ── Full-width header bar ────────────────────────────── */
headerbar.crow-main-hbar {
    background: #0f0e18;
    box-shadow: none;
    border-bottom: 1px solid rgba(255,255,255,0.05);
    padding: 0 6px;
    min-height: 44px;
}

headerbar.crow-main-hbar:backdrop {
    background: #0f0e18;
}

/* ── Sidebar panel (tab list, no own header) ──────────── */
.crow-sidebar-panel {
    background: #0e0d16;
    border-right: 1px solid rgba(255,255,255,0.05);
}

/* ── Tab rows in sidebar ──────────────────────────────── */
.crow-tab-row {
    border-radius: 10px;
    margin: 2px 8px;
    padding: 7px 4px 7px 10px;
    transition: background-color 130ms ease;
    min-height: 38px;
}

.crow-tab-row:hover {
    background: rgba(255,255,255,0.055);
}

.crow-tab-row.active-tab {
    background: rgba(108,85,245,0.20);
    box-shadow: inset 0 0 0 1px rgba(124,92,245,0.28);
}

.tab-title {
    color: rgba(255,255,255,0.80);
    font-size: 0.85rem;
}

.crow-tab-row.active-tab .tab-title {
    color: rgba(255,255,255,0.96);
    font-weight: 500;
}

.tab-close-btn {
    background: transparent;
    border: none;
    box-shadow: none;
    border-radius: 6px;
    padding: 3px;
    min-width: 22px;
    min-height: 22px;
    color: rgba(255,255,255,0.40);
    opacity: 0;
    transition: opacity 120ms, background-color 100ms;
}

.crow-tab-row:hover .tab-close-btn,
.crow-tab-row.active-tab .tab-close-btn {
    opacity: 1;
}

.tab-close-btn:hover {
    background: rgba(255,255,255,0.12);
    color: rgba(255,255,255,0.90);
}

/* ── Sidebar bottom bar ────────────────────────────────── */
.crow-sidebar-bottom {
    background: #0e0d16;
    border-top: 1px solid rgba(255,255,255,0.05);
    padding: 8px;
}

.crow-sidebar-action {
    background: transparent;
    border: none;
    box-shadow: none;
    border-radius: 9px;
    color: rgba(255,255,255,0.50);
    min-width: 34px;
    min-height: 34px;
    padding: 5px;
    transition: background-color 120ms, color 120ms;
}

.crow-sidebar-action:hover {
    background: rgba(255,255,255,0.07);
    color: rgba(255,255,255,0.88);
}

/* ── Right content area ────────────────────────────────── */
.crow-right-area {
    background: #17151f;
}

/* URL toolbar strip */
.crow-url-toolbar {
    background: #17151f;
    border-bottom: 1px solid rgba(255,255,255,0.05);
    padding: 8px 14px;
    min-height: 52px;
}

/* Nav buttons */
.nav-button {
    background: transparent;
    border: none;
    box-shadow: none;
    border-radius: 8px;
    color: rgba(255,255,255,0.52);
    min-width: 34px;
    min-height: 34px;
    padding: 5px;
    transition: background-color 120ms, color 120ms;
}

.nav-button:hover {
    background: rgba(255,255,255,0.07);
    color: rgba(255,255,255,0.90);
}

.nav-button:disabled {
    opacity: 0.28;
}

/* URL entry */
entry.crow-url-entry {
    background: rgba(255,255,255,0.065);
    border: 1px solid rgba(255,255,255,0.09);
    border-radius: 22px;
    color: rgba(255,255,255,0.90);
    caret-color: #7c5cf5;
    padding: 0 16px;
    font-size: 0.88rem;
    box-shadow: none;
    min-height: 36px;
    transition: border-color 150ms, background-color 150ms;
}

entry.crow-url-entry:focus {
    background: rgba(255,255,255,0.09);
    border-color: rgba(124,92,245,0.60);
    box-shadow: 0 0 0 3px rgba(124,92,245,0.14);
    outline: none;
}

entry.crow-url-entry text { color: rgba(255,255,255,0.90); }
entry.crow-url-entry image { color: rgba(255,255,255,0.35); }
entry.crow-url-entry:focus image { color: rgba(124,92,245,0.80); }

/* Action buttons (right side of toolbar) */
.crow-action-btn {
    background: transparent;
    border: none;
    box-shadow: none;
    border-radius: 8px;
    color: rgba(255,255,255,0.50);
    min-width: 34px;
    min-height: 34px;
    padding: 5px;
    transition: background-color 120ms, color 120ms;
}

.crow-action-btn:hover {
    background: rgba(255,255,255,0.07);
    color: rgba(255,255,255,0.88);
}

/* ── Progress bar ──────────────────────────────────────── */
progressbar.crow-progress trough {
    background: transparent;
    border: none;
    min-height: 2px;
    border-radius: 0;
}

progressbar.crow-progress trough progress {
    background: linear-gradient(90deg, #7c5cf5, #b8abff);
    min-height: 2px;
    border-radius: 0;
}

/* ── Scrollbar in sidebar ──────────────────────────────── */
.crow-tabs-scroll scrollbar {
    background: transparent;
    min-width: 4px;
}

.crow-tabs-scroll scrollbar slider {
    background: rgba(255,255,255,0.12);
    border-radius: 4px;
    min-width: 4px;
    margin: 2px;
}

.crow-tabs-scroll scrollbar slider:hover {
    background: rgba(255,255,255,0.22);
}

/* ── Private browsing bar ────────────────────────────────── */
.crow-private-bar {
    background: rgba(108, 85, 245, 0.12);
    border-bottom: 1px solid rgba(108, 85, 245, 0.22);
    color: rgba(196, 181, 253, 0.90);
    min-height: 26px;
}

/* ── Tor browsing bar ────────────────────────────────────── */
.crow-tor-bar {
    background: rgba(34, 160, 90, 0.13);
    border-bottom: 1px solid rgba(34, 200, 100, 0.20);
    color: rgba(80, 220, 150, 0.90);
    min-height: 26px;
}

/* ── DRM warning button ────────────────────────────────── */
.crow-sidebar-action-warn {
    color: #f0a500;
}

.crow-sidebar-action-warn:hover {
    color: #f5c842;
    background: rgba(240,165,0,0.10);
}

/* ── New-tab hint in empty sidebar ─────────────────────── */
.crow-new-tab-hint {
    color: rgba(255,255,255,0.22);
    font-size: 0.82rem;
}

/* ── Video type badge in downloader list ────────────────── */
.crow-video-badge {
    background: rgba(124,92,245,0.18);
    color: #b8abff;
    border-radius: 6px;
    padding: 2px 8px;
    font-size: 0.75rem;
    font-weight: 600;
    letter-spacing: 0.3px;
}

/* ── Download button — active (video detected) ──────────── */
.crow-action-btn.crow-dl-active {
    color: #7c5cf5;
}

.crow-action-btn.crow-dl-active:hover {
    background: rgba(124,92,245,0.14);
    color: #b8abff;
}

/* ── Video count badge on download button ───────────────── */
.crow-dl-badge {
    background: #7c5cf5;
    color: #fff;
    border-radius: 8px;
    font-size: 0.60rem;
    font-weight: 700;
    min-width: 14px;
    min-height: 14px;
    padding: 0 3px;
    margin: 3px 1px;
}
""");
            Gtk.StyleContext.add_provider_for_display (
                Gdk.Display.get_default (),
                css,
                Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION
            );
        }
    }
}
