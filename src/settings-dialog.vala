namespace CrowBrowser {

    private delegate void PermSetter (string v);

    public class SettingsDialog : Adw.PreferencesDialog {

        private SettingsManager sm;

        public SettingsDialog () {
            sm = SettingsManager.get_instance ();
            title = "Nastavenia";
            search_enabled = true;
            build_pages ();
        }

        private void build_pages () {
            add (build_general_page ());
            add (build_appearance_page ());
            add (build_downloads_page ());
            add (build_privacy_page ());
            add (build_permissions_page ());
            add (build_extensions_page ());
            add (build_about_page ());
        }

        // ── General ───────────────────────────────────────────────────────

        private Adw.PreferencesPage build_general_page () {
            var page = new Adw.PreferencesPage ();
            page.title = "Všeobecné";
            page.icon_name = "preferences-system-symbolic";

            // Startup
            var startup_group = new Adw.PreferencesGroup ();
            startup_group.title = "Spustenie";
            page.add (startup_group);

            string[] sb_labels = { "Nová karta", "Domovská stránka", "Obnoviť predošlú reláciu" };
            string[] sb_values = { "newtab", "homepage", "restore" };
            var sb_model = new Gtk.StringList (sb_labels);
            var sb_row = new Adw.ComboRow ();
            sb_row.title = "Pri spustení otvoriť";
            sb_row.model = sb_model;
            string cur_sb = sm.get_startup_behavior ();
            for (int i = 0; i < sb_values.length; i++)
                if (sb_values[i] == cur_sb) { sb_row.selected = (uint) i; break; }
            sb_row.notify["selected"].connect (() => {
                uint sel = sb_row.selected;
                if (sel < sb_values.length) sm.set_startup_behavior (sb_values[sel]);
            });
            startup_group.add (sb_row);

            var hp_row = new Adw.EntryRow ();
            hp_row.title = "Domovská stránka";
            hp_row.text = sm.get_homepage ();
            hp_row.input_purpose = Gtk.InputPurpose.URL;
            hp_row.changed.connect (() => sm.set_homepage (hp_row.text));
            startup_group.add (hp_row);

            // Search engine
            var search_group = new Adw.PreferencesGroup ();
            search_group.title = "Vyhľadávanie";
            page.add (search_group);

            string[] engine_labels = {
                "DuckDuckGo", "Google", "Bing", "Yahoo", "Ecosia", "Startpage"
            };
            string[] engine_values = {
                "duckduckgo", "google", "bing", "yahoo", "ecosia", "startpage"
            };
            var engine_model = new Gtk.StringList (engine_labels);
            var engine_row = new Adw.ComboRow ();
            engine_row.title = "Predvolený vyhľadávač";
            engine_row.model = engine_model;
            string cur = sm.get_search_engine ();
            for (int i = 0; i < engine_values.length; i++) {
                if (engine_values[i] == cur) { engine_row.selected = (uint) i; break; }
            }
            engine_row.notify["selected"].connect (() => {
                uint sel = engine_row.selected;
                if (sel < engine_values.length) sm.set_search_engine (engine_values[sel]);
            });
            search_group.add (engine_row);

            // Browsing
            var browsing_group = new Adw.PreferencesGroup ();
            browsing_group.title = "Prehliadanie";
            page.add (browsing_group);

            var smooth_row = new Adw.ActionRow ();
            smooth_row.title = "Plynulé rolovanie";
            smooth_row.subtitle = "Animované rolovanie stránok";
            var smooth_sw = new Gtk.Switch ();
            smooth_sw.valign = Gtk.Align.CENTER;
            smooth_sw.active = sm.get_smooth_scrolling ();
            smooth_sw.notify["active"].connect (() => sm.set_smooth_scrolling (smooth_sw.active));
            smooth_row.add_suffix (smooth_sw);
            smooth_row.activatable_widget = smooth_sw;
            browsing_group.add (smooth_row);

            return page;
        }

        // ── Appearance ────────────────────────────────────────────────────

        private Adw.PreferencesPage build_appearance_page () {
            var page = new Adw.PreferencesPage ();
            page.title = "Zobrazenie";
            page.icon_name = "applications-graphics-symbolic";

            // Font
            var font_group = new Adw.PreferencesGroup ();
            font_group.title = "Písmo";
            page.add (font_group);

            string[] size_labels = { "12", "14", "16", "18", "20", "22", "24" };
            int[] size_values    = {  12,   14,   16,   18,   20,   22,   24  };
            var size_model = new Gtk.StringList (size_labels);
            var size_row = new Adw.ComboRow ();
            size_row.title = "Predvolená veľkosť písma";
            size_row.subtitle = "Veľkosť textu na webových stránkach (v pixeloch)";
            size_row.model = size_model;
            int cur_size = sm.get_default_font_size ();
            uint size_idx = 2;
            for (uint i = 0; i < size_values.length; i++) {
                if (size_values[i] == cur_size) { size_idx = i; break; }
            }
            size_row.selected = size_idx;
            size_row.notify["selected"].connect (() => {
                uint sel = size_row.selected;
                if (sel < size_values.length) sm.set_default_font_size (size_values[sel]);
            });
            font_group.add (size_row);

            // Zoom
            var zoom_group = new Adw.PreferencesGroup ();
            zoom_group.title = "Priblíženie";
            page.add (zoom_group);

            string[] zoom_labels = { "75%", "90%", "100%", "110%", "125%", "150%", "175%", "200%" };
            double[] zoom_values = { 0.75,  0.90,  1.0,   1.1,   1.25,   1.5,   1.75,   2.0  };
            var zoom_model = new Gtk.StringList (zoom_labels);
            var zoom_row = new Adw.ComboRow ();
            zoom_row.title = "Predvolené priblíženie stránky";
            zoom_row.model = zoom_model;
            double cur_zoom = sm.get_default_zoom ();
            uint zoom_idx = 2;
            for (uint i = 0; i < zoom_values.length; i++) {
                if (GLib.Math.fabs (zoom_values[i] - cur_zoom) < 0.01) { zoom_idx = i; break; }
            }
            zoom_row.selected = zoom_idx;
            zoom_row.notify["selected"].connect (() => {
                uint sel = zoom_row.selected;
                if (sel < zoom_values.length) sm.set_default_zoom (zoom_values[sel]);
            });
            zoom_group.add (zoom_row);

            return page;
        }

        // ── Downloads ─────────────────────────────────────────────────────

        private Adw.PreferencesPage build_downloads_page () {
            var page = new Adw.PreferencesPage ();
            page.title = "Sťahovania";
            page.icon_name = "folder-download-symbolic";

            var group = new Adw.PreferencesGroup ();
            group.title = "Priečinok";
            page.add (group);

            var folder_row = new Adw.ActionRow ();
            folder_row.title = "Uložiť do";
            folder_row.subtitle = sm.get_download_folder ();
            var change_btn = new Gtk.Button.with_label ("Zmeniť…");
            change_btn.valign = Gtk.Align.CENTER;
            change_btn.add_css_class ("flat");
            change_btn.clicked.connect (() => {
                var chooser = new Gtk.FileDialog ();
                chooser.title = "Vyberte priečinok";
                chooser.select_folder.begin (null, null, (obj, res) => {
                    try {
                        var folder = chooser.select_folder.end (res);
                        string path = folder.get_path ();
                        sm.set_download_folder (path);
                        folder_row.subtitle = path;
                    } catch {}
                });
            });
            folder_row.add_suffix (change_btn);
            folder_row.activatable_widget = change_btn;
            group.add (folder_row);

            var ask_row = new Adw.ActionRow ();
            ask_row.title = "Vždy sa spýtať kde uložiť";
            var ask_sw = new Gtk.Switch ();
            ask_sw.valign = Gtk.Align.CENTER;
            ask_sw.active = sm.get_ask_download_location ();
            ask_sw.notify["active"].connect (() => sm.set_ask_download_location (ask_sw.active));
            ask_row.add_suffix (ask_sw);
            ask_row.activatable_widget = ask_sw;
            group.add (ask_row);

            var open_row = new Adw.ActionRow ();
            open_row.title = "Otvoriť súbor po stiahnutí";
            open_row.subtitle = "Automaticky otvoriť dokončené stiahnutia";
            var open_sw = new Gtk.Switch ();
            open_sw.valign = Gtk.Align.CENTER;
            open_sw.active = sm.get_open_after_download ();
            open_sw.notify["active"].connect (() => sm.set_open_after_download (open_sw.active));
            open_row.add_suffix (open_sw);
            open_row.activatable_widget = open_sw;
            group.add (open_row);

            return page;
        }

        // ── Privacy ───────────────────────────────────────────────────────

        private Adw.PreferencesPage build_privacy_page () {
            var page = new Adw.PreferencesPage ();
            page.title = "Súkromie";
            page.icon_name = "security-high-symbolic";

            // Adblock
            var adblock_group = new Adw.PreferencesGroup ();
            adblock_group.title = "Blokovanie reklám";
            adblock_group.description = "Blokuje reklamy a sledovače na všetkých stránkach";
            page.add (adblock_group);

            var ab_row = new Adw.ActionRow ();
            ab_row.title = "Blokovanie reklám";
            ab_row.subtitle = "Blokuje reklamy, sledovače a analytické skripty";
            var ab_sw = new Gtk.Switch ();
            ab_sw.valign = Gtk.Align.CENTER;
            ab_sw.active = sm.get_adblock_enabled ();
            ab_sw.notify["active"].connect (() => {
                sm.set_adblock_enabled (ab_sw.active);
                AdBlockManager.get_instance ().enabled = ab_sw.active;
            });
            ab_row.add_suffix (ab_sw);
            ab_row.activatable_widget = ab_sw;
            adblock_group.add (ab_row);

            // Security
            var security_group = new Adw.PreferencesGroup ();
            security_group.title = "Bezpečnosť";
            page.add (security_group);

            var https_row = new Adw.ActionRow ();
            https_row.title = "Iba HTTPS";
            https_row.subtitle = "Automaticky presmerovať HTTP stránky na HTTPS";
            var https_sw = new Gtk.Switch ();
            https_sw.valign = Gtk.Align.CENTER;
            https_sw.active = sm.get_https_only ();
            https_sw.notify["active"].connect (() => sm.set_https_only (https_sw.active));
            https_row.add_suffix (https_sw);
            https_row.activatable_widget = https_sw;
            security_group.add (https_row);

            // Content
            var content_group = new Adw.PreferencesGroup ();
            content_group.title = "Obsah";
            page.add (content_group);

            var js_row = new Adw.ActionRow ();
            js_row.title = "JavaScript";
            js_row.subtitle = "Spustiť JavaScript na webových stránkach";
            var js_sw = new Gtk.Switch ();
            js_sw.valign = Gtk.Align.CENTER;
            js_sw.active = sm.get_javascript_enabled ();
            js_sw.notify["active"].connect (() => sm.set_javascript_enabled (js_sw.active));
            js_row.add_suffix (js_sw);
            js_row.activatable_widget = js_sw;
            content_group.add (js_row);

            var hw_row = new Adw.ActionRow ();
            hw_row.title = "Hardvérová akcelerácia";
            hw_row.subtitle = "Používať GPU pre vykreslenie stránok";
            var hw_sw = new Gtk.Switch ();
            hw_sw.valign = Gtk.Align.CENTER;
            hw_sw.active = sm.get_hardware_accel ();
            hw_sw.notify["active"].connect (() => sm.set_hardware_accel (hw_sw.active));
            hw_row.add_suffix (hw_sw);
            hw_row.activatable_widget = hw_sw;
            content_group.add (hw_row);

            var ap_row = new Adw.ActionRow ();
            ap_row.title = "Blokovať automatické prehrávanie";
            ap_row.subtitle = "Vyžadovať kliknutie pred prehrávaním médií";
            var ap_sw = new Gtk.Switch ();
            ap_sw.valign = Gtk.Align.CENTER;
            ap_sw.active = sm.get_block_autoplay ();
            ap_sw.notify["active"].connect (() => sm.set_block_autoplay (ap_sw.active));
            ap_row.add_suffix (ap_sw);
            ap_row.activatable_widget = ap_sw;
            content_group.add (ap_row);

            // History
            var history_group = new Adw.PreferencesGroup ();
            history_group.title = "História";
            page.add (history_group);

            var sh_row = new Adw.ActionRow ();
            sh_row.title = "Ukladať históriu prehliadania";
            sh_row.subtitle = "Záznamy navštívených stránok";
            var sh_sw = new Gtk.Switch ();
            sh_sw.valign = Gtk.Align.CENTER;
            sh_sw.active = sm.get_save_history ();
            sh_sw.notify["active"].connect (() => sm.set_save_history (sh_sw.active));
            sh_row.add_suffix (sh_sw);
            sh_row.activatable_widget = sh_sw;
            history_group.add (sh_row);

            var clear_hist_row = new Adw.ActionRow ();
            clear_hist_row.title = "Vymazať históriu prehliadania";
            clear_hist_row.subtitle = "Odstrániť všetky záznamy z histórie";
            var clear_hist_btn = new Gtk.Button.with_label ("Vymazať");
            clear_hist_btn.valign = Gtk.Align.CENTER;
            clear_hist_btn.add_css_class ("destructive-action");
            clear_hist_btn.clicked.connect (confirm_clear_history);
            clear_hist_row.add_suffix (clear_hist_btn);
            clear_hist_row.activatable_widget = clear_hist_btn;
            history_group.add (clear_hist_row);

            var cache_row = new Adw.ActionRow ();
            cache_row.title = "Vymazať cookies a cache";
            cache_row.subtitle = "Odstrániť uložené dáta stránok, prihlásenia a vyrovnávaciu pamäť";
            var cache_btn = new Gtk.Button.with_label ("Vymazať");
            cache_btn.valign = Gtk.Align.CENTER;
            cache_btn.add_css_class ("destructive-action");
            cache_btn.clicked.connect (confirm_clear_data);
            cache_row.add_suffix (cache_btn);
            cache_row.activatable_widget = cache_btn;
            history_group.add (cache_row);

            return page;
        }

        private void confirm_clear_history () {
            var dlg = new Adw.AlertDialog (
                "Vymazať históriu?",
                "Toto trvalo odstráni všetky záznamy z histórie prehliadania."
            );
            dlg.add_response ("cancel", "Zrušiť");
            dlg.add_response ("clear", "Vymazať");
            dlg.set_response_appearance ("clear", Adw.ResponseAppearance.DESTRUCTIVE);
            dlg.default_response = "cancel";
            dlg.close_response = "cancel";
            dlg.response.connect ((resp) => {
                if (resp == "clear") HistoryManager.get_instance ().clear ();
            });
            dlg.present (this);
        }

        private void confirm_clear_data () {
            var dlg = new Adw.AlertDialog (
                "Vymazať dáta prehliadania?",
                "Toto vymaže cookies, prihlásenia, vyrovnávaciu pamäť a lokálne dáta stránok. Budete odhlásení zo všetkých webových stránok."
            );
            dlg.add_response ("cancel", "Zrušiť");
            dlg.add_response ("clear", "Vymazať");
            dlg.set_response_appearance ("clear", Adw.ResponseAppearance.DESTRUCTIVE);
            dlg.default_response = "cancel";
            dlg.close_response = "cancel";
            dlg.response.connect ((resp) => {
                if (resp == "clear") do_clear_browsing_data ();
            });
            dlg.present (this);
        }

        private void do_clear_browsing_data () {
            var session = WebTab.get_shared_session ();
            if (session == null) return;
            var dm = session.get_website_data_manager ();
            dm.clear.begin (
                WebKit.WebsiteDataTypes.ALL,
                (GLib.TimeSpan) 0,
                null,
                (obj, res) => {
                    try { dm.clear.end (res); } catch {}
                }
            );
        }

        // ── Permissions ───────────────────────────────────────────────────

        private Adw.PreferencesPage build_permissions_page () {
            var page = new Adw.PreferencesPage ();
            page.title = "Oprávnenia";
            page.icon_name = "dialog-information-symbolic";

            var group = new Adw.PreferencesGroup ();
            group.title = "Prístup k zariadeniam";
            group.description = "Predvolený prístup webových stránok k zariadeniam a funkciám";
            page.add (group);

            string[] perm_labels = { "Spýtať sa", "Povoliť", "Blokovať" };
            string[] perm_values = { "ask", "allow", "block" };

            group.add (make_perm_row (
                "Kamera", "camera-video-symbolic",
                sm.get_camera_permission (), perm_labels, perm_values,
                (v) => sm.set_camera_permission (v)
            ));
            group.add (make_perm_row (
                "Mikrofón", "audio-input-microphone-symbolic",
                sm.get_microphone_permission (), perm_labels, perm_values,
                (v) => sm.set_microphone_permission (v)
            ));
            group.add (make_perm_row (
                "Poloha", "find-location-symbolic",
                sm.get_location_permission (), perm_labels, perm_values,
                (v) => sm.set_location_permission (v)
            ));
            group.add (make_perm_row (
                "Notifikácie", "notification-symbolic",
                sm.get_notification_permission (), perm_labels, perm_values,
                (v) => sm.set_notification_permission (v)
            ));

            return page;
        }

        private Adw.ComboRow make_perm_row (string title, string icon_name,
            string current, string[] labels, string[] values, PermSetter setter) {
            var model = new Gtk.StringList (labels);
            var row = new Adw.ComboRow ();
            row.title = title;
            row.model = model;

            var icon = new Gtk.Image.from_icon_name (icon_name);
            icon.pixel_size = 16;
            row.add_prefix (icon);

            uint idx = 0;
            for (uint i = 0; i < values.length; i++) {
                if (values[i] == current) { idx = i; break; }
            }
            row.selected = idx;
            row.notify["selected"].connect (() => {
                uint sel = row.selected;
                if (sel < values.length) setter (values[sel]);
            });
            return row;
        }

        // ── Extensions ───────────────────────────────────────────────────

        private Adw.PreferencesPage build_extensions_page () {
            var page = new Adw.PreferencesPage ();
            page.title = "Rozšírenia";
            page.icon_name = "application-x-addon-symbolic";

            // Info / open folder
            var info_group = new Adw.PreferencesGroup ();
            info_group.title = "Priečinok rozšírení";
            info_group.description = "Pridajte rozšírenie: rozbaľte priečinok s manifest.json do priečinka rozšírení a reštartujte prehliadač.";
            page.add (info_group);

            var folder_row = new Adw.ActionRow ();
            folder_row.title = "Otvoriť priečinok rozšírení";
            var ext_mgr = ExtensionManager.get_instance ();
            folder_row.subtitle = ext_mgr.ext_dir;
            var open_btn = new Gtk.Button.with_label ("Otvoriť");
            open_btn.valign = Gtk.Align.CENTER;
            open_btn.add_css_class ("flat");
            open_btn.clicked.connect (() => {
                try {
                    GLib.AppInfo.launch_default_for_uri (
                        GLib.Filename.to_uri (ext_mgr.ext_dir, null), null
                    );
                } catch {}
            });
            folder_row.add_suffix (open_btn);
            folder_row.activatable_widget = open_btn;
            info_group.add (folder_row);

            // Installed extensions list
            var list_group = new Adw.PreferencesGroup ();
            list_group.title = "Nainštalované rozšírenia";
            page.add (list_group);

            var extensions = ext_mgr.get_extensions ();

            if (extensions.size == 0) {
                var empty_row = new Adw.ActionRow ();
                empty_row.title = "Žiadne rozšírenia";
                empty_row.subtitle = "Pridajte rozšírenia do priečinka vyššie";
                empty_row.sensitive = false;
                list_group.add (empty_row);
            } else {
                foreach (var ext in extensions) {
                    var row = new Adw.ActionRow ();
                    row.title = ext.name;
                    string sub = ext.version.length > 0 ? "v" + ext.version : "";
                    if (ext.description.length > 0) {
                        sub = sub.length > 0 ? sub + " – " + ext.description : ext.description;
                    }
                    if (sub.length > 0) row.subtitle = sub;

                    var sw = new Gtk.Switch ();
                    sw.valign = Gtk.Align.CENTER;
                    sw.active = ext.enabled;
                    sw.notify["active"].connect (() => {
                        ext.enabled = sw.active;
                        sm.set_extension_enabled (ext.id, sw.active);
                    });
                    row.add_suffix (sw);
                    row.activatable_widget = sw;
                    list_group.add (row);
                }
            }

            // Supported APIs note
            var note_group = new Adw.PreferencesGroup ();
            note_group.title = "Poznámka ku kompatibilite";
            page.add (note_group);

            var note_row = new Adw.ActionRow ();
            note_row.title = "Podporované funkcie";
            note_row.subtitle = "Content scripts (JS + CSS), chrome.storage, chrome.runtime. Background scripts, service workers a native messaging nie sú podporované.";
            note_row.sensitive = false;
            note_group.add (note_row);

            return page;
        }

        // ── About ─────────────────────────────────────────────────────────

        private Adw.PreferencesPage build_about_page () {
            var page = new Adw.PreferencesPage ();
            page.title = "O aplikácii";
            page.icon_name = "help-about-symbolic";

            var group = new Adw.PreferencesGroup ();
            page.add (group);

            var app_row = new Adw.ActionRow ();
            app_row.title = "CrowBrowser";
            app_row.subtitle = "Verzia " + UpdateManager.APP_VERSION;
            var app_ico = new Gtk.Image.from_icon_name ("web-browser-symbolic");
            app_ico.pixel_size = 32;
            app_row.add_prefix (app_ico);
            group.add (app_row);

            var wk_row = new Adw.ActionRow ();
            wk_row.title = "Renderovací engine";
            wk_row.subtitle = "WebKitGTK 6.0";
            group.add (wk_row);

            var ui_row = new Adw.ActionRow ();
            ui_row.title = "UI framework";
            ui_row.subtitle = "GTK4 + Libadwaita 1.9";
            group.add (ui_row);

            var lic_row = new Adw.ActionRow ();
            lic_row.title = "Licencia";
            lic_row.subtitle = "GNU General Public License v3.0";
            group.add (lic_row);

            var enc_row = new Adw.ActionRow ();
            enc_row.title = "Šifrovanie dát";
            enc_row.subtitle = "História je šifrovaná HMAC-SHA256 CTR pomocou kľúča tohto zariadenia";
            var enc_ico = new Gtk.Image.from_icon_name ("security-high-symbolic");
            enc_ico.pixel_size = 16;
            enc_row.add_prefix (enc_ico);
            group.add (enc_row);

            return page;
        }
    }
}
