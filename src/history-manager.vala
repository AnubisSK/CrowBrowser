namespace CrowBrowser {

    public struct HistoryEntry {
        public string url;
        public string title;
        public int64 timestamp;
    }

    public class HistoryManager : Object {

        private static HistoryManager? _instance = null;

        public static HistoryManager get_instance () {
            if (_instance == null) _instance = new HistoryManager ();
            return _instance;
        }

        private Gee.ArrayList<HistoryEntry?> entries;
        private string history_file;
        private bool dirty = false;

        construct {
            entries = new Gee.ArrayList<HistoryEntry?> ();
            string dir = GLib.Path.build_filename (
                GLib.Environment.get_user_data_dir (), "crow-browser"
            );
            GLib.DirUtils.create_with_parents (dir, 0755);
            history_file = GLib.Path.build_filename (dir, "history");
            load ();
            GLib.Timeout.add_seconds (30, () => {
                if (dirty) save ();
                return GLib.Source.CONTINUE;
            });
        }

        public void add (string url, string title) {
            if (url == "about:blank" || url == "" ||
                url.has_prefix ("data:") || url.has_prefix ("view-source:")) return;
            var e = HistoryEntry ();
            e.url = url;
            e.title = (title.length > 0) ? title : url;
            e.timestamp = GLib.get_real_time () / 1000000;
            entries.add (e);
            while (entries.size > 10000) entries.remove_at (0);
            dirty = true;
        }

        public Gee.ArrayList<HistoryEntry?> get_entries_reversed () {
            var rev = new Gee.ArrayList<HistoryEntry?> ();
            for (int i = entries.size - 1; i >= 0; i--)
                rev.add (entries[i]);
            return rev;
        }

        public int size () { return entries.size; }

        public void clear () {
            entries.clear ();
            dirty = false;
            save ();
        }

        // Called by Application.shutdown() to flush before exit
        public void save_now () {
            save ();
        }

        private void load () {
            uint8[] raw;
            try {
                GLib.FileUtils.get_data (history_file, out raw);
            } catch {
                return;
            }

            // Try decryption first; fall back to plain text (migration path)
            uint8[]? pt = CryptoUtils.decrypt (raw);
            string content;
            if (pt != null) {
                content = (string) pt;
            } else {
                content = (string) raw;
            }

            foreach (string line in content.split ("\n")) {
                string[] parts = line.split ("\t");
                if (parts.length < 3) continue;
                var e = HistoryEntry ();
                e.timestamp = int64.parse (parts[0]);
                e.url       = parts[1].replace ("%09", "\t").replace ("%0A", "\n");
                e.title     = parts[2].replace ("%09", "\t").replace ("%0A", "\n");
                if (e.url.length > 0) entries.add (e);
            }
        }

        private void save () {
            dirty = false;
            var sb = new GLib.StringBuilder ();
            foreach (var e in entries) {
                string su = e.url.replace ("\t", "%09").replace ("\n", "%0A");
                string st = e.title.replace ("\t", "%09").replace ("\n", "%0A");
                sb.append (@"$(e.timestamp)\t$(su)\t$(st)\n");
            }
            uint8[] enc = CryptoUtils.encrypt ((uint8[]) sb.str.to_utf8 ());
            try { GLib.FileUtils.set_data (history_file, enc); } catch {}
        }
    }
}
