namespace CrowBrowser {

    private class ContentScriptEntry : Object {
        public string[] matches  = {};
        public string[] js_files = {};
        public string[] css_files = {};
        public bool     at_start  = false;
    }

    public class ExtensionInfo : Object {
        public string id          = "";
        public string name        = "";
        public string version     = "";
        public string description = "";
        public string dir_path    = "";
        public bool   enabled     = true;
        internal Gee.ArrayList<ContentScriptEntry> content_scripts;
        construct { content_scripts = new Gee.ArrayList<ContentScriptEntry> (); }
    }

    public class ExtensionManager : Object {

        private static ExtensionManager? _instance = null;

        public static ExtensionManager get_instance () {
            if (_instance == null) _instance = new ExtensionManager ();
            return _instance;
        }

        private Gee.ArrayList<ExtensionInfo> _extensions;
        public string ext_dir { get; private set; }

        construct {
            ext_dir = GLib.Path.build_filename (
                GLib.Environment.get_user_config_dir (), "crow-browser", "extensions"
            );
            GLib.DirUtils.create_with_parents (ext_dir, 0755);
            _extensions = new Gee.ArrayList<ExtensionInfo> ();
            scan_extensions ();
        }

        public Gee.ArrayList<ExtensionInfo> get_extensions () {
            return _extensions;
        }

        // ── Scan & parse ──────────────────────────────────────────────────────

        private void scan_extensions () {
            try {
                var d = GLib.Dir.open (ext_dir, 0);
                string? entry;
                while ((entry = d.read_name ()) != null) {
                    string full = GLib.Path.build_filename (ext_dir, entry);
                    if (!GLib.FileUtils.test (full, GLib.FileTest.IS_DIR)) continue;
                    string mf = GLib.Path.build_filename (full, "manifest.json");
                    if (!GLib.FileUtils.test (mf, GLib.FileTest.EXISTS)) continue;
                    var info = parse_manifest (entry, full, mf);
                    if (info != null) _extensions.add (info);
                }
            } catch (GLib.FileError e) {
                warning ("Extensions: scan failed: %s", e.message);
            }
        }

        private ExtensionInfo? parse_manifest (string id, string dir, string mf_path) {
            var parser = new Json.Parser ();
            try {
                parser.load_from_file (mf_path);
            } catch (GLib.Error e) {
                warning ("Extensions: cannot parse %s: %s", mf_path, e.message);
                return null;
            }
            var root = parser.get_root ();
            if (root == null || root.get_node_type () != Json.NodeType.OBJECT) return null;
            var obj = root.get_object ();

            var info = new ExtensionInfo ();
            info.id          = id;
            info.dir_path    = dir;
            info.name        = str_member (obj, "name") ?? id;
            info.version     = str_member (obj, "version") ?? "";
            info.description = str_member (obj, "description") ?? "";
            info.enabled     = SettingsManager.get_instance ().get_extension_enabled (id);

            if (obj.has_member ("content_scripts") &&
                obj.get_member ("content_scripts").get_node_type () == Json.NodeType.ARRAY) {
                obj.get_array_member ("content_scripts").foreach_element ((_, _idx, node) => {
                    if (node.get_node_type () != Json.NodeType.OBJECT) return;
                    var cs_obj = node.get_object ();
                    var entry  = new ContentScriptEntry ();
                    entry.matches   = str_arr (cs_obj, "matches");
                    entry.js_files  = str_arr (cs_obj, "js");
                    entry.css_files = str_arr (cs_obj, "css");
                    string run_at   = str_member (cs_obj, "run_at") ?? "document_idle";
                    entry.at_start  = (run_at == "document_start");
                    info.content_scripts.add (entry);
                });
            }
            return info;
        }

        private static string? str_member (Json.Object obj, string key) {
            if (!obj.has_member (key)) return null;
            var n = obj.get_member (key);
            if (n.get_node_type () != Json.NodeType.VALUE) return null;
            if (n.get_value_type () != typeof (string)) return null;
            return n.get_string ();
        }

        private static string[] str_arr (Json.Object obj, string key) {
            if (!obj.has_member (key)) return {};
            var n = obj.get_member (key);
            if (n.get_node_type () != Json.NodeType.ARRAY) return {};
            var list = new Gee.ArrayList<string> ();
            n.get_array ().foreach_element ((_a, _i, en) => {
                if (en.get_node_type () == Json.NodeType.VALUE &&
                    en.get_value_type () == typeof (string))
                    list.add (en.get_string ());
            });
            return list.to_array ();
        }

        // ── Injection ─────────────────────────────────────────────────────────

        public void apply_to (WebKit.UserContentManager ucm) {
            foreach (var ext in _extensions) {
                if (!ext.enabled) continue;
                inject_extension (ucm, ext);
            }
        }

