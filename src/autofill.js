// Login helper: runs in Crow's isolated JS world (same DOM, own globals and prototypes), top
// document of HTTPS pages only. It holds no secret. The badge only asks Crow to open its own popover
// (polled through `PAGE_READ`, tabs have no IPC); values arrive after the user picks a login there.
// navigator.credentials is left alone: passkey calls go straight to the OS authenticator.
(() => {
  if (window.__crowFillIn || window !== window.top) return;
  if (location.protocol !== 'https:' || location.hostname === 'crow.invalid') return;

  const TYPES = new Set(['text', 'email', 'tel', 'number', 'password']);
  const TOKENS = /(^|\s)(username|email|current-password|new-password|one-time-code|webauthn)(\s|$)/;
  const NAMES = /user|login|e-?mail|pass|pwd|otp|one.?time/i;
  const OTP = /otp|one.?time/i;
  const token = (el) => (el.getAttribute('autocomplete') || '').toLowerCase();
  const isField = (el) => el instanceof HTMLInputElement && !el.disabled && !el.readOnly && TYPES.has(el.type)
    && (el.type === 'password' || TOKENS.test(token(el)) || NAMES.test(el.name + ' ' + el.id));
  const isOtp = (el) => token(el).includes('one-time-code') || OTP.test(el.name + ' ' + el.id);
  const canPass = (el) => el.type === 'password' && !isOtp(el) && !token(el).includes('new-password');
  const canUser = (el) => el.type !== 'password' && !isOtp(el);
  // Fields hidden from the user (autofill harvesting) never get a value.
  const shown = (el) => {
    const r = el.getBoundingClientRect(), c = getComputedStyle(el);
    return r.width > 1 && r.height > 1 && c.visibility === 'visible' && c.opacity !== '0';
  };

  // A plain element: a page could define any custom tag first. Styled through the CSSOM, which
  // page CSP does not restrict; nothing inside the closed root is reachable from the page.
  const host = document.createElement('div');
  host.style.cssText = 'all:initial!important;position:fixed!important;z-index:2147483647!important;width:22px!important;height:22px!important';
  const button = document.createElement('button');
  button.type = 'button';
  button.title = /*LABEL*/'';
  button.setAttribute('aria-label', button.title);
  button.style.cssText = 'all:initial;display:flex;align-items:center;justify-content:center;width:22px;height:22px;border-radius:6px;background:#5b8def;cursor:pointer';
  const SVG = 'http://www.w3.org/2000/svg';
  const icon = document.createElementNS(SVG, 'svg');
  for (const [k, v] of [['viewBox', '0 0 24 24'], ['width', '14'], ['height', '14'], ['fill', 'none'], ['stroke', '#fff'], ['stroke-width', '2.2'], ['stroke-linecap', 'round']]) icon.setAttribute(k, v);
  for (const [tag, attrs] of [['circle', [['cx', '8'], ['cy', '15'], ['r', '4']]], ['path', [['d', 'M11 12l9-9M17 6l3 3M15 8l2 2']]]]) {
    const part = document.createElementNS(SVG, tag);
    for (const [k, v] of attrs) part.setAttribute(k, v);
    icon.append(part);
  }
  button.append(icon);
  host.attachShadow({ mode: 'closed' }).append(button);

  // Keeps the field focused; a page cannot synthesize this click (`isTrusted`, closed root).
  button.addEventListener('mousedown', (e) => e.preventDefault());
  button.addEventListener('click', (e) => {
    e.stopPropagation();
    if (e.isTrusted) window.__crowFillAsk = 1;
  });

  let active = null, last = null;
  const hide = () => { active = null; host.remove(); };
  const place = () => {
    if (!active) return;
    if (!active.isConnected) return hide();
    const r = active.getBoundingClientRect();
    host.style.setProperty('left', Math.round(r.right - 26) + 'px', 'important');
    host.style.setProperty('top', Math.round(r.top + (r.height - 22) / 2) + 'px', 'important');
  };
  window.addEventListener('focusin', (e) => {
    const el = e.composedPath()[0];
    if (!isField(el)) return hide();
    active = last = el;
    place();
    if (!host.isConnected) document.documentElement.append(host);
  }, true);
  window.addEventListener('focusout', hide, true);
  window.addEventListener('scroll', place, true);
  window.addEventListener('resize', place, true);

  const set = (el, value) => {
    el.focus();
    el.value = value;
    for (const type of ['input', 'change']) el.dispatchEvent(new Event(type, { bubbles: true }));
  };

  // Called by Crow after the user picked a login in its popover. `site` guards against a
  // navigation since then: another document never gets the values.
  window.__crowFillIn = (site, user, pass) => {
    if (location.hostname !== site || location.protocol !== 'https:') return '';
    const from = last && last.isConnected ? last : null;
    const scope = from ? from.form || from.getRootNode() : document;
    const all = Array.from(scope.querySelectorAll('input')).filter((el) => isField(el) && shown(el));
    const pw = from && all.includes(from) && canPass(from) ? from : all.find(canPass);
    const name = from && all.includes(from) && canUser(from) ? from
      : pw ? all.slice(0, Math.max(0, all.indexOf(pw))).filter(canUser).pop() : all.find(canUser);
    if (name) set(name, user);
    if (pw) set(pw, pass);
    return name || pw ? 'ok' : '';
  };
})();
