(function () {
  'use strict';
  // Sites whose app code (web components, Turbo frames, fetch-driven hydration) must see the
  // native fetch, Function.prototype.toString and layout getters, and that run no anti-adblock
  // walls: nothing is patched there.
  if (/(^|\.)github\.com$/i.test(location.hostname)) return;
  // Every replaced function reports the source of the native one it stands for, so
  // `Function.prototype.toString`, `__lookupGetter__(...).toString()` and `.name` checks see
  // "[native code]" (the toString override masks itself the same way).
  const natives = new WeakMap();
  const toString0 = Function.prototype.toString;
  const mask = (fake, real) => {
    natives.set(fake, real);
    Object.defineProperty(fake, 'name', { value: real.name, configurable: true });
    return fake;
  };
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
