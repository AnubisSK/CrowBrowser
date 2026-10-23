(function () {
  'use strict';
  // Every replaced function reports the source of the native one it stands for, so
  // `Function.prototype.toString`, `__lookupGetter__(...).toString()` and `.name` checks see
  // "[native code]" (the toString override masks itself the same way; not installed on trusted
  // sites below).
  const natives = new WeakMap();
  const toString0 = Function.prototype.toString;
  const mask = (fake, real) => {
    natives.set(fake, real);
    Object.defineProperty(fake, 'name', { value: real.name, configurable: true });
    return fake;
  };

  // Gamepads (setting "gamepad_enabled"), on every site. Off: the API is removed (macOS also
  // switches it off in WebKit) and connect events never reach the page. On: every pad reports one
  // generic id, so model, vendor and product ids cannot be read; mapping, buttons and axes stay
  // real so games work. A page's first read is signalled to Crow's isolated JS world with a
  // per-session event name the page cannot know (tabs have no IPC).
  const GAMEPAD = /*GAMEPAD*/false;
  const PAD_EVENT = /*PAD_EVENT*/'';
  const PAD_ID = /*PAD_ID*/'';
  const PAD_EVENTS = ['gamepadconnected', 'gamepaddisconnected'];
  if (!GAMEPAD) {
    for (const k of ['getGamepads', 'webkitGetGamepads']) delete Navigator.prototype[k];
    for (const k of ['ongamepadconnected', 'ongamepaddisconnected']) { delete Window.prototype[k]; delete window[k]; }
    for (const k of ['Gamepad', 'GamepadEvent', 'GamepadButton', 'GamepadHapticActuator']) delete window[k];
    for (const type of PAD_EVENTS) window.addEventListener(type, (e) => e.stopImmediatePropagation(), true);
  } else if (Navigator.prototype.getGamepads) {
    const proxies = new WeakMap();
    const wrap = (g) => {
      if (!g) return g;
      let p = proxies.get(g);
      if (!p) {
        p = new Proxy(g, {
          get(target, key) {
            if (key === 'id') return PAD_ID;
            const v = Reflect.get(target, key);
            return typeof v === 'function' ? v.bind(target) : v;
          },
        });
        proxies.set(g, p);
      }
      return p;
    };
    const dispatch = EventTarget.prototype.dispatchEvent, doc = document, Ev = Event, map = Array.prototype.map;
    let told = false;
    const getGamepads0 = Navigator.prototype.getGamepads;
    Navigator.prototype.getGamepads = mask(function getGamepads() {
      const pads = getGamepads0.apply(this, arguments);
      if (!told) {
        told = true;
        try { dispatch.call(doc, new Ev(PAD_EVENT)); } catch (e) {}
      }
      return map.call(pads, wrap);
    }, getGamepads0);
    const d = window.GamepadEvent && Object.getOwnPropertyDescriptor(GamepadEvent.prototype, 'gamepad');
    if (d && d.get) {
      const get = mask({ get gamepad() { return wrap(d.get.call(this)); } }.__lookupGetter__('gamepad'), d.get);
      Object.defineProperty(GamepadEvent.prototype, 'gamepad', { configurable: d.configurable, enumerable: d.enumerable, get });
    }
  }

  // Sites whose app code (web components, Turbo frames, fetch-driven hydration, Gmail's "adn ads"
  // message containers, admin consoles, sign-in and payment frames) must see the native fetch,
  // Function.prototype.toString and layout getters, and that run no anti-adblock walls: nothing
  // is patched there. about:blank / srcdoc frames (editors, modals) count as the page that
  // created them.
  const TRUSTED = /(^|\.)(github\.com|google\.com|booking\.com|stripe\.com|stripe\.network)$|^accounts\./i;
  const hostOf = (o) => { try { return new URL(o).hostname; } catch (e) { return ''; } };
  const host = location.hostname || hostOf((location.ancestorOrigins || [])[0]);
  if (TRUSTED.test(host)) return;
  const toString = mask({ toString() { return toString0.call(natives.get(this) || this); } }.toString, toString0);
  Object.defineProperty(Function.prototype, 'toString', { value: toString, writable: true, configurable: true });

  // Bait globals normally set by an ads.js file that the blocker stops. Defined before any page
  // markup exists, read-only and non-configurable: `<img id="canRunAds">` (DOM clobbering) or a
  // page script cannot replace them.
  const bait = { canRunAds: true, isAdBlockActive: false, adblockEnabled: false, _isAdBlockBlocked: false };
  for (const k in bait) {
    if (!Object.prototype.hasOwnProperty.call(window, k)) {
      Object.defineProperty(window, k, { value: bait[k], writable: false, configurable: false, enumerable: true });
    }
  }

  const AD_HOST = /(^|\.)(doubleclick\.net|googlesyndication\.com|googleadservices\.com|googletagservices\.com|googletagmanager\.com|google-analytics\.com|adservice\.google\.[a-z.]+|imasdk\.googleapis\.com|amazon-adsystem\.com|adnxs\.com|taboola\.com|outbrain\.com|criteo\.(com|net)|pubmatic\.com|rubiconproject\.com|moatads\.com|scorecardresearch\.com|popads\.net|propellerads\.com|exoclick\.com|adsterra\.com)$/i;
  const AD_PATH = /\/(ads?|adsbygoogle|advert\w*|pagead\d*|prebid\w*|show_ads\w*)([\/._-]|$)/i;
  const isAd = (u) => {
    try { const x = new URL(u, location.href); return AD_HOST.test(x.hostname) || AD_PATH.test(x.pathname); } catch (e) { return false; }
  };

  // Blocked ad scripts/images/frames: report "load" instead of "error" (like an empty 200 response).
  window.addEventListener('error', (e) => {
    const t = e.target;
    const src = t && t !== window && (t.src || t.href);
    if (!src || !isAd(src)) return;
    e.stopImmediatePropagation();
    if (t.tagName === 'SCRIPT' && /adsbygoogle/i.test(src)) {
      const a = window.adsbygoogle = window.adsbygoogle || [];
      a.loaded = true;
      a.push = () => 0;
    }
    t.dispatchEvent(new Event('load'));
  }, true);

  // Blocked ad fetches resolve with an empty 200 response instead of rejecting.
  const fetch0 = window.fetch;
  if (fetch0) {
    window.fetch = mask(function (input) {
      const u = typeof input === 'string' ? input : input && input.url;
      return fetch0.apply(this, arguments).catch((err) => {
        if (u && isAd(String(u))) return new Response('', { status: 200 });
        throw err;
      });
    }, fetch0);
  }

  // Bait elements (ad-like class/id) hidden by cosmetic filters report a non-zero size.
  const BAIT = /(^|[\s_-])(ad|ads|adsbox|adsbygoogle|ad-?banner|ad-?placement|ad-?container|advert\w*|banner-?ads?|textads?|pub_\d+x\d+)([\s_-]|$)/i;
  const isBait = (el) => (typeof el.className === 'string' && BAIT.test(el.className)) || BAIT.test(el.id || '');
  const patch = (proto, prop, fake) => {
    const d = Object.getOwnPropertyDescriptor(proto, prop);
    if (!d || !d.get) return;
    const get = mask({ get() { const v = d.get.call(this); return (v === 0 || v === null) && isBait(this) ? fake(this) : v; } }.get, d.get);
    Object.defineProperty(proto, prop, { configurable: d.configurable, enumerable: d.enumerable, get, set: d.set });
  };
  for (const p of ['offsetHeight', 'offsetWidth']) patch(HTMLElement.prototype, p, () => 100);
  for (const p of ['clientHeight', 'clientWidth']) patch(Element.prototype, p, () => 100);
  patch(HTMLElement.prototype, 'offsetParent', () => document.body);
})();