        private void inject_extension (WebKit.UserContentManager ucm, ExtensionInfo ext) {
            // Inject chrome API shim at document_start (runs before page scripts)
            string shim = build_shim (ext);
            ucm.add_script (new WebKit.UserScript (
                shim,
                WebKit.UserContentInjectedFrames.ALL_FRAMES,
                WebKit.UserScriptInjectionTime.START,
                null, null
            ));

            foreach (var cs in ext.content_scripts) {
                string guard = build_guard (cs.matches);
                var when = cs.at_start
                    ? WebKit.UserScriptInjectionTime.START
                    : WebKit.UserScriptInjectionTime.END;

                foreach (var js_file in cs.js_files) {
                    string js_path = GLib.Path.build_filename (ext.dir_path, js_file);
                    string src;
                    try { GLib.FileUtils.get_contents (js_path, out src); }
                    catch { warning ("Extensions: cannot read %s", js_path); continue; }

                    string wrapped = "(function(){\n" + guard + src + "\n})();";
                    ucm.add_script (new WebKit.UserScript (
                        wrapped,
                        WebKit.UserContentInjectedFrames.ALL_FRAMES,
                        when, null, null
                    ));
                }

                foreach (var css_file in cs.css_files) {
                    string css_path = GLib.Path.build_filename (ext.dir_path, css_file);
                    string css_src;
                    try { GLib.FileUtils.get_contents (css_path, out css_src); }
                    catch { warning ("Extensions: cannot read %s", css_path); continue; }

                    ucm.add_style_sheet (new WebKit.UserStyleSheet (
                        css_src,
                        WebKit.UserContentInjectedFrames.ALL_FRAMES,
                        WebKit.UserStyleLevel.USER,
                        null, null
                    ));
                }
            }
        }

        // ── Helpers ───────────────────────────────────────────────────────────

        private static string build_guard (string[] patterns) {
            if (patterns.length == 0) return "";
            foreach (var p in patterns)
                if (p == "<all_urls>" || p == "*://*/*") return "";

            var sb = new GLib.StringBuilder ();
            sb.append ("var __cp=");
            sb.append (to_js_str_array (patterns));
            sb.append (";\n");
            sb.append (URL_MATCH_FN);
            sb.append ("\nif(!__crow_ext_matches(window.location.href,__cp))return;\n");
            return sb.str;
        }

        private static string to_js_str_array (string[] arr) {
            var sb = new GLib.StringBuilder ("[");
            bool first = true;
            foreach (var s in arr) {
                if (!first) sb.append_c (',');
                first = false;
                sb.append_c ('"');
                sb.append (js_esc (s));
                sb.append_c ('"');
            }
            sb.append_c (']');
            return sb.str;
        }

        private static string js_esc (string s) {
            return s.replace ("\\", "\\\\")
                    .replace ("\"", "\\\"")
                    .replace ("'",  "\\'")
                    .replace ("\n", "\\n")
                    .replace ("\r", "\\r");
        }

        private static string build_shim (ExtensionInfo ext) {
            return CHROME_SHIM
                .replace ("{{EXT_ID}}",      js_esc (ext.id))
                .replace ("{{EXT_NAME}}",    js_esc (ext.name))
                .replace ("{{EXT_VERSION}}", js_esc (ext.version));
        }

        // ── URL match function (inlined into each content-script wrapper) ─────

        private const string URL_MATCH_FN = """
function __crow_ext_matches(url,ps){
  if(!ps||!ps.length)return true;
  for(var i=0;i<ps.length;i++){
    var p=ps[i];
    if(p==='<all_urls>')return true;
    try{
      var m=p.match(/^(\*|https?|file|ftp):\/\/([^/]*)(\/.*)$/);
      if(!m)continue;
      var s=m[1],h=m[2],pa=m[3]||'/';
      var u=new URL(url);
      if(s!=='*'&&u.protocol!==s+':')continue;
      if(s==='*'&&u.protocol!=='http:'&&u.protocol!=='https:')continue;
      if(h!=='*'){
        if(h.startsWith('*.')){var b=h.slice(2);if(u.hostname!==b&&!u.hostname.endsWith('.'+b))continue;}
        else if(u.hostname!==h)continue;
      }
      var pr='^'+pa.replace(/[.+^${}()|[\]\\]/g,'\\$&').replace(/\\\*/g,'.*')+'$';
      if(!(new RegExp(pr)).test(u.pathname+u.search))continue;
      return true;
    }catch(e){continue;}
  }
  return false;
}
""";

        // ── Chrome API shim ───────────────────────────────────────────────────
        // Placeholders: {{EXT_ID}}, {{EXT_NAME}}, {{EXT_VERSION}}

