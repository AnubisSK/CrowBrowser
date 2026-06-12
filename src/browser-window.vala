namespace CrowBrowser {

    // ── Sidebar tab row ─────────────────────────────────────────────────────

    private class SidebarTabRow : Gtk.Box {

        public unowned Adw.TabPage page { get; private set; }

        private Gtk.Stack icon_stack;
        private Gtk.Image favicon_img;
        private Gtk.Spinner spinner;
        private Gtk.Label title_lbl;
        private Gtk.Button close_btn;

        public signal void activated ();
        public signal void close_requested ();

        public SidebarTabRow (Adw.TabPage page) {
            Object (orientation: Gtk.Orientation.HORIZONTAL, spacing: 7);
            this.page = page;
            add_css_class ("crow-tab-row");

            favicon_img = new Gtk.Image.from_icon_name ("web-browser-symbolic");
            favicon_img.pixel_size = 14;
            favicon_img.valign = Gtk.Align.CENTER;

            spinner = new Gtk.Spinner ();
            spinner.spinning = true;
            spinner.valign = Gtk.Align.CENTER;

            icon_stack = new Gtk.Stack ();
            icon_stack.transition_type = Gtk.StackTransitionType.CROSSFADE;
            icon_stack.transition_duration = 80;
            icon_stack.add_named (favicon_img, "favicon");
            icon_stack.add_named (spinner, "spinner");
            icon_stack.visible_child_name = "favicon";
            icon_stack.valign = Gtk.Align.CENTER;

            title_lbl = new Gtk.Label ("Nová karta");
            title_lbl.add_css_class ("tab-title");
            title_lbl.hexpand = true;
            title_lbl.halign = Gtk.Align.START;
            title_lbl.ellipsize = Pango.EllipsizeMode.END;
            title_lbl.valign = Gtk.Align.CENTER;
            title_lbl.xalign = 0;

            close_btn = new Gtk.Button.from_icon_name ("window-close-symbolic");
            close_btn.add_css_class ("tab-close-btn");
            close_btn.valign = Gtk.Align.CENTER;
            close_btn.clicked.connect (() => close_requested ());

            append (icon_stack);
            append (title_lbl);
            append (close_btn);

            var click = new Gtk.GestureClick ();
            click.set_propagation_phase (Gtk.PropagationPhase.BUBBLE);
            click.pressed.connect ((n, x, y) => activated ());
            add_controller (click);
            set_cursor_from_name ("pointer");
        }

        public void set_active (bool active) {
            if (active) add_css_class ("active-tab");
            else remove_css_class ("active-tab");
        }

        public void update_title (string t) {
            title_lbl.label = t;
            page.title = t;
        }

        public void update_favicon (Gdk.Texture? tex) {
            if (tex != null) favicon_img.set_from_paintable (tex);
            else favicon_img.icon_name = "web-browser-symbolic";
        }

        public void set_loading (bool l) {
            icon_stack.visible_child_name = l ? "spinner" : "favicon";
        }
    }

    // ── Main browser window ─────────────────────────────────────────────────

    public class BrowserWindow : Adw.ApplicationWindow {

        // "normal" | "private" | "tor"
        public string window_mode { get; construct; default = "normal"; }

        private Adw.TabView tab_view;
        private Gtk.Box tabs_box;
        private Gee.ArrayList<SidebarTabRow> tab_rows;

        private Gtk.Entry url_entry;
        private Gtk.Button back_button;
        private Gtk.Button forward_button;
        private Gtk.Button reload_button;
        private Gtk.Button dl_button;
        private Gtk.ProgressBar progress_bar;
        private Adw.ToastOverlay toast_overlay;

        private Adw.HeaderBar header_bar;
        private Gtk.Revealer sidebar_revealer;
        private bool sidebar_visible = true;
        private bool is_fullscreen = false;
        private bool pre_fs_sidebar = true;

        private Gtk.Popover dl_popover;
        private Gtk.Stack dl_pop_stack;
        private Gtk.ListBox dl_pop_list;
        private Gtk.Label dl_pop_header;
        private Gtk.Label dl_badge;

        private Adw.ToolbarView tview;
        private Adw.Banner update_banner;

        // Ephemeral network session shared by all tabs in this private/tor window
        private WebKit.NetworkSession? private_session = null;
        private bool tor_not_found = false;

        public BrowserWindow (Gtk.Application app) {
            Object (application: app);
        }

        public BrowserWindow.with_mode (Gtk.Application app, string mode) {
            Object (application: app, window_mode: mode);
        }

        construct {
            // Initialize private/tor session before building UI
            if (window_mode == "private") {
                private_session = new WebKit.NetworkSession.ephemeral ();
            } else if (window_mode == "tor") {
                int tor_port = detect_tor_port ();
                private_session = new WebKit.NetworkSession.ephemeral ();
                if (tor_port > 0) {
                    var proxy = new WebKit.NetworkProxySettings (
                        @"socks://127.0.0.1:$(tor_port)", null
                    );
                    private_session.set_proxy_settings (
                        WebKit.NetworkProxyMode.CUSTOM, proxy
                    );
                } else {
                    tor_not_found = true;
                }
            }

            string mode_label = (window_mode == "tor") ? " — Tor"
                : (window_mode == "private") ? " — Súkromné"
                : "";
            title = "CrowBrowser" + mode_label;

            set_default_size (1280, 820);
            tab_rows = new Gee.ArrayList<SidebarTabRow> ();
            build_ui ();
            setup_shortcuts ();
            setup_actions ();
            setup_initial_tabs ();

            // Show Tor warning on next main loop iteration (after window is shown)
            if (tor_not_found) {
                GLib.Idle.add (() => {
                    var toast = new Adw.Toast (
                        "Tor nie je spustený — prehliadanie prebieha ako súkromné (bez Tor siete)"
                    );
                    toast.timeout = 10;
                    toast_overlay.add_toast (toast);
                    return GLib.Source.REMOVE;
                });
            }

            // Connect update-manager signals (only normal windows show the update banner)
            if (window_mode != "private" && window_mode != "tor") {
                var um = UpdateManager.get_instance ();
                um.update_available.connect ((ver) => {
                    GLib.Idle.add (() => { show_update_available (ver); return GLib.Source.REMOVE; });
                });
                um.install_progress.connect ((status) => {
                    GLib.Idle.add (() => {
                        update_banner.title = status;
                        update_banner.button_label = "";
                        update_banner.revealed = true;
                        return GLib.Source.REMOVE;
                    });
                });
                um.install_done.connect (() => {
                    GLib.Idle.add (() => {
                        update_banner.title = "Aktualizácia nainštalovaná — reštartujte CrowBrowser";
                        update_banner.button_label = "";
                        update_banner.revealed = true;
                        return GLib.Source.REMOVE;
                    });
                });
                um.install_failed.connect ((msg) => {
                    GLib.Idle.add (() => {
                        update_banner.revealed = false;
                        var t = new Adw.Toast ("Aktualizácia zlyhala: " + msg);
                        t.timeout = 8;
                        toast_overlay.add_toast (t);
                        return GLib.Source.REMOVE;
                    });
                });
            }
        }

        private void show_update_available (string version) {
            update_banner.title = "Dostupná aktualizácia v%s".printf (version);
            update_banner.button_label = "Nainštalovať";
            update_banner.revealed = true;
            update_banner.button_clicked.connect (() => {
                update_banner.button_label = "";
                UpdateManager.get_instance ().install_async.begin (version);
            });
        }

        private void setup_initial_tabs () {
            // Private and Tor windows always start with a blank tab
            if (window_mode == "private" || window_mode == "tor") {
                new_tab ("about:blank");
                return;
            }

            var sm_inst = SettingsManager.get_instance ();
            string behavior = sm_inst.get_startup_behavior ();

            if (behavior == "restore") {
                var app = application as Application;
                if (app != null) {
                    string[] urls = app.consume_session_urls ();
                    if (urls.length > 0) {
                        foreach (string url in urls) new_tab (url);
                        return;
                    }
                }
            }

            if (behavior == "homepage") {
                string hp = sm_inst.get_homepage ();
                new_tab (hp.length > 0 ? hp : "https://start.duckduckgo.com");
            } else {
                new_tab ("about:blank");
            }
        }

        public string[] get_open_urls () {
            // Don't save private or Tor window sessions
            if (window_mode == "private" || window_mode == "tor") return new string[0];
            var urls = new Gee.ArrayList<string> ();
            for (int i = 0; i < tab_view.n_pages; i++) {
                var pg = tab_view.get_nth_page (i);
                var tab = pg.get_child () as WebTab;
                if (tab != null && tab.tab_uri.length > 0 &&
                    tab.tab_uri != "about:blank")
                    urls.add (tab.tab_uri);
            }
            return urls.to_array ();
        }

        private static int detect_tor_port () {
            foreach (uint16 port in new uint16[] { 9050, 9150 }) {
                try {
                    var client = new GLib.SocketClient ();
                    client.set_timeout (1);
                    var conn = client.connect_to_host ("127.0.0.1", port, null);
                    conn.close ();
                    return (int) port;
                } catch {}
            }
            return 0;
        }

        // ── UI construction ─────────────────────────────────────────────────

        private void build_ui () {
            // TabView — no TabBar (sidebar replaces it)
            tab_view = new Adw.TabView ();
            tab_view.close_page.connect (on_close_page);
            tab_view.notify["selected-page"].connect (on_selected_page_changed);
            tab_view.page_attached.connect (on_page_attached);
            tab_view.page_detached.connect (on_page_detached);

            // Progress bar overlaid at top of WebView area
            progress_bar = new Gtk.ProgressBar ();
            progress_bar.add_css_class ("crow-progress");
            progress_bar.valign = Gtk.Align.START;
            progress_bar.visible = false;

            var webview_overlay = new Gtk.Overlay ();
            webview_overlay.set_child (tab_view);
            webview_overlay.add_overlay (progress_bar);
            webview_overlay.vexpand = true;
            webview_overlay.hexpand = true;

            // Toast overlay (DRM warnings etc.)
            toast_overlay = new Adw.ToastOverlay ();
            toast_overlay.set_child (webview_overlay);
            toast_overlay.vexpand = true;
            toast_overlay.hexpand = true;

            // Sidebar panel (tab list only — NO headerbar inside)
            var sidebar_panel = build_sidebar_panel ();
            sidebar_revealer = new Gtk.Revealer ();
            sidebar_revealer.transition_type = Gtk.RevealerTransitionType.SLIDE_RIGHT;
            sidebar_revealer.transition_duration = 180;
            sidebar_revealer.reveal_child = true;
            sidebar_revealer.hexpand = false;
            sidebar_revealer.vexpand = true;
            sidebar_revealer.set_child (sidebar_panel);

            // Horizontal split: sidebar | content
            var body = new Gtk.Box (Gtk.Orientation.HORIZONTAL, 0);
            body.append (sidebar_revealer);
            body.append (toast_overlay);

            // Full-width header bar + body inside ToolbarView
            tview = new Adw.ToolbarView ();
            tview.add_top_bar (build_header_bar ());
            if (window_mode == "private" || window_mode == "tor")
                tview.add_top_bar (build_mode_bar ());

            // Update notification banner (hidden until an update is available)
            update_banner = new Adw.Banner ("");
            update_banner.revealed = false;
            tview.add_top_bar (update_banner);

            tview.content = body;

            content = tview;
        }

        private Gtk.Widget build_mode_bar () {
            var box = new Gtk.Box (Gtk.Orientation.HORIZONTAL, 8);
            box.margin_start = 16;
            box.margin_end = 16;
            box.margin_top = 5;
            box.margin_bottom = 5;

            bool is_tor = (window_mode == "tor");
            string icon_name = is_tor ? "network-vpn-symbolic" : "security-high-symbolic";
            string text = is_tor
                ? "Tor — anonymná šifrovaná sieť · história a cookies sa neukladajú"
                : "Súkromné prehliadanie · história a cookies sa neukladajú";
            string css_class = is_tor ? "crow-tor-bar" : "crow-private-bar";

            var icon = new Gtk.Image.from_icon_name (icon_name);
            icon.pixel_size = 14;
            icon.valign = Gtk.Align.CENTER;

            var lbl = new Gtk.Label (text);
            lbl.add_css_class ("caption");
            lbl.halign = Gtk.Align.START;
            lbl.hexpand = true;

            box.append (icon);
            box.append (lbl);
            box.add_css_class (css_class);

            return box;
        }

        // ── Header bar (full-width, has window controls) ────────────────────

        private Adw.HeaderBar build_header_bar () {
            header_bar = new Adw.HeaderBar ();
            header_bar.add_css_class ("crow-main-hbar");

            // Sidebar toggle (leftmost)
            var toggle = new Gtk.Button.from_icon_name ("sidebar-show-symbolic");
            toggle.add_css_class ("nav-button");
            toggle.tooltip_text = "Skryť/zobraziť panel (Ctrl+B)";
            toggle.clicked.connect (toggle_sidebar);
            header_bar.pack_start (toggle);

            // Navigation
            back_button = new Gtk.Button.from_icon_name ("go-previous-symbolic");
            back_button.add_css_class ("nav-button");
            back_button.tooltip_text = "Späť (Alt+←)";
            back_button.sensitive = false;
            back_button.clicked.connect (() => current_tab ()?.go_back ());

            forward_button = new Gtk.Button.from_icon_name ("go-next-symbolic");
            forward_button.add_css_class ("nav-button");
            forward_button.tooltip_text = "Vpred (Alt+→)";
            forward_button.sensitive = false;
            forward_button.clicked.connect (() => current_tab ()?.go_forward ());

            reload_button = new Gtk.Button.from_icon_name ("view-refresh-symbolic");
            reload_button.add_css_class ("nav-button");
            reload_button.tooltip_text = "Obnoviť (F5)";
            reload_button.clicked.connect (() => current_tab ()?.reload ());

            header_bar.pack_start (back_button);
            header_bar.pack_start (forward_button);
            header_bar.pack_start (reload_button);

            // URL entry as centre title widget
            url_entry = new Gtk.Entry ();
            url_entry.add_css_class ("crow-url-entry");
            url_entry.hexpand = true;
            url_entry.placeholder_text = "Hľadaj alebo zadaj URL…";
            url_entry.input_purpose = Gtk.InputPurpose.URL;
            url_entry.set_icon_from_icon_name (Gtk.EntryIconPosition.PRIMARY, "web-browser-symbolic");
            url_entry.activate.connect (on_url_activated);
            url_entry.icon_press.connect (on_url_icon_pressed);

            var url_clamp = new Adw.Clamp ();
            url_clamp.maximum_size = 820;
            url_clamp.hexpand = true;
            url_clamp.set_child (url_entry);
            header_bar.set_title_widget (url_clamp);

            // Right buttons — video download with custom film+arrow icon
            dl_button = new Gtk.Button.from_icon_name ("crow-video-dl-symbolic");
            dl_button.add_css_class ("crow-action-btn");
            dl_button.tooltip_text = "Stiahnuť video (Ctrl+D)";

            // Badge overlay showing detected video count
            var dl_overlay = new Gtk.Overlay ();
            dl_badge = new Gtk.Label ("");
            dl_badge.add_css_class ("crow-dl-badge");
            dl_badge.halign = Gtk.Align.END;
            dl_badge.valign = Gtk.Align.START;
            dl_badge.visible = false;
            dl_overlay.set_child (dl_button);
            dl_overlay.add_overlay (dl_badge);

            // Popover panel (Video DownloadHelper-style)
            dl_popover = build_dl_popover ();
            dl_popover.set_parent (dl_button);
            dl_button.clicked.connect (() => {
                rebuild_dl_popover (current_tab ());
                dl_popover.popup ();
            });

            var devtools_btn = new Gtk.Button.from_icon_name ("preferences-system-symbolic");
            devtools_btn.add_css_class ("crow-action-btn");
            devtools_btn.tooltip_text = "Nástroje pre vývojárov (F12)";
            devtools_btn.clicked.connect (() => current_tab ()?.open_inspector ());

            var menu_btn = new Gtk.MenuButton ();
            menu_btn.add_css_class ("crow-action-btn");
            menu_btn.icon_name = "open-menu-symbolic";
            menu_btn.tooltip_text = "Menu";
            menu_btn.menu_model = build_app_menu ();

            header_bar.pack_end (menu_btn);
            header_bar.pack_end (devtools_btn);
            header_bar.pack_end (dl_overlay);

            return header_bar;
        }

        // ── Sidebar panel (tab list only) ────────────────────────────────────

        private Gtk.Widget build_sidebar_panel () {
            // Tab list
            tabs_box = new Gtk.Box (Gtk.Orientation.VERTICAL, 0);
            tabs_box.margin_top = 4;
            tabs_box.margin_bottom = 4;

            var scroll = new Gtk.ScrolledWindow ();
            scroll.add_css_class ("crow-tabs-scroll");
            scroll.set_policy (Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC);
            scroll.vexpand = true;
            scroll.set_child (tabs_box);

            // Bottom bar
            var bottom = new Gtk.Box (Gtk.Orientation.HORIZONTAL, 2);
            bottom.add_css_class ("crow-sidebar-bottom");

            var new_tab_btn = make_sidebar_btn ("tab-new-symbolic", "Nová karta (Ctrl+T)");
            new_tab_btn.clicked.connect (() => new_tab ("about:blank"));

            bool has_drm = WebTab.widevine_cdm_available ();
            var drm_btn = make_sidebar_btn (
                has_drm ? "security-high-symbolic" : "security-low-symbolic",
                has_drm ? "DRM: Widevine aktívny" : "DRM: nie je nainštalovaný"
            );
            if (!has_drm) drm_btn.add_css_class ("crow-sidebar-action-warn");
            drm_btn.clicked.connect (show_drm_info);

            var spacer = new Gtk.Box (Gtk.Orientation.HORIZONTAL, 0);
            spacer.hexpand = true;

            bottom.append (new_tab_btn);
            bottom.append (drm_btn);
            bottom.append (spacer);

            // Panel box
            var panel = new Gtk.Box (Gtk.Orientation.VERTICAL, 0);
            panel.add_css_class ("crow-sidebar-panel");
            panel.width_request = 168;
            panel.append (scroll);
            panel.append (bottom);

            return panel;
        }

        private Gtk.Button make_sidebar_btn (string icon, string tip) {
            var b = new Gtk.Button.from_icon_name (icon);
            b.add_css_class ("crow-sidebar-action");
            b.tooltip_text = tip;
            return b;
        }

        // ── App menu ────────────────────────────────────────────────────────

        private GLib.MenuModel build_app_menu () {
            var menu = new GLib.Menu ();

            var nav = new GLib.Menu ();
            nav.append ("Nová karta",  "win.new-tab");
            nav.append ("Nové okno",   "app.new-window");
            nav.append ("Súkromné okno (Ctrl+Shift+N)", "app.new-private-window");
            nav.append ("Tor okno (Ctrl+Shift+T)",      "app.new-tor-window");
            menu.append_section (null, nav);

            var browser = new GLib.Menu ();
            browser.append ("História (Ctrl+H)",          "win.history");
            browser.append ("Správca sťahovania (Ctrl+J)", "win.downloads");
            browser.append ("Stiahnuť video…",            "win.download-video");
            menu.append_section (null, browser);

            var dev = new GLib.Menu ();
            dev.append ("Nástroje pre vývojárov", "win.open-devtools");
            dev.append ("Zobraziť zdrojový kód",  "win.view-source");
            dev.append ("DRM / Widevine…",        "win.drm-info");
            menu.append_section (null, dev);

            var misc = new GLib.Menu ();
            misc.append ("Nastavenia",   "win.settings");
            misc.append ("O CrowBrowser", "win.about");
            misc.append ("Ukončiť",       "app.quit");
            menu.append_section (null, misc);

            return menu;
        }

        // ── Actions ─────────────────────────────────────────────────────────

        private void setup_actions () {
            var a1 = new GLib.SimpleAction ("new-tab", null);
            a1.activate.connect ((a, p) => new_tab ("about:blank"));
            add_action (a1);

            var a2 = new GLib.SimpleAction ("close-tab", null);
            a2.activate.connect ((a, p) => on_close_current_tab ());
            add_action (a2);

            var a3 = new GLib.SimpleAction ("reload", null);
            a3.activate.connect ((a, p) => current_tab ()?.reload ());
            add_action (a3);

            var a4 = new GLib.SimpleAction ("focus-url", null);
            a4.activate.connect ((a, p) => {
                url_entry.grab_focus ();
                url_entry.select_region (0, -1);
            });
            add_action (a4);

            var a5 = new GLib.SimpleAction ("download-video", null);
            a5.activate.connect ((a, p) => on_download_video_clicked ());
            add_action (a5);

            var a6 = new GLib.SimpleAction ("drm-info", null);
            a6.activate.connect ((a, p) => show_drm_info ());
            add_action (a6);

            var a7 = new GLib.SimpleAction ("about", null);
            a7.activate.connect ((a, p) => show_about ());
            add_action (a7);

            var a8 = new GLib.SimpleAction ("toggle-sidebar", null);
            a8.activate.connect ((a, p) => toggle_sidebar ());
            add_action (a8);

            var a9 = new GLib.SimpleAction ("find", null);
            a9.activate.connect ((a, p) => { /* TODO */ });
            add_action (a9);

            var a10 = new GLib.SimpleAction ("open-devtools", null);
            a10.activate.connect ((a, p) => current_tab ()?.open_inspector ());
            add_action (a10);

            var a11 = new GLib.SimpleAction ("view-source", null);
            a11.activate.connect ((a, p) => {
                var tab = current_tab ();
                if (tab != null && !tab.tab_uri.has_prefix ("view-source:"))
                    tab.load_url ("view-source:" + tab.tab_uri);
            });
            add_action (a11);

            var a12 = new GLib.SimpleAction ("history", null);
            a12.activate.connect ((a, p) => show_history ());
            add_action (a12);

            var a13 = new GLib.SimpleAction ("downloads", null);
            a13.activate.connect ((a, p) => show_downloads ());
            add_action (a13);

            var a14 = new GLib.SimpleAction ("settings", null);
            a14.activate.connect ((a, p) => show_settings ());
            add_action (a14);
        }

        // ── Shortcuts ────────────────────────────────────────────────────────

        private void setup_shortcuts () {
            var app = application as Gtk.Application;
            if (app == null) return;

            app.set_accels_for_action ("win.new-tab",            {"<primary>t"});
            app.set_accels_for_action ("win.close-tab",          {"<primary>w"});
            app.set_accels_for_action ("win.reload",             {"<primary>r", "F5"});
            app.set_accels_for_action ("win.focus-url",          {"<primary>l", "F6"});
            app.set_accels_for_action ("win.download-video",     {"<primary>d"});
            app.set_accels_for_action ("win.toggle-sidebar",     {"<primary>b"});
            app.set_accels_for_action ("win.open-devtools",      {"F12"});
            app.set_accels_for_action ("win.view-source",        {"<primary>u"});
            app.set_accels_for_action ("win.history",            {"<primary>h"});
            app.set_accels_for_action ("win.downloads",          {"<primary>j"});
            app.set_accels_for_action ("win.settings",           {"<primary>comma"});
            app.set_accels_for_action ("app.new-private-window", {"<primary><shift>n"});
            app.set_accels_for_action ("app.new-tor-window",     {"<primary><shift>t"});

            var ctrl = new Gtk.ShortcutController ();
            ctrl.add_shortcut (new Gtk.Shortcut (
                Gtk.ShortcutTrigger.parse_string ("<alt>Left"),
                new Gtk.CallbackAction (() => { current_tab ()?.go_back (); return true; })
            ));
            ctrl.add_shortcut (new Gtk.Shortcut (
                Gtk.ShortcutTrigger.parse_string ("<alt>Right"),
                new Gtk.CallbackAction (() => { current_tab ()?.go_forward (); return true; })
            ));
            ctrl.add_shortcut (new Gtk.Shortcut (
                Gtk.ShortcutTrigger.parse_string ("<primary>Tab"),
                new Gtk.CallbackAction (() => { cycle_tabs (1); return true; })
            ));
            ctrl.add_shortcut (new Gtk.Shortcut (
                Gtk.ShortcutTrigger.parse_string ("<primary><shift>Tab"),
                new Gtk.CallbackAction (() => { cycle_tabs (-1); return true; })
            ));
            ctrl.add_shortcut (new Gtk.Shortcut (
                Gtk.ShortcutTrigger.parse_string ("Escape"),
                new Gtk.CallbackAction (() => {
                    if (!is_fullscreen) return false;
                    on_leave_fullscreen ();
                    return true;
                })
            ));
            add_controller (ctrl);
        }

        // ── Sidebar toggle ────────────────────────────────────────────────────

        private void toggle_sidebar () {
            sidebar_visible = !sidebar_visible;
            sidebar_revealer.reveal_child = sidebar_visible;
        }

        // ── Fullscreen ────────────────────────────────────────────────────────

        private void on_enter_fullscreen () {
            if (is_fullscreen) return;
            is_fullscreen = true;
            pre_fs_sidebar = sidebar_visible;
            header_bar.visible = false;
            if (sidebar_visible) {
                sidebar_visible = false;
                sidebar_revealer.reveal_child = false;
            }
            fullscreen ();
        }

        private void on_leave_fullscreen () {
            if (!is_fullscreen) return;
            is_fullscreen = false;
            header_bar.visible = true;
            if (pre_fs_sidebar && !sidebar_visible) {
                sidebar_visible = true;
                sidebar_revealer.reveal_child = true;
            }
            unfullscreen ();
        }

        // ── Tab management ────────────────────────────────────────────────────

        public unowned Adw.TabPage new_tab (string url) {
            WebTab tab;
            if (private_session != null) {
                tab = (WebTab) GLib.Object.new (typeof (WebTab),
                    "tab-network-session", private_session);
            } else {
                tab = new WebTab ();
            }
            tab.title_changed.connect ((t)   => on_tab_title_changed (tab, t));
            tab.uri_changed.connect ((u)     => on_tab_uri_changed (tab, u));
            tab.loading_changed.connect ((l, p) => on_tab_loading_changed (tab, l, p));
            tab.favicon_changed.connect ((f) => on_tab_favicon_changed (tab, f));
            tab.drm_blocked.connect (()      => on_drm_blocked ());
            tab.video_detected.connect ((u)  => on_tab_video_detected (tab, u));
            tab.fullscreen_entered.connect (on_enter_fullscreen);
            tab.fullscreen_left.connect    (on_leave_fullscreen);

            unowned Adw.TabPage page = tab_view.append (tab);
            page.title = "Nová karta";
            page.loading = true;
            tab_view.selected_page = page;

            if (url != "about:blank") tab.load_url (url);
            else load_newtab_page (tab);

            return page;
        }

        private void load_newtab_page (WebTab tab) {
            tab.web_view.load_html ("""<!DOCTYPE html>
<html><head>
<meta charset="UTF-8"><title>Nová karta</title>
<meta name="color-scheme" content="dark">
<style>
*{box-sizing:border-box;margin:0;padding:0}
body{font-family:system-ui,sans-serif;display:flex;flex-direction:column;align-items:center;justify-content:center;min-height:100vh;background:#17151f;color:rgba(255,255,255,.85)}
.logo{font-size:2.6rem;font-weight:700;letter-spacing:-1.5px;background:linear-gradient(135deg,#c4b5fd,#7c5cf5);-webkit-background-clip:text;-webkit-text-fill-color:transparent;margin-bottom:.2rem}
.tagline{font-size:.9rem;opacity:.4;margin-bottom:2rem}
form{display:flex;gap:8px;width:100%;max-width:480px;padding:0 1rem}
input{flex:1;padding:10px 18px;font-size:.9rem;background:rgba(255,255,255,.07);border:1px solid rgba(255,255,255,.10);border-radius:22px;outline:none;color:rgba(255,255,255,.9)}
input:focus{border-color:rgba(124,92,245,.6);background:rgba(255,255,255,.10);box-shadow:0 0 0 3px rgba(124,92,245,.14)}
button{padding:10px 20px;font-size:.9rem;font-weight:600;background:#7c5cf5;color:#fff;border:none;border-radius:22px;cursor:pointer;transition:background 120ms}
button:hover{background:#9270ff}
.tip{margin-top:2rem;font-size:.75rem;opacity:.28;letter-spacing:.2px}
</style></head><body>
<div class="logo">CrowBrowser</div>
<div class="tagline">Rýchly, moderný prehliadač pre Linux</div>
<form onsubmit="go(event)">
  <input id="q" autofocus placeholder="Hľadaj alebo zadaj URL…" autocomplete="off">
  <button type="submit">Hľadaj</button>
</form>
<div class="tip">Ctrl+T = nová karta &nbsp;·&nbsp; Ctrl+D = stiahnuť video &nbsp;·&nbsp; Ctrl+B = panel</div>
<script>
function go(e){e.preventDefault();var q=document.getElementById('q').value.trim();if(!q)return;
if(/^https?:\/\//.test(q)||/^[a-zA-Z0-9-]+\.[a-zA-Z]{2,}/.test(q))window.location=q.startsWith('http')?q:'https://'+q;
else window.location='https://duckduckgo.com/?q='+encodeURIComponent(q);}
</script></body></html>""", "about:blank");
        }

        private WebTab? current_tab () {
            var page = tab_view.selected_page;
            return (page != null) ? (page.get_child () as WebTab) : null;
        }

        private SidebarTabRow? row_for_tab (WebTab tab) {
            var page = tab_view.get_page (tab);
            if (page == null) return null;
            foreach (var row in tab_rows)
                if (row.page == page) return row;
            return null;
        }

        private void cycle_tabs (int dir) {
            int n = tab_view.n_pages;
            if (n <= 1) return;
            var page = tab_view.selected_page;
            if (page == null) return;
            int pos = tab_view.get_page_position (page);
            tab_view.selected_page = tab_view.get_nth_page ((pos + dir + n) % n);
        }

        private void on_close_current_tab () {
            var p = tab_view.selected_page;
            if (p != null) tab_view.close_page (p);
        }

        // ── TabView signals ───────────────────────────────────────────────────

        private void on_page_attached (Adw.TabPage page, int pos) {
            var row = new SidebarTabRow (page);
            row.activated.connect (() => tab_view.selected_page = row.page);
            row.close_requested.connect (() => tab_view.close_page (row.page));
            tab_rows.add (row);
            tabs_box.append (row);
            sync_active_row ();
        }

        private void on_page_detached (Adw.TabPage page, int pos) {
            SidebarTabRow? dead = null;
            foreach (var row in tab_rows)
                if (row.page == page) { dead = row; break; }
            if (dead != null) { tab_rows.remove (dead); tabs_box.remove (dead); }
        }

        private bool on_close_page (Adw.TabPage page) {
            tab_view.close_page_finish (page, true);
            if (tab_view.n_pages == 0) close ();
            return true;
        }

        private void on_selected_page_changed () {
            sync_active_row ();
            var tab = current_tab ();
            if (tab == null) {
                url_entry.text = "";
                back_button.sensitive = false;
                forward_button.sensitive = false;
                progress_bar.visible = false;
                sync_dl_button (null);
                return;
            }
            update_url_bar (tab.tab_uri);
            update_nav_buttons (tab);
            update_progress (tab.loading, tab.load_progress);
            sync_reload_icon (tab.loading);
            sync_dl_button (tab);
            this.title = tab.tab_title + " – CrowBrowser";
        }

        // ── Video downloader popover ──────────────────────────────────────────

        private Gtk.Popover build_dl_popover () {
            var pop = new Gtk.Popover ();
            pop.has_arrow = true;
            pop.width_request = 360;

            var outer = new Gtk.Box (Gtk.Orientation.VERTICAL, 0);

            dl_pop_header = new Gtk.Label ("Stiahnuť video");
            dl_pop_header.halign = Gtk.Align.START;
            dl_pop_header.add_css_class ("heading");
            dl_pop_header.margin_top = 12;
            dl_pop_header.margin_bottom = 8;
            dl_pop_header.margin_start = 14;
            dl_pop_header.margin_end = 14;
            outer.append (dl_pop_header);

            outer.append (new Gtk.Separator (Gtk.Orientation.HORIZONTAL));

            dl_pop_stack = new Gtk.Stack ();

            var empty_lbl = new Gtk.Label ("Žiadne videá neboli detekované\nna tejto stránke.");
            empty_lbl.wrap = true;
            empty_lbl.justify = Gtk.Justification.CENTER;
            empty_lbl.add_css_class ("dim-label");
            empty_lbl.margin_top = 20;
            empty_lbl.margin_bottom = 20;
            empty_lbl.margin_start = 16;
            empty_lbl.margin_end = 16;
            dl_pop_stack.add_named (empty_lbl, "empty");

            var list_scroll = new Gtk.ScrolledWindow ();
            list_scroll.set_policy (Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC);
            list_scroll.max_content_height = 300;
            list_scroll.propagate_natural_height = true;

            dl_pop_list = new Gtk.ListBox ();
            dl_pop_list.selection_mode = Gtk.SelectionMode.NONE;
            dl_pop_list.add_css_class ("boxed-list");
            dl_pop_list.margin_top = 8;
            dl_pop_list.margin_bottom = 6;
            dl_pop_list.margin_start = 8;
            dl_pop_list.margin_end = 8;
            list_scroll.set_child (dl_pop_list);
            dl_pop_stack.add_named (list_scroll, "list");
            dl_pop_stack.visible_child_name = "empty";
            outer.append (dl_pop_stack);

            outer.append (new Gtk.Separator (Gtk.Orientation.HORIZONTAL));

            var more_btn = new Gtk.Button.with_label ("Otvoriť sťahovač…");
            more_btn.add_css_class ("flat");
            more_btn.margin_top = 4;
            more_btn.margin_bottom = 4;
            more_btn.margin_start = 8;
            more_btn.margin_end = 8;
            more_btn.clicked.connect (() => {
                pop.popdown ();
                open_video_downloader_dialog ();
            });
            outer.append (more_btn);

            pop.set_child (outer);
            return pop;
        }

        private void rebuild_dl_popover (WebTab? tab) {
            var child = dl_pop_list.get_first_child ();
            while (child != null) {
                var next = child.get_next_sibling ();
                dl_pop_list.remove (child);
                child = next;
            }

            int count = (tab != null) ? tab.detected_video_urls.size : 0;
            if (count == 0) {
                dl_pop_header.label = "Stiahnuť video";
                dl_pop_stack.visible_child_name = "empty";
                return;
            }

            dl_pop_header.label = count == 1 ? "1 video nájdené" : @"$(count) videí nájdených";
            foreach (var url in tab.detected_video_urls)
                dl_pop_list.append (make_popover_video_row (url));
            dl_pop_stack.visible_child_name = "list";
        }

        private Gtk.Widget make_popover_video_row (string url) {
            var row = new Gtk.ListBoxRow ();
            row.activatable = false;

            var box = new Gtk.Box (Gtk.Orientation.HORIZONTAL, 8);
            box.margin_top = 7;
            box.margin_bottom = 7;
            box.margin_start = 8;
            box.margin_end = 4;

            var icon = new Gtk.Image.from_icon_name (pop_video_icon (url));
            icon.pixel_size = 16;
            icon.valign = Gtk.Align.CENTER;

            var text_box = new Gtk.Box (Gtk.Orientation.VERTICAL, 1);
            text_box.hexpand = true;

            var title_lbl = new Gtk.Label (pop_video_title (url));
            title_lbl.halign = Gtk.Align.START;
            title_lbl.ellipsize = Pango.EllipsizeMode.MIDDLE;
            title_lbl.max_width_chars = 28;

            string disp = url.length > 46 ? url.substring (0, 43) + "…" : url;
            var url_lbl = new Gtk.Label (disp);
            url_lbl.halign = Gtk.Align.START;
            url_lbl.add_css_class ("caption");
            url_lbl.add_css_class ("dim-label");
            url_lbl.ellipsize = Pango.EllipsizeMode.MIDDLE;
            url_lbl.max_width_chars = 30;

            text_box.append (title_lbl);
            text_box.append (url_lbl);

            var badge = new Gtk.Label (pop_video_badge (url));
            badge.valign = Gtk.Align.CENTER;
            badge.add_css_class ("crow-video-badge");

            var dl_btn = new Gtk.Button.from_icon_name ("folder-download-symbolic");
            dl_btn.add_css_class ("flat");
            dl_btn.valign = Gtk.Align.CENTER;
            dl_btn.tooltip_text = "Stiahnuť";
            string captured = url;
            dl_btn.clicked.connect (() => {
                dl_popover.popdown ();
                quick_download_url (captured);
            });

            box.append (icon);
            box.append (text_box);
            box.append (badge);
            box.append (dl_btn);
            row.set_child (box);
            return row;
        }

        private string pop_video_icon (string url) {
            string lo = url.down ();
            if (lo.contains (".m3u8") || lo.contains (".mpd"))
                return "multimedia-player-symbolic";
            if (lo.contains (".mp4") || lo.contains (".webm") || lo.contains (".mkv"))
                return "video-x-generic-symbolic";
            return "web-browser-symbolic";
        }

        private string pop_video_badge (string url) {
            string lo = url.down ();
            if (lo.contains (".m3u8")) return "HLS";
            if (lo.contains (".mpd"))  return "DASH";
            if (lo.contains (".mp4"))  return "MP4";
            if (lo.contains (".webm")) return "WebM";
            if (lo.contains (".mkv"))  return "MKV";
            if (lo.has_prefix ("blob:")) return "Stream";
            if (lo.contains ("filemoon"))   return "FileMoon";
            if (lo.contains ("streamtape")) return "Streamtape";
            if (lo.contains ("doodstream")) return "Dood";
            if (lo.contains ("streamwish") || lo.contains ("wishfast")) return "StreamWish";
            if (lo.contains ("voe.sx"))     return "VOE";
            if (lo.contains ("rumble"))     return "Rumble";
            if (lo.contains ("dailymotion")) return "Dailymotion";
            if (lo.contains ("ok.ru") || lo.contains ("okru")) return "OK.ru";
            return "Embed";
        }

        private string pop_video_title (string url) {
            try {
                var uri = GLib.Uri.parse (url, GLib.UriFlags.NONE);
                string path = uri.get_path () ?? "";
                string fname = GLib.Path.get_basename (path);
                if (fname.length > 1 && fname != "/" && fname.contains (".")) return fname;
                string host = uri.get_host () ?? "";
                return host.length > 0 ? host : url;
            } catch { return url.length > 40 ? url.substring (0, 37) + "…" : url; }
        }

        private void quick_download_url (string video_url) {
            var tab = current_tab ();
            string page = (tab != null) ? tab.tab_uri : "";
            new VideoDownloader (this, page, { video_url }).present (this);
        }

        private void open_video_downloader_dialog () {
            var tab = current_tab ();
            string url = (tab != null) ? tab.tab_uri : "";
            string[] detected = (tab != null) ? tab.detected_video_urls.to_array () : new string[0];
            new VideoDownloader (this, url, detected).present (this);
        }

        private void sync_dl_button (WebTab? tab) {
            int count = (tab != null) ? tab.detected_video_urls.size : 0;
            if (count == 0) {
                dl_button.remove_css_class ("crow-dl-active");
                dl_button.tooltip_text = "Stiahnuť video (Ctrl+D)";
                dl_badge.visible = false;
                dl_badge.label = "";
            } else {
                dl_button.add_css_class ("crow-dl-active");
                dl_button.tooltip_text = count == 1
                    ? "1 video nájdené — Ctrl+D"
                    : @"$(count) videí nájdených — Ctrl+D";
                dl_badge.label = count.to_string ();
                dl_badge.visible = true;
            }
        }

        private void sync_active_row () {
            var sel = tab_view.selected_page;
            foreach (var row in tab_rows) row.set_active (row.page == sel);
        }

        // ── Tab content signals ───────────────────────────────────────────────

        private void on_tab_title_changed (WebTab tab, string title) {
            row_for_tab (tab)?.update_title (title);
            if (tab == current_tab ()) this.title = title + " – CrowBrowser";
        }

        private void on_tab_uri_changed (WebTab tab, string uri) {
            if (tab == current_tab ()) update_url_bar (uri);
        }

        private void on_tab_loading_changed (WebTab tab, bool loading, double progress) {
            row_for_tab (tab)?.set_loading (loading);
            row_for_tab (tab)?.page.set_loading (loading);
            if (tab == current_tab ()) {
                update_nav_buttons (tab);
                update_progress (loading, progress);
                sync_reload_icon (loading);
                // Reset download button on new page navigation
                if (loading && progress < 0.06) sync_dl_button (tab);
            }
        }

        private void on_tab_favicon_changed (WebTab tab, Gdk.Texture? favicon) {
            row_for_tab (tab)?.update_favicon (favicon);
        }

        private void on_tab_video_detected (WebTab tab, string url) {
            if (tab != current_tab ()) return;
            int count = tab.detected_video_urls.size;
            sync_dl_button (tab);
            if (count == 1) {
                var toast = new Adw.Toast ("Video detekované na stránke");
                toast.button_label = "Stiahnuť";
                toast.button_clicked.connect (() => {
                    rebuild_dl_popover (current_tab ());
                    dl_popover.popup ();
                });
                toast.timeout = 4;
                toast_overlay.add_toast (toast);
            }
        }

        private void on_drm_blocked () {
            var toast = new Adw.Toast ("DRM zablokovaný — Widevine nie je nainštalovaný");
            toast.button_label = "Ako?";
            toast.button_clicked.connect (show_drm_info);
            toast.timeout = 6;
            toast_overlay.add_toast (toast);
        }

        // ── URL bar ──────────────────────────────────────────────────────────

        private void update_url_bar (string uri) {
            if (!url_entry.has_focus)
                url_entry.text = (uri == "about:blank" || uri == "") ? "" : uri;
        }

        private void update_nav_buttons (WebTab tab) {
            back_button.sensitive = tab.nav_can_go_back;
            forward_button.sensitive = tab.nav_can_go_forward;
        }

        private void update_progress (bool loading, double progress) {
            if (loading && progress > 0.0 && progress < 1.0) {
                progress_bar.visible = true;
                progress_bar.fraction = progress;
            } else {
                progress_bar.visible = false;
                progress_bar.fraction = 0.0;
            }
        }

        private void sync_reload_icon (bool loading) {
            reload_button.icon_name = loading ? "process-stop-symbolic" : "view-refresh-symbolic";
            reload_button.tooltip_text = loading ? "Zastaviť (Esc)" : "Obnoviť (F5)";
        }

        private void on_url_activated () {
            string text = url_entry.text.strip ();
            if (text.length == 0) return;
            var tab = current_tab ();
            if (tab == null) new_tab (text);
            else { tab.load_url (text); tab.web_view.grab_focus (); }
        }

        private void on_url_icon_pressed (Gtk.EntryIconPosition pos) {
            if (pos == Gtk.EntryIconPosition.PRIMARY) {
                url_entry.grab_focus ();
                url_entry.select_region (0, -1);
            }
        }

        // ── DRM info ─────────────────────────────────────────────────────────

        private void show_drm_info () {
            bool ok = WebTab.widevine_cdm_available ();
            var dlg = new Adw.Dialog ();
            dlg.title = "DRM / Widevine";
            dlg.set_content_width (460);
            dlg.set_content_height (340);

            var tv = new Adw.ToolbarView ();
            tv.add_top_bar (new Adw.HeaderBar ());

            var box = new Gtk.Box (Gtk.Orientation.VERTICAL, 14);
            box.margin_top = 20; box.margin_bottom = 20;
            box.margin_start = 24; box.margin_end = 24;

            var ico = new Gtk.Image.from_icon_name (
                ok ? "security-high-symbolic" : "security-low-symbolic");
            ico.pixel_size = 48;
            ico.add_css_class (ok ? "success" : "warning");

            var lbl = new Gtk.Label (ok
                ? "Widevine CDM nájdený.\nNetflix, Disney+, Spotify a podobné stránky by mali fungovať."
                : "Widevine CDM nie je nainštalovaný.\nDRM obsah (Netflix, Disney+…) sa neprehrá.");
            lbl.wrap = true;
            lbl.justify = Gtk.Justification.CENTER;
            box.append (ico);
            box.append (lbl);

            if (!ok) {
                box.append (new Gtk.Separator (Gtk.Orientation.HORIZONTAL));
                var instructions = new Gtk.Label ("Inštalácia na Arch Linux:");
                instructions.halign = Gtk.Align.START;
                instructions.add_css_class ("heading");
                box.append (instructions);

                var txt = new Gtk.TextView ();
                txt.buffer.text =
                    "Možnosť 1 (AUR — odporúčané):\n" +
                    "  yay -S chromium-widevine\n\n" +
                    "Možnosť 2 — Chromium:\n" +
                    "  sudo pacman -S chromium\n\n" +
                    "Možnosť 3 — Google Chrome:\n" +
                    "  yay -S google-chrome\n\n" +
                    "Reštartuj CrowBrowser po inštalácii.";
                txt.editable = false;
                txt.cursor_visible = false;
                txt.monospace = true;
                txt.add_css_class ("card");

                var sc = new Gtk.ScrolledWindow ();
                sc.vexpand = true;
                sc.set_child (txt);
                box.append (sc);
            }

            var sc2 = new Gtk.ScrolledWindow ();
            sc2.set_child (box);
            sc2.vexpand = true;
            tv.content = sc2;
            dlg.set_child (tv);
            dlg.present (this);
        }

        // ── Video download ────────────────────────────────────────────────────

        private void on_download_video_clicked () {
            rebuild_dl_popover (current_tab ());
            dl_popover.popup ();
        }

        // ── History ──────────────────────────────────────────────────────────

        private void show_history () {
            var dlg = new HistoryDialog ();
            dlg.navigate_to.connect ((url) => {
                var tab = current_tab ();
                if (tab != null) tab.load_url (url);
                else new_tab (url);
            });
            dlg.present (this);
        }

        // ── Downloads ─────────────────────────────────────────────────────────

        private void show_downloads () {
            new DownloadsDialog ().present (this);
        }

        // ── Settings ──────────────────────────────────────────────────────────

        private void show_settings () {
            new SettingsDialog ().present (this);
        }

        // ── About ─────────────────────────────────────────────────────────────

        private void show_about () {
            var about = new Adw.AboutDialog ();
            about.application_name = "CrowBrowser";
            about.version = "1.0.0";
            about.developer_name = "CrowBrowser Project";
            about.license_type = Gtk.License.GPL_3_0;
            about.comments = "Moderný webový prehliadač pre Linux\npostavený na WebKitGTK 6 a Libadwaita.";
            about.present (this);
        }
    }
}
