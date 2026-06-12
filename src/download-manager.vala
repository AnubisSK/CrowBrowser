namespace CrowBrowser {

    public struct DownloadRecord {
        public string filename;
        public string dest_path;
        public string url;
        public int64  file_size;
        public int64  timestamp;
        public bool   failed;
    }

    public class DownloadManager : Object {

        private static DownloadManager? _instance = null;

        public static DownloadManager get_instance () {
            if (_instance == null) _instance = new DownloadManager ();
            return _instance;
        }

        private Gee.ArrayList<WebKit.Download> _active;
        private Gee.ArrayList<DownloadRecord?> _records;

        public signal void changed ();

        construct {
            _active  = new Gee.ArrayList<WebKit.Download> ();
            _records = new Gee.ArrayList<DownloadRecord?> ();
        }

        public void track (WebKit.Download dl) {
            _active.add (dl);
            changed ();

            dl.notify["estimated-progress"].connect (() => changed ());

            dl.finished.connect (() => {
                _active.remove (dl);
                var r = DownloadRecord ();
                r.dest_path = dl.get_destination () ?? "";
                r.filename  = GLib.Path.get_basename (r.dest_path);
                r.url       = dl.get_request ()?.get_uri () ?? "";
                r.failed    = false;
                r.timestamp = GLib.get_real_time () / 1000000;
                try {
                    var f = GLib.File.new_for_path (r.dest_path);
                    var info = f.query_info (GLib.FileAttribute.STANDARD_SIZE,
                                            GLib.FileQueryInfoFlags.NONE);
                    r.file_size = info.get_size ();
                } catch { r.file_size = 0; }
                _records.add (r);
                while (_records.size > 200) _records.remove_at (0);
                changed ();
            });

            dl.failed.connect ((err) => {
                _active.remove (dl);
                var r = DownloadRecord ();
                r.dest_path = dl.get_destination () ?? "";
                r.filename  = GLib.Path.get_basename (r.dest_path);
                r.url       = dl.get_request ()?.get_uri () ?? "";
                r.failed    = true;
                r.file_size = 0;
                r.timestamp = GLib.get_real_time () / 1000000;
                _records.add (r);
                changed ();
            });
        }

        public Gee.ArrayList<WebKit.Download> get_active () { return _active; }

        public Gee.ArrayList<DownloadRecord?> get_records_reversed () {
            var rev = new Gee.ArrayList<DownloadRecord?> ();
            for (int i = _records.size - 1; i >= 0; i--)
                rev.add (_records[i]);
            return rev;
        }

        public void clear_records () { _records.clear (); changed (); }
    }
}
