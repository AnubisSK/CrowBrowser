namespace CrowBrowser {

    public class HistoryDialog : Adw.Dialog {

        private Gtk.ListBox list_box;
        private Gtk.SearchEntry search_entry;
        private HistoryManager history;

        public signal void navigate_to (string url);

        public HistoryDialog () {
            Object (title: "História");
            set_content_width (640);
            set_content_height (620);
            history = HistoryManager.get_instance ();
            build_content ();
        }

        private void build_content () {
            var toolbar = new Adw.ToolbarView ();

            var header = new Adw.HeaderBar ();
            search_entry = new Gtk.SearchEntry ();
            search_entry.hexpand = true;
            search_entry.placeholder_text = "Hľadaj v histórii…";
            search_entry.search_changed.connect (() => rebuild_list (search_entry.text));
            header.set_title_widget (search_entry);
            toolbar.add_top_bar (header);

            var scroll = new Gtk.ScrolledWindow ();
            scroll.set_policy (Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC);
            scroll.vexpand = true;

            list_box = new Gtk.ListBox ();
            list_box.add_css_class ("boxed-list");
            list_box.margin_top    = 12;
            list_box.margin_bottom = 12;
            list_box.margin_start  = 16;
            list_box.margin_end    = 16;
            scroll.set_child (list_box);
            toolbar.content = scroll;

            var bar = new Gtk.ActionBar ();
            var clear_btn = new Gtk.Button.with_label ("Vymazať celú históriu");
            clear_btn.add_css_class ("destructive-action");
            clear_btn.clicked.connect (() => {
                history.clear ();
                rebuild_list ("");
            });
            bar.pack_start (clear_btn);
            toolbar.add_bottom_bar (bar);

            set_child (toolbar);
            rebuild_list ("");
        }

        private void rebuild_list (string query) {
            var child = list_box.get_first_child ();
            while (child != null) {
                var next = child.get_next_sibling ();
                list_box.remove (child);
                child = next;
            }

            string q = query.strip ().down ();
            var all = history.get_entries_reversed ();

            int shown = 0;
            string cur_day = "";

            foreach (var e in all) {
                if (q.length > 0) {
                    if (!e.title.down ().contains (q) && !e.url.down ().contains (q)) continue;
                }

                var dt = new GLib.DateTime.from_unix_local (e.timestamp);
                string day = dt.format ("%A, %d. %B %Y");
                if (day != cur_day) {
                    cur_day = day;
                    var sep_row = new Gtk.ListBoxRow ();
                    sep_row.selectable = false;
                    sep_row.activatable = false;
                    var sep_lbl = new Gtk.Label (day);
                    sep_lbl.halign = Gtk.Align.START;
                    sep_lbl.add_css_class ("heading");
                    sep_lbl.margin_top    = 10;
                    sep_lbl.margin_bottom =  4;
                    sep_lbl.margin_start  =  4;
                    sep_row.set_child (sep_lbl);
                    list_box.append (sep_row);
                }

                var row = new Adw.ActionRow ();
                string time_str = dt.format ("%H:%M");
                row.title    = GLib.Markup.escape_text (e.title);
                row.subtitle = time_str + "  ·  " + GLib.Markup.escape_text (e.url);
                row.activatable = true;

                var open_btn = new Gtk.Button.from_icon_name ("go-jump-symbolic");
                open_btn.add_css_class ("flat");
                open_btn.valign = Gtk.Align.CENTER;
                open_btn.tooltip_text = "Otvoriť";
                string captured_url = e.url;
                open_btn.clicked.connect (() => {
                    close ();
                    navigate_to (captured_url);
                });
                row.add_suffix (open_btn);
                row.activated.connect (() => {
                    close ();
                    navigate_to (captured_url);
                });
                list_box.append (row);

                shown++;
                if (shown >= 500) break;
            }

            if (shown == 0) {
                var empty_row = new Gtk.ListBoxRow ();
                empty_row.selectable  = false;
                empty_row.activatable = false;
                var lbl = new Gtk.Label (q.length > 0 ? "Žiadne výsledky" : "História je prázdna");
                lbl.add_css_class ("dim-label");
                lbl.margin_top    = 24;
                lbl.margin_bottom = 24;
                empty_row.set_child (lbl);
                list_box.append (empty_row);
            }
        }
    }
}
