// Volume booster, run in Crow's isolated JS world of every frame (same DOM; globals, listeners
// and Web Audio nodes are invisible to the page, which can neither read nor undo them). Inert at
// 100 %: routing an element through Web Audio is permanent, so nothing is touched until the
// browser calls `__crowBoost(level)` with more. Per element:
// MediaElementSource -> DynamicsCompressor -> Gain -> destination.
(() => {
  if (window.__crowBoost) return;
  const RATIO = 20;
  const graphs = new WeakMap(); // element -> graph, or `null` while its context is starting
  const contexts = new Set();
  let level = 1, observer = null;
  window.__crowBoostBlocked = 0;

  // The compressor sits before the gain, so it limits relative to the boost: a 0 dBFS input
  // leaves the chain at about -1 dBFS. Web Audio adds makeup gain ((1 / full-range gain) ^ 0.6,
  // hard knee); the gain node takes it back out, so quiet audio gets exactly `level`.
  function tune(g) {
    const boost = level > 1, ratio = boost ? RATIO : 1;
    const threshold = boost ? -(20 * Math.log10(level) + 1) / (1 - 1 / ratio) : 0;
    const now = g.ctx.currentTime;
    g.comp.threshold.setValueAtTime(threshold, now);
    g.comp.knee.setValueAtTime(0, now);
    g.comp.ratio.setValueAtTime(ratio, now);
    g.comp.attack.setValueAtTime(0.003, now);
    g.comp.release.setValueAtTime(0.25, now);
    const makeup = Math.pow(10, -threshold * (1 - 1 / ratio) * 0.6 / 20);
    g.gain.gain.setTargetAtTime(level / makeup, now, 0.02); // no zipper noise while sliding
  }

  // Web Audio outputs silence for cross-origin media loaded without CORS; such elements keep
  // their normal playback. Same-origin and `blob:` (MSE players such as YouTube) are readable.
  function readable(m) {
    if (m.srcObject) return false;
    if (m.crossOrigin) return true; // loaded through a CORS check
    try {
      return new URL(m.currentSrc || m.src, location.href).origin === location.origin;
    } catch (e) {
      return false;
    }
  }

  async function hook(m) {
    if (level <= 1 || graphs.has(m) || m.__crowBoostSkip) return;
    if (!m.currentSrc && !m.src && !m.srcObject) return; // decided once it has a source
    if (!readable(m)) {
      m.__crowBoostSkip = true;
      window.__crowBoostBlocked++;
      return;
    }
    let ctx;
    try { ctx = new AudioContext(); } catch (e) { return; }
    graphs.set(m, null);
    try { await ctx.resume(); } catch (e) {}
    // Without a user activation the context stays suspended: hooking now would mute the
    // element, so it waits for its next `play`.
    if (ctx.state !== 'running' || level <= 1) {
      graphs.delete(m);
      ctx.close();
      return;
    }
    let src;
    try {
      src = ctx.createMediaElementSource(m);
    } catch (e) {
      // Already routed through the page's own Web Audio graph: left alone.
      graphs.delete(m);
      ctx.close();
      m.__crowBoostSkip = true;
      return;
    }
    const g = { ctx, comp: ctx.createDynamicsCompressor(), gain: ctx.createGain() };
    src.connect(g.comp).connect(g.gain).connect(ctx.destination);
    graphs.set(m, g);
    contexts.add(g);
    tune(g);
  }

  function scan() {
    for (const m of document.querySelectorAll('video, audio')) if (!m.paused) hook(m);
  }

  // `play` does not bubble; a capturing listener on the document sees every element's.
  document.addEventListener('play', (e) => {
    const m = e.target;
    if (!(m instanceof HTMLMediaElement)) return;
    const g = graphs.get(m);
    if (g && g.ctx.state === 'suspended') g.ctx.resume();
    else if (!g) hook(m);
  }, true);

  // Leaving the page frees every audio thread; a page kept for back/forward only pauses them.
  window.addEventListener('pagehide', (e) => {
    for (const g of contexts) {
      if (e.persisted) g.ctx.suspend(); else g.ctx.close();
    }
    if (!e.persisted) contexts.clear();
  }, true);
  window.addEventListener('pageshow', (e) => {
    if (e.persisted) for (const g of contexts) g.ctx.resume();
  }, true);

  // 1 = normal … 6 = 600 %. Returns how many elements are boosted.
  window.__crowBoost = (value) => {
    const v = Number(value);
    level = Number.isFinite(v) ? Math.min(6, Math.max(1, v)) : 1;
    for (const g of contexts) tune(g);
    if (observer) { observer.disconnect(); observer = null; }
    if (level > 1) {
      scan();
      let queued = false;
      observer = new MutationObserver(() => {
        if (queued) return;
        queued = true;
        requestAnimationFrame(() => { queued = false; scan(); });
      });
      observer.observe(document.documentElement, { childList: true, subtree: true });
    }
    return String(contexts.size);
  };
})();
