// crow:gamepad: runs in Crow's isolated JS world of that page (wk::eval_isolated), so it reads
// the real gamepads while websites only ever get `PAD_ID`. DOM updates use textContent and
// attributes only. The diagnostics table tells where input stops: macOS (`__crowSys`, pushed by
// Crow), WebKit (slots), or the page (secure context, focus, visibility).
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
  window.__crowSys = (list) => text('d-sys', !list ? '—' : list.length ? list.join(', ') : diag.dataset.none);
  const api = ENABLED && !!navigator.getGamepads;
  if (!api) {
    state.textContent = state.dataset.dis;
    document.body.classList.add('off');
  }
  // WebKit exposes a pad only after a button press while this page has focus (W3C gamepad
  // user activation); `gamepadconnected` marks that moment, polling then follows the state.
  let index = null, frames = 0;
  window.addEventListener('gamepadconnected', (e) => { index = e.gamepad.index; });
  window.addEventListener('gamepaddisconnected', (e) => { if (e.gamepad.index === index) index = null; });
  const fmt = (v) => (v >= 0 ? '+' : '') + v.toFixed(3);
  function frame() {
    frames++;
    const pads = api ? navigator.getGamepads() : [];
    text('d-js', yn(true) + ' (' + frames + ')');
    text('d-secure', yn(window.isSecureContext));
    text('d-focus', yn(document.hasFocus()));
    text('d-vis', yn(document.visibilityState === 'visible'));
    text('d-slots', api ? Array.from(pads).filter(Boolean).length + ' / ' + pads.length : '—');
    if (api) {
      const pad = (index !== null && pads[index]) || Array.from(pads).find(Boolean) || null;
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
      text('raw', pad ? pad.id : '—');
      text('spoof', pad ? PAD_ID : '—');
      text('map', pad ? pad.mapping || '—' : '—');
      text('counts', pad ? pad.buttons.length + ' / ' + pad.axes.length : '—');
      text('axes', pad ? Array.from(ax, (v, i) => 'axes[' + i + '] ' + fmt(v)).join('    ') : '');
    }
    requestAnimationFrame(frame);
  }
  requestAnimationFrame(frame);
})();
