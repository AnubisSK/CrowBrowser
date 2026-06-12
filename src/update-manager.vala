namespace CrowBrowser {

    public class UpdateManager : GLib.Object {

        public const string APP_VERSION = "1.0.0";

        private const string RELEASES_API =
            "https://api.github.com/repos/AnubisSK/CrowBrowser/releases/latest";

        private static UpdateManager? _instance = null;

        public static UpdateManager get_instance () {
            if (_instance == null) _instance = new UpdateManager ();
            return _instance;
        }

        public signal void update_available (string new_version);
        public signal void install_progress (string status);
        public signal void install_done ();
        public signal void install_failed (string reason);

        // ── Public API ────────────────────────────────────────────────

        public async void check_async () {
            string? latest = yield fetch_latest_version ();
            if (latest == null) return;
            if (compare_versions (latest, APP_VERSION) > 0)
                update_available (latest);
        }

        public async void install_async (string version) {
            string tmp      = GLib.Path.build_filename (GLib.Environment.get_tmp_dir (), "crow-update");
            string tarball  = GLib.Path.build_filename (tmp, "source.tar.gz");
            string src_dir  = GLib.Path.build_filename (tmp, "src");
            string build_dir = GLib.Path.build_filename (tmp, "build");

            yield run_silent (new string[] { "rm", "-rf", tmp });
            GLib.DirUtils.create_with_parents (src_dir, 0755);

            // Download source tarball
            install_progress ("Sťahujem v%s…".printf (version));
            string url = "https://github.com/AnubisSK/CrowBrowser/archive/refs/tags/v%s.tar.gz"
                .printf (version);
            if (!yield run_ok (new string[] {
                "curl", "-L", "--max-time", "120", "--fail", "-o", tarball, url
            })) {
                install_failed ("Sťahovanie zlyhalo — skontrolujte pripojenie");
                return;
            }

            // Extract
            install_progress ("Rozbaľujem archív…");
            if (!yield run_ok (new string[] {
                "tar", "xzf", tarball, "-C", src_dir, "--strip-components=1"
            })) {
                install_failed ("Rozbalenie zlyhalo");
                return;
            }

            // Meson configure
            install_progress ("Konfigurujem zostavenie…");
            GLib.DirUtils.create_with_parents (build_dir, 0755);
            if (!yield run_ok (new string[] { "meson", "setup", "--wipe", build_dir, src_dir })) {
                install_failed ("Konfigurácia zostavenia zlyhala (chýbajú závislosti?)");
                return;
            }

            // Build
            install_progress ("Kompilujem… (môže trvať niekoľko minút)");
            if (!yield run_ok (new string[] { "ninja", "-C", build_dir })) {
                install_failed ("Kompilácia zlyhala");
                return;
            }

            // Copy new binary over the running one
            install_progress ("Inštalujem…");
            string new_bin = GLib.Path.build_filename (build_dir, "crow-browser");
            if (!GLib.FileUtils.test (new_bin, GLib.FileTest.EXISTS)) {
                install_failed ("Skompilovaný binárny súbor nebol nájdený");
                return;
            }

            if (!yield copy_binary (new_bin)) {
                yield run_silent (new string[] { "rm", "-rf", tmp });
                return;
            }

            yield run_silent (new string[] { "rm", "-rf", tmp });
            install_done ();
        }

        // ── Internals ─────────────────────────────────────────────────

        private async bool copy_binary (string src_path) {
            string dest = resolve_install_dest ();
            var src  = GLib.File.new_for_path (src_path);
            var dst  = GLib.File.new_for_path (dest);
            try {
                src.copy (dst, GLib.FileCopyFlags.OVERWRITE, null, null);
                return true;
            } catch {
                // Primary location not writable — fall back to ~/.local/bin/
                string fallback = GLib.Path.build_filename (
                    GLib.Environment.get_home_dir (), ".local", "bin", "crow-browser"
                );
                GLib.DirUtils.create_with_parents (
                    GLib.Path.get_dirname (fallback), 0755
                );
                var fb = GLib.File.new_for_path (fallback);
                try {
                    src.copy (fb, GLib.FileCopyFlags.OVERWRITE, null, null);
                    return true;
                } catch (GLib.Error e) {
                    install_failed ("Inštalácia zlyhala: " + e.message);
                    return false;
                }
            }
        }

        private async string? fetch_latest_version () {
            try {
                var sub = new GLib.Subprocess.newv (
                    new string[] {
                        "curl", "-s", "--max-time", "10", "--fail",
                        "-H", @"User-Agent: CrowBrowser/$(APP_VERSION)",
                        RELEASES_API
                    },
                    GLib.SubprocessFlags.STDOUT_PIPE | GLib.SubprocessFlags.STDERR_SILENCE
                );
                string? json = null;
                string? err  = null;
                yield sub.communicate_utf8_async (null, null, out json, out err);
                if (json == null || json.length == 0) return null;
                return parse_tag_name (json);
            } catch {
                return null;
            }
        }

        private async bool run_ok (string[] args) {
            try {
                var sub = new GLib.Subprocess.newv (
                    args,
                    GLib.SubprocessFlags.STDOUT_SILENCE | GLib.SubprocessFlags.STDERR_SILENCE
                );
                yield sub.wait_async (null);
                return sub.get_successful ();
            } catch {
                return false;
            }
        }

        private async void run_silent (string[] args) {
            try {
                var sub = new GLib.Subprocess.newv (
                    args,
                    GLib.SubprocessFlags.STDOUT_SILENCE | GLib.SubprocessFlags.STDERR_SILENCE
                );
                yield sub.wait_async (null);
            } catch {}
        }

        private static string resolve_install_dest () {
            // Use the path of the currently running binary
            try {
                string self = GLib.FileUtils.read_link ("/proc/self/exe");
                if (self.length > 0) return self;
            } catch {}
            return GLib.Path.build_filename (
                GLib.Environment.get_home_dir (), ".local", "bin", "crow-browser"
            );
        }

        private static string? parse_tag_name (string json) {
            int idx = json.index_of ("\"tag_name\"");
            if (idx < 0) return null;
            int q1 = json.index_of ("\"", idx + 10);
            if (q1 < 0) return null;
            q1++;
            int q2 = json.index_of ("\"", q1);
            if (q2 <= q1) return null;
            string tag = json.substring (q1, q2 - q1);
            if (tag.has_prefix ("v")) tag = tag.substring (1);
            return tag.length > 0 ? tag : null;
        }

        private static int compare_versions (string a, string b) {
            string[] pa = a.split (".");
            string[] pb = b.split (".");
            int len = (pa.length > pb.length) ? (int) pa.length : (int) pb.length;
            for (int i = 0; i < len; i++) {
                int va = (i < pa.length) ? int.parse (pa[i]) : 0;
                int vb = (i < pb.length) ? int.parse (pb[i]) : 0;
                if (va != vb) return va - vb;
            }
            return 0;
        }
    }
}
