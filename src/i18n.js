// Localization for the browser UI. Injected inline by `ui_view` before the page's own script
// (UI pages allow inline scripts only), with the dictionaries and the current language filled in.
(function () {
  'use strict';
  const L = /*LOCALES*/;
  let lang = /*LANG*/;
  let dict = L[lang] || L.en;

  // `t('key', a, b)`: translation in the current language, English fallback, then the key itself;
  // `{0}`, `{1}` … are replaced by the arguments.
  const t = (key, ...args) => {
    const s = String(dict[key] ?? L.en[key] ?? key ?? '');
    return s.replace(/\{(\d+)\}/g, (m, i) => (args[i] === undefined ? m : String(args[i])));
  };
  // HTML entity escaping for markup contexts. `t()` output is only ever assigned via
  // textContent / attributes; any future HTML template must use `t.html()` instead.
  const escapeHtml = (v) => String(v).replace(/[&<>"']/g, (c) =>
    ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]);
  // Like `t()`, but every argument is HTML-escaped before it is substituted.
  t.html = (key, ...args) => t(key, ...args.map(escapeHtml));
  // Plural forms: Slovak/Czech have one / few (2–4) / many, English/German one / other.
  // `shown` replaces `{0}` instead of `n` (e.g. a locale-formatted number).
  t.plural = (n, base, shown = n) => {
    const slavic = lang === 'sk' || lang === 'cs';
    const form = n === 1 ? 'one' : slavic && n >= 2 && n <= 4 ? 'few' : 'many';
    return t(base + '_' + form, shown);
  };

  // Static markup: data-i18n (text), data-i18n-title (+ data-hint suffix, e.g. a shortcut),
  // data-i18n-ph (placeholder). Text only via textContent / attributes, never HTML.
  const apply = (root) => {
    (root || document).querySelectorAll('[data-i18n]').forEach((el) => { el.textContent = t(el.dataset.i18n); });
    (root || document).querySelectorAll('[data-i18n-title]').forEach((el) => { el.title = t(el.dataset.i18nTitle) + (el.dataset.hint || ''); });
    (root || document).querySelectorAll('[data-i18n-ph]').forEach((el) => { el.placeholder = t(el.dataset.i18nPh); });
  };

  // Called from Rust when the language changes; pages re-render their dynamic text in `onLanguage`.
  const setLanguage = (code) => {
    if (!L[code]) return;
    lang = code;
    dict = L[code];
    document.documentElement.lang = code;
    apply();
    if (typeof window.onLanguage === 'function') window.onLanguage();
  };

  document.documentElement.lang = lang;
  window.t = t;
  window.applyI18n = apply;
  window.setLanguage = setLanguage;
  window.currentLanguage = () => lang;
  document.addEventListener('DOMContentLoaded', () => apply());
})();