        private const string CHROME_SHIM = """
(function(){
  if(!window.__crowExts)window.__crowExts={};
  if(window.__crowExts['{{EXT_ID}}'])return;
  window.__crowExts['{{EXT_ID}}']=true;
  var __id='{{EXT_ID}}',__sp='crow_ext_{{EXT_ID}}_';
  function mkStore(pre){
    function _get(k){try{var v=localStorage.getItem(pre+k);if(v!==null)return JSON.parse(v);}catch(e){}return undefined;}
    function _set(k,v){try{localStorage.setItem(pre+k,JSON.stringify(v));}catch(e){}}
    return{
      get:function(k,cb){
        var r={};
        try{
          if(k==null){for(var i=0;i<localStorage.length;i++){var lk=localStorage.key(i);if(lk&&lk.startsWith(pre)){var rk=lk.slice(pre.length);r[rk]=_get(rk);}}}
          else if(typeof k==='string'){var v=_get(k);if(v!==undefined)r[k]=v;}
          else if(Array.isArray(k)){k.forEach(function(x){var v=_get(x);if(v!==undefined)r[x]=v;});}
          else if(typeof k==='object'){Object.keys(k).forEach(function(x){var v=_get(x);r[x]=(v!==undefined)?v:k[x];});}
        }catch(e){}
        if(cb)cb(r);return Promise.resolve(r);
      },
      set:function(items,cb){
        try{Object.keys(items).forEach(function(k){_set(k,items[k]);});}catch(e){}
        if(cb)cb();return Promise.resolve();
      },
      remove:function(k,cb){
        (Array.isArray(k)?k:[k]).forEach(function(x){try{localStorage.removeItem(pre+x);}catch(e){}});
        if(cb)cb();return Promise.resolve();
      },
      clear:function(cb){
        var rm=[];for(var i=0;i<localStorage.length;i++){var lk=localStorage.key(i);if(lk&&lk.startsWith(pre))rm.push(lk);}
        rm.forEach(function(k){localStorage.removeItem(k);});if(cb)cb();return Promise.resolve();
      },
      onChanged:{addListener:function(){},removeListener:function(){},hasListener:function(){return false;}}
    };
  }
  var __st={
    local:mkStore(__sp+'l_'),sync:mkStore(__sp+'s_'),session:mkStore(__sp+'e_'),
    onChanged:{addListener:function(){},removeListener:function(){},hasListener:function(){return false;}}
  };
  var __rt={
    id:__id,
    getManifest:function(){return{name:'{{EXT_NAME}}',version:'{{EXT_VERSION}}'};},
    getURL:function(p){return'chrome-extension://'+__id+'/'+p;},
    sendMessage:function(){return Promise.resolve();},
    onMessage:{addListener:function(){},removeListener:function(){},hasListener:function(){return false;}},
    onConnect:{addListener:function(){},removeListener:function(){},hasListener:function(){return false;}},
    onInstalled:{addListener:function(){}},onStartup:{addListener:function(){}},
    lastError:null
  };
  if(!window.chrome)window.chrome={};
  window.chrome.runtime=__rt;
  window.chrome.storage=__st;
  window.chrome.tabs={
    query:function(q,cb){if(cb)cb([]);return Promise.resolve([]);},
    sendMessage:function(){return Promise.resolve();},
    create:function(){},update:function(){},get:function(id,cb){if(cb)cb(null);return Promise.resolve(null);},
    onUpdated:{addListener:function(){}},onActivated:{addListener:function(){}},
    onCreated:{addListener:function(){}},onRemoved:{addListener:function(){}}
  };
  window.chrome.windows={
    getCurrent:function(q,cb){if(cb)cb(null);return Promise.resolve(null);},
    onFocusChanged:{addListener:function(){}}
  };
  window.chrome.extension={getURL:__rt.getURL,getBackgroundPage:function(){return null;}};
  window.chrome.notifications={
    create:function(id,o,cb){if(cb)cb(id||'');},clear:function(){},
    onClicked:{addListener:function(){}},onClosed:{addListener:function(){}}
  };
  window.chrome.i18n={getMessage:function(k){return k;},getUILanguage:function(){return navigator.language||'en';}};
  window.chrome.contextMenus={create:function(){},remove:function(){},update:function(){},onClicked:{addListener:function(){}}};
  window.chrome.action=window.chrome.browserAction={
    setIcon:function(){},setBadgeText:function(){},setBadgeBackgroundColor:function(){},
    setTitle:function(){},setPopup:function(){},enable:function(){},disable:function(){},
    onClicked:{addListener:function(){}}
  };
  window.chrome.pageAction={setIcon:function(){},show:function(){},hide:function(){},onClicked:{addListener:function(){}}};
  window.chrome.scripting={executeScript:function(){return Promise.resolve([]);},insertCSS:function(){return Promise.resolve();},removeCSS:function(){return Promise.resolve();}};
  window.chrome.declarativeNetRequest={getDynamicRules:function(cb){if(cb)cb([]);return Promise.resolve([]);},updateDynamicRules:function(){return Promise.resolve();}};
  window.browser=window.browser||window.chrome;
})();
""";
    }
}
