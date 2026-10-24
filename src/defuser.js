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

  // Site of this document; about:blank / srcdoc frames count as the page that created them.
  const hostOf = (o) => { try { return new URL(o).hostname; } catch (e) { return ''; } };
  const host = location.hostname || hostOf((location.ancestorOrigins || [])[0]);
  const listed = (list) => !!host && list.some((d) => host === d || host.endsWith('.' + d));
  // Signals to Crow's isolated JS world (tabs have no IPC): events with per-session names the
  // page cannot know, sent with functions captured before any page script runs.
  const PAD_EVENT = /*PAD_EVENT*/'';
  const dispatch = EventTarget.prototype.dispatchEvent, doc = document, Ev = Event;
  const signal = (name) => { try { dispatch.call(doc, new Ev(name)); } catch (e) {} };

  // WebGL off for this site (shield popover): no context is ever created, so neither rendering
  // nor GPU details (renderer, extensions) can be read. Each refusal is counted by Crow.
  if (listed(/*WEBGL_OFF*/[])) {
    const GL = /^(experimental-)?webgl2?$/i;
    for (const C of [window.HTMLCanvasElement, window.OffscreenCanvas]) {
      if (!C) continue;
      const getContext0 = C.prototype.getContext;
      C.prototype.getContext = mask(function getContext(type) {
        if (GL.test(String(type))) { signal(PAD_EVENT + 'g'); return null; }
        return getContext0.apply(this, arguments);
      }, getContext0);
    }
    for (const k of ['WebGLRenderingContext', 'WebGL2RenderingContext']) delete window[k];
  }

  // Gamepads: setting "gamepad_enabled", flipped for sites listed in the shield popover. WebKit's
  // own gamepads are never used (switched off on macOS; on Linux its events are dropped here).
  // Off: the API is removed and connect events never reach the page. On: Crow's native bridge
  // (src/gamepad.rs) feeds the top frame with the W3C standard layout, every pad under the one
  // generic id `PAD_ID`; frames get an empty list. Like the spec, pads stay hidden until a button
  // or stick is used on this page. A page's first read is signalled to Crow.
  const GAMEPAD = /*GAMEPAD*/false !== listed(/*GAMEPAD_FLIP*/[]);
  const PAD_EVENTS = ['gamepadconnected', 'gamepaddisconnected'];
  if (!GAMEPAD) {
    for (const k of ['getGamepads', 'webkitGetGamepads']) delete Navigator.prototype[k];
    for (const k of ['ongamepadconnected', 'ongamepaddisconnected']) { delete Window.prototype[k]; delete window[k]; }
    for (const k of ['Gamepad', 'GamepadEvent', 'GamepadButton', 'GamepadHapticActuator']) delete window[k];
    for (const type of PAD_EVENTS) window.addEventListener(type, (e) => e.stopImmediatePropagation(), true);
  } else {
    // Crow dispatches the state (JSON string) on `document` under a per-session event name.
    const parse = JSON.parse, freeze = Object.freeze, now = performance.now.bind(performance);
    const define = Object.defineProperty, isArray = Array.isArray;
    const ours = new WeakSet();
    let shown = new Map(), exposed = false, told = false;
    delete Navigator.prototype.webkitGetGamepads;
    for (const type of PAD_EVENTS) {
      window.addEventListener(type, (e) => { if (!ours.has(e)) e.stopImmediatePropagation(); }, true);
    }
    const native = Object.prototype.hasOwnProperty.call(Window.prototype, 'ongamepadconnected');
    const fire = (type, pad) => {
      const e = new Ev(type);
      define(e, 'gamepad', { value: pad, enumerable: true });
      ours.add(e);
      try { dispatch.call(window, e); } catch (x) {}
      const on = !native && window['on' + type];
      if (typeof on === 'function') { try { on.call(window, e); } catch (x) {} }
    };
    const pad = (p, t) => freeze({
      id: String(p.id),
      index: p.index | 0,
      connected: true,
      mapping: p.mapping === 'standard' ? 'standard' : '',
      timestamp: t,
      axes: freeze(p.axes.map(Number)),
      buttons: freeze(p.buttons.map((b) => freeze({ pressed: !!b.pressed, touched: !!b.touched, value: +b.value || 0 }))),
      vibrationActuator: null,
    });
    const used = (p) => p.buttons.some((b) => b.pressed) || p.axes.some((a) => a > 0.5 || a < -0.5);
    doc.addEventListener(PAD_EVENT + 'p', (e) => {
      e.stopImmediatePropagation();
      let list;
      try { list = parse(e.detail); } catch (x) { return; }
      if (!isArray(list)) return;
      const t = now();
      const pads = list.filter((p) => p && isArray(p.buttons) && isArray(p.axes)).map((p) => pad(p, t));
      if (!exposed && !pads.some(used)) return;
      exposed = true;
      const next = new Map(pads.map((p) => [p.index, p]));
      for (const [i, p] of shown) if (!next.has(i)) fire('gamepaddisconnected', freeze({ ...p, connected: false }));
      for (const [i, p] of next) if (!shown.has(i)) fire('gamepadconnected', p);
      shown = next;
    }, true);
    const getGamepads = function getGamepads() {
      if (!told) {
        told = true;
        signal(PAD_EVENT);
      }
      const out = [];
      for (const p of shown.values()) out[p.index] = p;
      for (let i = 0; i < out.length; i++) if (!out[i]) out[i] = null;
      return out;
    };
    const getGamepads0 = Navigator.prototype.getGamepads;
    define(Navigator.prototype, 'getGamepads', {
      value: getGamepads0 ? mask(getGamepads, getGamepads0) : getGamepads,
      writable: true, configurable: true, enumerable: true,
    });
  }

  // Sites whose app code (web components, Turbo frames, fetch-driven hydration, Gmail's "adn ads"
  // message containers, admin consoles, sign-in and payment frames) must see the native fetch,
  // Function.prototype.toString and layout getters, and that run no anti-adblock walls: nothing
  // is patched there. about:blank / srcdoc frames (editors, modals) count as the page that
  // created them.
  const TRUSTED = /(^|\.)(github\.com|google\.com|booking\.com|stripe\.com|stripe\.network)$|^accounts\./i;
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
