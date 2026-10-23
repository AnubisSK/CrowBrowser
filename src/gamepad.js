// crow:gamepad: runs in Crow's isolated JS world of that page (wk::eval_isolated), so it reads
// the real gamepads while websites only ever get `PAD_ID`. DOM updates use textContent and
// attributes only.
(() => {
  if (window.__crowDiag) return;
  window.__crowDiag = true;
  const ENABLED = /*ENABLED*/false;
  const PAD_ID = /*PAD_ID*/'';
  const $ = (id) => document.getElementById(id);
  const state = $('state');
  if (!state) return;
  const text = (id, s) => { const el = $(id); if (el && el.textContent !== s) el.textContent = s; };
  if (!ENABLED || !navigator.getGamepads) {
    state.textContent = state.dataset.dis;
    return;
  }
  const fmt = (v) => (v >= 0 ? '+' : '') + v.toFixed(3);
  function frame() {
    const pad = Array.from(navigator.getGamepads()).find(Boolean) || null;
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
    requestAnimationFrame(frame);
  }
  requestAnimationFrame(frame);
})();
