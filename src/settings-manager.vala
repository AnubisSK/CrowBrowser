namespace CrowBrowser {

    public class SettingsManager : Object {

        private static SettingsManager? _instance = null;

        public static SettingsManager get_instance () {
            if (_instance == null) _instance = new SettingsManager ();
            return _instance;
        }

        private GLib.KeyFile kf;
        private string config_file;

        construct {
            kf = new GLib.KeyFile ();
            string dir = GLib.Path.build_filename (
                GLib.Environment.get_user_config_dir (), "crow-browser"
            );
            GLib.DirUtils.create_with_parents (dir, 0755);
            config_file = GLib.Path.build_filename (dir, "settings.ini");
            try { kf.load_from_file (config_file, GLib.KeyFileFlags.NONE); } catch {}
        }

        // ── General ───────────────────────────────────────────────────────

        public string get_homepage () {
            try { return kf.get_string ("general", "homepage"); } catch { return ""; }
        }
        public void set_homepage (string v) {
            kf.set_string ("general", "homepage", v); save ();
        }

        public string get_search_engine () {
            try { return kf.get_string ("general", "search_engine"); }
            catch { return "duckduckgo"; }
        }
        public void set_search_engine (string v) {
            kf.set_string ("general", "search_engine", v); save ();
        }

        public string get_startup_behavior () {
            try { return kf.get_string ("general", "startup"); } catch { return "newtab"; }
        }
        public void set_startup_behavior (string v) {
            kf.set_string ("general", "startup", v); save ();
        }

        public bool get_smooth_scrolling () {
            try { return kf.get_boolean ("general", "smooth_scrolling"); } catch { return true; }
        }
        public void set_smooth_scrolling (bool v) {
            kf.set_boolean ("general", "smooth_scrolling", v); save ();
        }

        public string get_search_url (string query) {
            string q = GLib.Uri.escape_string (query, null, true);
            switch (get_search_engine ()) {
                case "google":    return "https://www.google.com/search?q=" + q;
                case "bing":      return "https://www.bing.com/search?q=" + q;
                case "yahoo":     return "https://search.yahoo.com/search?p=" + q;
                case "ecosia":    return "https://www.ecosia.org/search?method=index&q=" + q;
                case "startpage": return "https://www.startpage.com/search?q=" + q;
                default:          return "https://duckduckgo.com/?q=" + q;
            }
        }

        // ── Appearance ────────────────────────────────────────────────────

        public double get_default_zoom () {
            try { return kf.get_double ("appearance", "zoom"); } catch { return 1.0; }
        }
        public void set_default_zoom (double v) {
            kf.set_double ("appearance", "zoom", v); save ();
        }

        public int get_default_font_size () {
            try { return kf.get_integer ("appearance", "font_size"); } catch { return 16; }
        }
        public void set_default_font_size (int v) {
            kf.set_integer ("appearance", "font_size", v); save ();
        }

        // ── Downloads ─────────────────────────────────────────────────────

        public string get_download_folder () {
            try { return kf.get_string ("downloads", "folder"); }
            catch { return GLib.Environment.get_user_special_dir (GLib.UserDirectory.DOWNLOAD); }
        }
        public void set_download_folder (string v) {
            kf.set_string ("downloads", "folder", v); save ();
        }

        public bool get_ask_download_location () {
            try { return kf.get_boolean ("downloads", "ask_location"); } catch { return false; }
        }
        public void set_ask_download_location (bool v) {
            kf.set_boolean ("downloads", "ask_location", v); save ();
        }

        public bool get_open_after_download () {
            try { return kf.get_boolean ("downloads", "open_after"); } catch { return false; }
        }
        public void set_open_after_download (bool v) {
            kf.set_boolean ("downloads", "open_after", v); save ();
        }

        // ── Privacy ───────────────────────────────────────────────────────

        public bool get_javascript_enabled () {
            try { return kf.get_boolean ("privacy", "javascript"); } catch { return true; }
        }
        public void set_javascript_enabled (bool v) {
            kf.set_boolean ("privacy", "javascript", v); save ();
        }

        public bool get_hardware_accel () {
            try { return kf.get_boolean ("privacy", "hw_accel"); } catch { return true; }
        }
        public void set_hardware_accel (bool v) {
            kf.set_boolean ("privacy", "hw_accel", v); save ();
        }

        public bool get_adblock_enabled () {
            try { return kf.get_boolean ("privacy", "adblock"); } catch { return true; }
        }
        public void set_adblock_enabled (bool v) {
            kf.set_boolean ("privacy", "adblock", v); save ();
        }

        public bool get_https_only () {
            try { return kf.get_boolean ("privacy", "https_only"); } catch { return false; }
        }
        public void set_https_only (bool v) {
            kf.set_boolean ("privacy", "https_only", v); save ();
        }

        public bool get_save_history () {
            try { return kf.get_boolean ("privacy", "save_history"); } catch { return true; }
        }
        public void set_save_history (bool v) {
            kf.set_boolean ("privacy", "save_history", v); save ();
        }

        public bool get_block_autoplay () {
            try { return kf.get_boolean ("privacy", "block_autoplay"); } catch { return false; }
        }
        public void set_block_autoplay (bool v) {
            kf.set_boolean ("privacy", "block_autoplay", v); save ();
        }

        // ── Permissions ───────────────────────────────────────────────────

        public string get_camera_permission () {
            try { return kf.get_string ("permissions", "camera"); } catch { return "ask"; }
        }
        public void set_camera_permission (string v) {
            kf.set_string ("permissions", "camera", v); save ();
        }

        public string get_microphone_permission () {
            try { return kf.get_string ("permissions", "microphone"); } catch { return "ask"; }
        }
        public void set_microphone_permission (string v) {
            kf.set_string ("permissions", "microphone", v); save ();
        }

        public string get_location_permission () {
            try { return kf.get_string ("permissions", "location"); } catch { return "ask"; }
        }
        public void set_location_permission (string v) {
            kf.set_string ("permissions", "location", v); save ();
        }

        public string get_notification_permission () {
            try { return kf.get_string ("permissions", "notification"); } catch { return "ask"; }
        }
        public void set_notification_permission (string v) {
            kf.set_string ("permissions", "notification", v); save ();
        }

        // ── Extensions ────────────────────────────────────────────────────

        public bool get_extension_enabled (string id) {
            try { return kf.get_boolean ("extensions", id); } catch { return true; }
        }
        public void set_extension_enabled (string id, bool v) {
            kf.set_boolean ("extensions", id, v); save ();
        }

        // ─────────────────────────────────────────────────────────────────

        private void save () {
            try { kf.save_to_file (config_file); } catch {}
        }
    }
}
