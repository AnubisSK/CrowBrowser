// crow:gamepad: runs in Crow's isolated JS world of that page (wk::eval_isolated). Crow pushes
// the native bridge's state (`__crowPads`, gamepad.rs) and the controllers it reads with their
// real names (`__crowSys`), while websites only ever get `PAD_ID`. DOM updates use textContent
// and attributes only. The diagnostics table tells where input stops: the bridge (controllers),
// the page feed (slots), or the page (secure context, focus, visibility).
(() => {
  if (window.__crowDiag) return;
  window.__crowDiag = true;
  const ENABLED = /*ENABLED*/false;
  const PAD_ID = /*PAD_ID*/'';
  const $ = (id) => document.getElementById(id);
  const state = $('state'), diag = $('diag');
  if (!state || !diag) return;
  const text = (id, s) => { const el = $(id); if (el && el.textContent !== s) el.textContent = s; };
  const yn = (b) => (b ? diag.dataset.yes : diag.dataset.no);
  let names = [], pads = [];
  window.__crowSys = (list) => {
    names = Array.isArray(list) ? list : [];
    text('d-sys', !list ? '—' : list.length ? list.map((p) => p.name).join(', ') : diag.dataset.none);
  };
  window.__crowPads = (list) => { pads = Array.isArray(list) ? list : []; };
  const api = ENABLED;
  if (!api) {
    state.textContent = state.dataset.dis;
    document.body.classList.add('off');
  }
  // The pad last used stays selected while it is connected.
  let index = null, frames = 0;
  const fmt = (v) => (v >= 0 ? '+' : '') + v.toFixed(3);
  function frame() {
    frames++;
    text('d-js', yn(true) + ' (' + frames + ')');
    text('d-secure', yn(window.isSecureContext));
    text('d-focus', yn(document.hasFocus()));
    text('d-vis', yn(document.visibilityState === 'visible'));
    text('d-slots', api ? pads.length + ' / ' + names.length : '—');
    if (api) {
      const used = pads.find((p) => p.buttons.some((b) => b.pressed));
      if (used) index = used.index;
      const pad = pads.find((p) => p.index === index) || pads[0] || null;
      document.body.classList.toggle('live', !!pad);
      text('state', pad ? state.dataset.on : state.dataset.off);
      state.classList.toggle('on', !!pad);
      for (let i = 0; i <= 16; i++) {
        const el = $('b' + i);
        if (!el) continue;
        const b = pad && pad.buttons[i];
        el.classList.toggle('on', !!(b && b.pressed));
      }
      for (const [id, i] of [['lt', 6], ['rt', 7]]) {
        const b = pad && pad.buttons[i];
        $(id).setAttribute('width', String(60 * (b ? b.value : 0)));
      }
      const ax = pad ? pad.axes : [];
      for (const [id, x, y] of [['ls', 0, 1], ['rs', 2, 3]]) {
        $(id).setAttribute('transform', 'translate(' + (ax[x] || 0) * 28 + ' ' + (ax[y] || 0) * 28 + ')');
      }
      const real = pad && names.find((p) => p.index === pad.index);
      text('raw', real ? real.name : '—');
      text('spoof', pad ? PAD_ID : '—');
      text('map', pad ? pad.mapping || '—' : '—');
      text('counts', pad ? pad.buttons.length + ' / ' + pad.axes.length : '—');
      text('axes', pad ? Array.from(ax, (v, i) => 'axes[' + i + '] ' + fmt(v)).join('    ') : '');
    }
    requestAnimationFrame(frame);
  }
  requestAnimationFrame(frame);
})();
