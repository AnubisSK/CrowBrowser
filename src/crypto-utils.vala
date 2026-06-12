// HMAC-SHA256 counter-mode stream cipher.
// Key = HMAC-SHA256(machine-id, app-salt) — machine-specific, no extra deps.
namespace CrowBrowser {

    namespace CryptoUtils {

        private static uint8[]? _key = null;

        // Derive a 32-byte key bound to this machine + user
        public static uint8[] derive_key () {
            if (_key != null) return _key;

            string machine_id = "";
            try { GLib.FileUtils.get_contents ("/etc/machine-id", out machine_id); } catch {}
            machine_id = machine_id.strip ();
            if (machine_id.length == 0)
                machine_id = GLib.Environment.get_home_dir ();

            // salt encodes app + user so separate users on the same machine get different keys
            string user  = GLib.Environment.get_user_name ();
            string salt  = "crow-browser:v1:" + user;

            var hmac = new GLib.Hmac (GLib.ChecksumType.SHA256, machine_id.data);
            hmac.update (salt.data);
            uint8[] key = new uint8[32];
            size_t  klen = 32;
            hmac.get_digest (key, ref klen);
            _key = key;
            return key;
        }

        // 8-byte nonce from /dev/urandom (fallback: GLib.Random)
        private static uint8[] random_nonce () {
            uint8[] n = new uint8[8];
            var f = GLib.FileStream.open ("/dev/urandom", "rb");
            if (f != null && f.read (n) == 8) return n;
            for (int i = 0; i < 8; i++) n[i] = (uint8)(GLib.Random.next_int () & 0xff);
            return n;
        }

        // One 32-byte block of keystream for position `block_idx`
        private static uint8[] keystream_block (uint8[] key, uint8[] nonce, int block_idx) {
            var hmac = new GLib.Hmac (GLib.ChecksumType.SHA256, key);
            hmac.update (nonce);
            uint8[] ctr = {
                (uint8)(block_idx >> 24), (uint8)(block_idx >> 16),
                (uint8)(block_idx >>  8), (uint8)(block_idx & 0xff)
            };
            hmac.update (ctr);
            uint8[] ks = new uint8[32];
            size_t  klen = 32;
            hmac.get_digest (ks, ref klen);
            return ks;
        }

        // Encrypt bytes → header "CROW" | 8-byte nonce | ciphertext
        public static uint8[] encrypt (uint8[] plaintext) {
            uint8[] key   = derive_key ();
            uint8[] nonce = random_nonce ();
            int     n     = plaintext.length;

            uint8[] ct = new uint8[n];
            for (int b = 0; b * 32 < n; b++) {
                uint8[] ks = keystream_block (key, nonce, b);
                int off = b * 32;
                int end = int.min (off + 32, n);
                for (int i = off; i < end; i++) ct[i] = plaintext[i] ^ ks[i - off];
            }

            uint8[] out_buf = new uint8[12 + n];
            out_buf[0] = 'C'; out_buf[1] = 'R'; out_buf[2] = 'O'; out_buf[3] = 'W';
            for (int i = 0; i < 8; i++) out_buf[4 + i] = nonce[i];
            for (int i = 0; i < n; i++) out_buf[12 + i] = ct[i];
            return out_buf;
        }

        // Decrypt → plaintext, or null if header is wrong
        public static uint8[]? decrypt (uint8[] data) {
            if (data.length < 12) return null;
            if (data[0] != 'C' || data[1] != 'R' || data[2] != 'O' || data[3] != 'W') return null;

            uint8[] key   = derive_key ();
            uint8[] nonce = new uint8[8];
            for (int i = 0; i < 8; i++) nonce[i] = data[4 + i];

            int     ct_len = data.length - 12;
            uint8[] pt     = new uint8[ct_len];
            for (int i = 0; i < ct_len; i++) pt[i] = data[12 + i];

            for (int b = 0; b * 32 < ct_len; b++) {
                uint8[] ks = keystream_block (key, nonce, b);
                int off = b * 32;
                int end = int.min (off + 32, ct_len);
                for (int i = off; i < end; i++) pt[i] ^= ks[i - off];
            }
            return pt;
        }
    }
}
