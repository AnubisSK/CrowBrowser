namespace CrowBrowser {

    public class DownloadsDialog : Adw.Dialog {

        private Gtk.Box main_box;
        private Gtk.Box active_box;
        private Gtk.Label active_header;
        private Gtk.ListBox record_list;
        private DownloadManager dm;
        private ulong changed_id = 0;

        public DownloadsDialog () {
            Object (title: "Správca sťahovania");
            set_content_width (580);
            set_content_height (540);
            dm = DownloadManager.get_instance ();
            build_content ();
            rebuild ();
            changed_id = dm.changed.connect (rebuild);
        }

        public override void closed () {
            if (changed_id != 0) {
                dm.disconnect (changed_id);
                changed_id = 0;
            }
            base.closed ();
        }

        private void build_content () {
            var toolbar = new Adw.ToolbarView ();
            toolbar.add_top_bar (new Adw.HeaderBar ());

            var scroll = new Gtk.ScrolledWindow ();
            scroll.set_policy (Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC);
            scroll.vexpand = true;

            main_box = new Gtk.Box (Gtk.Orientation.VERTICAL, 0);
            main_box.margin_top    = 12;
            main_box.margin_bottom = 12;
            main_box.margin_start  = 16;
            main_box.margin_end    = 16;

            // Active downloads section
            active_header = new Gtk.Label ("Prebieha sťahovanie");
            active_header.halign = Gtk.Align.START;
            active_header.add_css_class ("heading");
            active_header.margin_bottom = 6;

            active_box = new Gtk.Box (Gtk.Orientation.VERTICAL, 4);
            active_box.add_css_class ("boxed-list");
            active_box.margin_bottom = 16;

            // Completed section
            var rec_header = new Gtk.Label ("Nedávne sťahovania");
            rec_header.halign = Gtk.Align.START;
            rec_header.add_css_class ("heading");
            rec_header.margin_top    = 4;
            rec_header.margin_bottom = 6;

            record_list = new Gtk.ListBox ();
            record_list.selection_mode = Gtk.SelectionMode.NONE;
            record_list.add_css_class ("boxed-list");

            main_box.append (active_header);
            main_box.append (active_box);
            main_box.append (rec_header);
            main_box.append (record_list);

            scroll.set_child (main_box);
            toolbar.content = scroll;

            var bar = new Gtk.ActionBar ();
            var clear_btn = new Gtk.Button.with_label ("Vymazať históriu");
            clear_btn.add_css_class ("flat");
            clear_btn.clicked.connect (() => {
                dm.clear_records ();
                rebuild ();
            });
            bar.pack_start (clear_btn);
            toolbar.add_bottom_bar (bar);

            set_child (toolbar);
        }

        private void rebuild () {
            // Rebuild active downloads
            var ac = active_box.get_first_child ();
            while (ac != null) { var n = ac.get_next_sibling (); active_box.remove (ac); ac = n; }

            var active = dm.get_active ();
            active_header.visible = (active.size > 0);
            active_box.visible    = (active.size > 0);

            foreach (var dl in active) {
                active_box.append (make_active_row (dl));
            }

            // Rebuild completed records
            var rc = record_list.get_first_child ();
            while (rc != null) { var n = rc.get_next_sibling (); record_list.remove (rc); rc = n; }

            var records = dm.get_records_reversed ();
            if (records.size == 0 && active.size == 0) {
                var empty_row = new Gtk.ListBoxRow ();
                empty_row.selectable  = false;
                empty_row.activatable = false;
                var lbl = new Gtk.Label ("Žiadne sťahovania");
                lbl.add_css_class ("dim-label");
                lbl.margin_top    = 20;
                lbl.margin_bottom = 20;
                empty_row.set_child (lbl);
                record_list.append (empty_row);
                return;
            }

            foreach (var r in records) {
                record_list.append (make_record_row (r));
            }
        }

        private Gtk.Widget make_active_row (WebKit.Download dl) {
            var row = new Adw.ActionRow ();
            string fname = GLib.Path.get_basename (dl.get_destination () ?? dl.get_request ()?.get_uri () ?? "");
            row.title = fname.length > 0 ? fname : "Sťahujem…";

            var progress = new Gtk.ProgressBar ();
            progress.valign = Gtk.Align.CENTER;
            progress.hexpand = true;
            double p = dl.estimated_progress;
            if (p > 0 && p < 1.0) {
                progress.fraction = p;
                progress.text = "%.0f%%".printf (p * 100.0);
            } else {
                progress.pulse ();
                progress.text = "Sťahujem…";
            }
            progress.show_text = true;

            var cancel_btn = new Gtk.Button.from_icon_name ("process-stop-symbolic");
            cancel_btn.add_css_class ("flat");
            cancel_btn.valign = Gtk.Align.CENTER;
            cancel_btn.tooltip_text = "Zrušiť";
            cancel_btn.clicked.connect (() => dl.cancel ());

            row.add_suffix (progress);
            row.add_suffix (cancel_btn);
            return row;
        }

        private Gtk.Widget make_record_row (DownloadRecord r) {
            var row = new Adw.ActionRow ();
            row.title = GLib.Markup.escape_text (r.filename.length > 0 ? r.filename : r.url);

            var dt = new GLib.DateTime.from_unix_local (r.timestamp);
            string time_str = dt.format ("%d.%m.%Y %H:%M");
            string size_str = r.file_size > 0 ? format_size (r.file_size) + "  ·  " : "";
            row.subtitle = size_str + time_str;

            var icon = new Gtk.Image.from_icon_name (
                r.failed ? "dialog-error-symbolic" : "emblem-ok-symbolic"
            );
            icon.pixel_size = 16;
            icon.valign = Gtk.Align.CENTER;
            if (!r.failed) icon.add_css_class ("success");
            else           icon.add_css_class ("error");
            row.add_prefix (icon);

            if (!r.failed && r.dest_path.length > 0) {
                var open_btn = new Gtk.Button.from_icon_name ("folder-open-symbolic");
                open_btn.add_css_class ("flat");
                open_btn.valign = Gtk.Align.CENTER;
                open_btn.tooltip_text = "Otvoriť priečinok";
                string path = r.dest_path;
                open_btn.clicked.connect (() => {
                    try {
                        var launcher = new Gtk.FileLauncher (GLib.File.new_for_path (path));
                        launcher.open_containing_folder.begin (null, null, null);
                    } catch {}
                });
                row.add_suffix (open_btn);
            }

            return row;
        }

        private string format_size (int64 bytes) {
            if (bytes < 1024)       return @"$(bytes) B";
            if (bytes < 1024*1024)  return "%.1f KB".printf (bytes / 1024.0);
            if (bytes < 1024*1024*1024) return "%.1f MB".printf (bytes / (1024.0*1024));
            return "%.2f GB".printf (bytes / (1024.0*1024*1024));
        }
    }
}
