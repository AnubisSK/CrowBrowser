namespace CrowBrowser {

    public class VideoDownloader : Adw.Dialog {

        private Gtk.Entry url_entry;
        private Gtk.DropDown format_dropdown;
        private Gtk.Label output_dir_label;
        private Gtk.TextView log_view;
        private Gtk.Button download_btn;
        private Gtk.Button cancel_btn;
        private Gtk.Stack button_stack;
        private Gtk.ProgressBar dl_progress;

        private string output_dir;
        private string page_url;
        private string[] video_urls;
        private GLib.Subprocess? current_process = null;
        private bool is_running = false;

        private static string[] FORMAT_LABELS = {
            "Najlepšia kvalita (video+audio)",
            "MP4 – H.264 + AAC",
            "WebM – VP9 + Opus",
            "Len audio – MP3",
            "Len audio – Opus",
            "720p max",
            "1080p max",
            "4K max",
        };

        private static string[] FORMAT_VALUES = {
            "bestvideo+bestaudio/best",
            "bestvideo[ext=mp4]+bestaudio[ext=m4a]/best[ext=mp4]/best",
            "bestvideo[ext=webm]+bestaudio[ext=webm]/best[ext=webm]/best",
            "bestaudio/best -x --audio-format mp3",
            "bestaudio/best -x --audio-format opus",
            "bestvideo[height<=720]+bestaudio/best[height<=720]",
            "bestvideo[height<=1080]+bestaudio/best[height<=1080]",
            "bestvideo[height<=2160]+bestaudio/best[height<=2160]",
        };

        public VideoDownloader (Gtk.Window parent, string initial_url,
                                string[] detected_urls = {}) {
            Object (title: "Stiahnuť video");
            set_content_width (600);
            set_content_height (580);
            output_dir = GLib.Environment.get_user_special_dir (GLib.UserDirectory.DOWNLOAD);
            page_url = (initial_url != "about:blank") ? initial_url : "";
            video_urls = detected_urls;
            build_content (initial_url);
        }

        private void build_content (string initial_url) {
            var toolbar = new Adw.ToolbarView ();
            toolbar.add_top_bar (new Adw.HeaderBar ());

            var outer_scroll = new Gtk.ScrolledWindow ();
            outer_scroll.set_policy (Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC);
            outer_scroll.vexpand = true;

            var main_box = new Gtk.Box (Gtk.Orientation.VERTICAL, 14);
            main_box.margin_top = 14;
            main_box.margin_bottom = 14;
            main_box.margin_start = 20;
            main_box.margin_end = 20;

            // Create url_entry early so list row_selected handler can write to it
            url_entry = new Gtk.Entry ();
            url_entry.hexpand = true;
            url_entry.placeholder_text = "https://…";
            url_entry.input_purpose = Gtk.InputPurpose.URL;
            url_entry.activate.connect (start_download);

            // ── Graphical video selection list ─────────────────────────────────
            if (video_urls.length > 0) {
                var section_lbl = new Gtk.Label ("Vyber video na stiahnutie:");
                section_lbl.halign = Gtk.Align.START;
                section_lbl.add_css_class ("heading");
                main_box.append (section_lbl);

                var list = new Gtk.ListBox ();
                list.selection_mode = Gtk.SelectionMode.SINGLE;
                list.add_css_class ("boxed-list");
                list.margin_bottom = 2;

                foreach (var vurl in video_urls) {
                    list.append (make_video_row (vurl));
                }

                // Pre-select the best URL
                string best = pick_best_url (video_urls, initial_url);
                url_entry.text = best;
                for (int i = 0; i < video_urls.length; i++) {
                    if (video_urls[i] == best) {
                        var row = list.get_row_at_index (i);
                        if (row != null) list.select_row (row);
                        break;
                    }
                }

                list.row_selected.connect ((row) => {
                    if (row == null) return;
                    int idx = row.get_index ();
                    if (idx >= 0 && idx < video_urls.length)
                        url_entry.text = video_urls[idx];
                });

                main_box.append (list);

                var custom_lbl = new Gtk.Label ("Vlastná URL (ak nechceš žiadnu z vyššie):");
                custom_lbl.halign = Gtk.Align.START;
                custom_lbl.add_css_class ("caption");
                main_box.append (custom_lbl);
            } else {
                var url_label = new Gtk.Label ("URL videa alebo stránky:");
                url_label.halign = Gtk.Align.START;
                url_label.add_css_class ("caption");
                main_box.append (url_label);

                url_entry.text = (initial_url != "about:blank") ? initial_url : "";
            }

            main_box.append (url_entry);

            // ── Format selector ────────────────────────────────────────────────
            var fmt_label = new Gtk.Label ("Formát / Kvalita:");
            fmt_label.halign = Gtk.Align.START;
            fmt_label.add_css_class ("caption");

            var fmt_model = new Gtk.StringList (FORMAT_LABELS);
            format_dropdown = new Gtk.DropDown (fmt_model, null);
            format_dropdown.selected = 0;

            main_box.append (fmt_label);
            main_box.append (format_dropdown);

            // ── Output directory ───────────────────────────────────────────────
            var dir_box = new Gtk.Box (Gtk.Orientation.HORIZONTAL, 8);
            var dir_title = new Gtk.Label ("Uložiť do:");
            dir_title.halign = Gtk.Align.START;
            output_dir_label = new Gtk.Label (output_dir);
            output_dir_label.hexpand = true;
            output_dir_label.halign = Gtk.Align.START;
            output_dir_label.ellipsize = Pango.EllipsizeMode.START;
            var dir_btn = new Gtk.Button.with_label ("Zmeniť…");
            dir_btn.clicked.connect (choose_output_dir);

            dir_box.append (dir_title);
            dir_box.append (output_dir_label);
            dir_box.append (dir_btn);
            main_box.append (dir_box);

            // ── yt-dlp status ──────────────────────────────────────────────────
            string? ytdlp_path = GLib.Environment.find_program_in_path ("yt-dlp");
            if (ytdlp_path == null) {
                var warn_box = new Gtk.Box (Gtk.Orientation.HORIZONTAL, 8);
                warn_box.add_css_class ("card");
                var warn_ico = new Gtk.Image.from_icon_name ("dialog-warning-symbolic");
                var warn_lbl = new Gtk.Label (
                    "yt-dlp nie je nainštalovaný. Pre stiahnutie spusti:\n" +
                    "  sudo pacman -S yt-dlp"
                );
                warn_lbl.halign = Gtk.Align.START;
                warn_lbl.xalign = 0;
                warn_lbl.use_markup = false;
                warn_box.margin_top = 4;
                warn_box.margin_bottom = 4;
                warn_box.margin_start = 8;
                warn_box.margin_end = 8;
                warn_box.append (warn_ico);
                warn_box.append (warn_lbl);
                main_box.append (warn_box);
            }

            // ── Progress ───────────────────────────────────────────────────────
            dl_progress = new Gtk.ProgressBar ();
            dl_progress.visible = false;
            dl_progress.show_text = true;
            main_box.append (dl_progress);

            // ── Log output ─────────────────────────────────────────────────────
            log_view = new Gtk.TextView ();
            log_view.editable = false;
            log_view.cursor_visible = false;
            log_view.monospace = true;
            log_view.wrap_mode = Gtk.WrapMode.CHAR;
            log_view.add_css_class ("card");

            var log_scroll = new Gtk.ScrolledWindow ();
            log_scroll.set_policy (Gtk.PolicyType.AUTOMATIC, Gtk.PolicyType.AUTOMATIC);
            log_scroll.vexpand = true;
            log_scroll.min_content_height = 100;
            log_scroll.set_child (log_view);
            main_box.append (log_scroll);

            outer_scroll.set_child (main_box);
            toolbar.content = outer_scroll;

            // ── Buttons ────────────────────────────────────────────────────────
            download_btn = new Gtk.Button.with_label ("Stiahnuť");
            download_btn.add_css_class ("suggested-action");
            download_btn.add_css_class ("pill");
            download_btn.clicked.connect (start_download);

            cancel_btn = new Gtk.Button.with_label ("Zrušiť");
            cancel_btn.add_css_class ("destructive-action");
            cancel_btn.add_css_class ("pill");
            cancel_btn.clicked.connect (cancel_download);

            button_stack = new Gtk.Stack ();
            button_stack.add_named (download_btn, "download");
            button_stack.add_named (cancel_btn, "cancel");
            button_stack.visible_child_name = "download";

            var bar = new Gtk.ActionBar ();
            bar.pack_end (button_stack);
            toolbar.add_bottom_bar (bar);

            set_child (toolbar);
        }

        // ── Video row (graphical list entry) ──────────────────────────────────

        private Gtk.ListBoxRow make_video_row (string url) {
            var row = new Adw.ActionRow ();
            row.title = GLib.Markup.escape_text (video_title (url));
            string disp_url = url.length > 72 ? url.substring (0, 69) + "…" : url;
            row.subtitle = GLib.Markup.escape_text (disp_url);
            row.tooltip_text = url;
            row.activatable = true;

            var icon = new Gtk.Image.from_icon_name (video_type_icon (url));
            icon.pixel_size = 22;
            icon.valign = Gtk.Align.CENTER;
            row.add_prefix (icon);

            var badge = new Gtk.Label (video_type_label (url));
            badge.valign = Gtk.Align.CENTER;
            badge.add_css_class ("caption");
            badge.add_css_class ("crow-video-badge");
            row.add_suffix (badge);

            return row;
        }

        private string video_type_icon (string url) {
            string lo = url.down ();
            if (lo.contains (".m3u8") || lo.contains (".mpd"))
                return "multimedia-player-symbolic";
            if (lo.contains (".mp4") || lo.contains (".webm") ||
                lo.contains (".mkv") || lo.contains (".mov") ||
                lo.contains (".avi") || lo.contains (".m4v"))
                return "video-x-generic-symbolic";
            if (lo.has_prefix ("blob:"))
                return "multimedia-player-symbolic";
            return "web-browser-symbolic";
        }

        private string video_type_label (string url) {
            string lo = url.down ();
            if (lo.contains (".m3u8")) return "HLS";
            if (lo.contains (".mpd")) return "DASH";
            if (lo.contains (".mp4")) return "MP4";
            if (lo.contains (".webm")) return "WebM";
            if (lo.contains (".mkv")) return "MKV";
            if (lo.contains (".mov")) return "MOV";
            if (lo.has_prefix ("blob:")) return "Stream";
            // Known video hosters — show recognisable name
            if (lo.contains ("filemoon")) return "FileMoon";
            if (lo.contains ("streamtape")) return "Streamtape";
            if (lo.contains ("doodstream")) return "Dood";
            if (lo.contains ("mixdrop")) return "Mixdrop";
            if (lo.contains ("streamwish") || lo.contains ("wishfast")) return "StreamWish";
            if (lo.contains ("vidhide")) return "VidHide";
            if (lo.contains ("vidmoly")) return "VidMoly";
            if (lo.contains ("streamzz") || lo.contains ("streamvid")) return "StreamZZ";
            if (lo.contains ("voe.sx")) return "VOE";
            if (lo.contains ("mp4upload")) return "MP4Upload";
            if (lo.contains ("uqload")) return "Uqload";
            if (lo.contains ("supervideo")) return "SuperVideo";
            if (lo.contains ("rumble")) return "Rumble";
            if (lo.contains ("dailymotion")) return "Dailymotion";
            if (lo.contains ("ok.ru") || lo.contains ("okru")) return "OK.ru";
            return "Embed";
        }

        private string video_title (string url) {
            try {
                var uri = GLib.Uri.parse (url, GLib.UriFlags.NONE);
                string path = uri.get_path () ?? "";
                string fname = GLib.Path.get_basename (path);
                if (fname.length > 1 && fname != "/" && fname.contains ("."))
                    return fname;
                string host = uri.get_host () ?? "";
                return host.length > 0 ? host : url;
            } catch {
                return url.length > 50 ? url.substring (0, 47) + "…" : url;
            }
        }

        // ── Output directory ───────────────────────────────────────────────────

        private void choose_output_dir () {
            var chooser = new Gtk.FileDialog ();
            chooser.title = "Vyberte priečinok";
            chooser.select_folder.begin (null, null, (obj, res) => {
                try {
                    var folder = chooser.select_folder.end (res);
                    output_dir = folder.get_path ();
                    output_dir_label.label = output_dir;
                } catch (Error e) { /* cancelled */ }
            });
        }

        // ── Download logic ─────────────────────────────────────────────────────

        // true = last run used ffmpeg (affects progress parsing)
        private bool using_ffmpeg = false;

        private void start_download () {
            string url = url_entry.text.strip ();
            if (url.length == 0) { append_log ("URL je prázdna.\n"); return; }

            string lo = url.down ();
            bool is_stream = lo.contains (".m3u8") || lo.contains (".mpd");

            string[] argv;

            if (is_stream) {
                // Direct HLS/DASH stream → prefer ffmpeg (handles token auth reliably)
                string? ffmpeg = GLib.Environment.find_program_in_path ("ffmpeg");
                if (ffmpeg != null) {
                    argv = build_ffmpeg_argv (ffmpeg, url);
                    using_ffmpeg = true;
                } else {
                    // ffmpeg missing — fall back to yt-dlp
                    string? ytdlp = GLib.Environment.find_program_in_path ("yt-dlp");
                    if (ytdlp == null) {
                        append_log ("Nainštaluj ffmpeg (odporúčané) alebo yt-dlp:\n");
                        append_log ("  sudo pacman -S ffmpeg\n");
                        return;
                    }
                    argv = build_ytdlp_argv (ytdlp, url);
                    using_ffmpeg = false;
                }
            } else {
                // Page URL or direct file → yt-dlp handles extraction
                string? ytdlp = GLib.Environment.find_program_in_path ("yt-dlp");
                if (ytdlp == null) {
                    append_log ("yt-dlp nie je nainštalovaný:\n  sudo pacman -S yt-dlp\n");
                    return;
                }
                argv = build_ytdlp_argv (ytdlp, url);
                using_ffmpeg = false;
            }

            clear_log ();
            append_log (url + "\n\n");

            is_running = true;
            button_stack.visible_child_name = "cancel";
            dl_progress.visible = true;
            dl_progress.fraction = 0.0;
            dl_progress.text = "Sťahujem…";

            try {
                current_process = new GLib.Subprocess.newv (
                    argv,
                    GLib.SubprocessFlags.STDOUT_PIPE | GLib.SubprocessFlags.STDERR_MERGE
                );
                read_output_async.begin ();
            } catch (Error e) {
                append_log ("Chyba: " + e.message + "\n");
                on_finished (false);
            }
        }

        private string[] build_ffmpeg_argv (string ffmpeg, string url) {
            string fname = stream_url_to_filename (url);
            string output = GLib.Path.build_filename (output_dir, fname);

            string[] argv = { ffmpeg, "-y", "-loglevel", "info", "-stats" };

            // Send Referer + Origin so CDN token auth passes
            if (page_url.length > 0) {
                string origin = url_origin (page_url);
                argv += "-headers";
                argv += "Referer: " + page_url + "\r\nOrigin: " + origin + "\r\n";
            }

            argv += "-i";
            argv += url;
            argv += "-c";
            argv += "copy";
            argv += "-bsf:a";
            argv += "aac_adtstoasc";
            argv += output;
            return argv;
        }

        private string[] build_ytdlp_argv (string ytdlp, string url) {
            uint sel = format_dropdown.selected;
            string fmt_raw = (sel < FORMAT_VALUES.length) ? FORMAT_VALUES[sel] : FORMAT_VALUES[0];
            string output_tmpl = GLib.Path.build_filename (output_dir, "%(title)s.%(ext)s");

            string[] argv = { ytdlp, "--newline", "-o", output_tmpl,
                              "--no-warnings", "--ignore-errors" };

            if (page_url.length > 0) {
                argv += "--referer";
                argv += page_url;
                argv += "--add-header";
                argv += "Origin:" + url_origin (page_url);
            }

            string[] fmt_parts = fmt_raw.split (" ");
            bool fmt_added = false;
            foreach (string p in fmt_parts) {
                string t = p.strip ();
                if (t.length == 0) continue;
                if (t.has_prefix ("-")) {
                    argv += t;
                } else if (!fmt_added) {
                    argv += "-f"; argv += t;
                    fmt_added = true;
                } else {
                    argv += t;
                }
            }
            argv += url;
            return argv;
        }

        // ── Async output reader (shared by both ffmpeg and yt-dlp) ─────────────

        private async void read_output_async () {
            if (current_process == null) return;
            var reader = new GLib.DataInputStream (current_process.get_stdout_pipe ());

            try {
                while (true) {
                    string? line = yield reader.read_line_async (GLib.Priority.DEFAULT, null);
                    if (line == null) break;
                    Idle.add (() => {
                        append_log (line + "\n");
                        parse_progress (line);
                        return false;
                    });
                }

                bool ok = yield current_process.wait_check_async (null);
                Idle.add (() => {
                    if (ok) {
                        dl_progress.fraction = 1.0;
                        dl_progress.text = "Dokončené";
                        append_log ("\nHotovo – uložené v: " + output_dir + "\n");
                    } else {
                        append_log ("\nSkončilo s chybou.\n");
                    }
                    on_finished (ok);
                    return false;
                });
            } catch (Error e) {
                string errmsg = e.message;
                Idle.add (() => {
                    if (is_running) {
                        append_log ("Prerušené: " + errmsg + "\n");
                        on_finished (false);
                    }
                    return false;
                });
            }
        }

        private void parse_progress (string line) {
            if (using_ffmpeg) {
                // ffmpeg: "frame= 100 fps=24 … time=00:01:23.42 …"
                int tp = line.index_of ("time=");
                if (tp >= 0) {
                    string t = line.substring (tp + 5).split (" ")[0];
                    if (t.length >= 8) {
                        dl_progress.text = t.substring (0, 8);
                        dl_progress.pulse ();
                    }
                }
            } else {
                // yt-dlp: "[download]  42.3% of ..."
                if (!("[download]" in line) || !("%" in line)) return;
                int pct_pos = line.index_of ("%");
                if (pct_pos <= 0) return;
                int start = pct_pos - 1;
                while (start > 0 && (line[start - 1].isdigit () || line[start - 1] == '.'))
                    start--;
                double pct = double.parse (line.substring (start, pct_pos - start)) / 100.0;
                if (pct >= 0.0 && pct <= 1.0) {
                    dl_progress.fraction = pct;
                    dl_progress.text = "%.1f%%".printf (pct * 100.0);
                }
            }
        }

        private void cancel_download () {
            if (current_process != null) {
                current_process.force_exit ();
                current_process = null;
            }
            append_log ("\nZrušené.\n");
            on_finished (false);
        }

        private void on_finished (bool success) {
            is_running = false;
            current_process = null;
            button_stack.visible_child_name = "download";
            if (!success) dl_progress.visible = false;
        }

        // ── URL helpers ────────────────────────────────────────────────────────

        private string stream_url_to_filename (string url) {
            try {
                var uri = GLib.Uri.parse (url, GLib.UriFlags.NONE);
                string path = uri.get_path () ?? "";
                string stem = GLib.Path.get_basename (path);
                int dot = stem.last_index_of_char ('.');
                if (dot > 0) stem = stem.substring (0, dot);
                if (stem.length > 2) return stem + ".mp4";
            } catch (Error e) {}
            return "stream_%lld.mp4".printf (GLib.get_real_time () / 1000000);
        }

        private string url_origin (string url) {
            try {
                var uri = GLib.Uri.parse (url, GLib.UriFlags.NONE);
                string scheme = uri.get_scheme () ?? "https";
                string host   = uri.get_host ()   ?? "";
                int    port   = uri.get_port ();
                return scheme + "://" + host + (port > 0 ? ":" + port.to_string () : "");
            } catch (Error e) { return url; }
        }

        private void append_log (string text) {
            var buf = log_view.buffer;
            Gtk.TextIter iter;
            buf.get_end_iter (out iter);
            buf.insert (ref iter, text, -1);
            log_view.scroll_to_mark (buf.get_insert (), 0.0, false, 0.0, 1.0);
        }

        private void clear_log () {
            log_view.buffer.text = "";
        }

        private string pick_best_url (string[] detected, string page_url) {
            foreach (var u in detected) {
                string lo = u.down ();
                if (lo.contains (".mp4") || lo.contains (".webm") ||
                    lo.contains (".mkv") || lo.contains (".m3u8") ||
                    lo.contains (".mpd") || lo.contains (".mov") ||
                    lo.contains (".avi") || lo.contains (".m4v")) {
                    return u;
                }
            }
            if (detected.length > 0) return detected[0];
            return (page_url != "about:blank") ? page_url : "";
        }
    }
}
