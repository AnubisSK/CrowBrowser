// CrowClarity: video enhancement, run in Crow's isolated JS world of every frame (same DOM,
// globals and DOM expandos invisible to the page). Inert until the browser calls
// `__crowClarity(on, sharpen)` in the main frame. Everything runs on the local GPU: compositor
// CSS filters (WebKit: Core Animation / Metal, WebKitGTK: GL / DMABuf, WebView2: D3D), plus an
// optional Contrast Adaptive Sharpening fragment shader. No network, no models.
(() => {
  if (window.__crowClarity) return;
  // Subtle contrast lift + saturation normalization; compositor filters, no per-pixel JS.
  const FILTER = 'contrast(1.05) saturate(1.06)';
  // Shader target resolution cap (one RGBA texture + back buffer, 1080p ≈ 16 MB of GPU memory).
  const MAX_W = 1920, MAX_H = 1080;
  const VS = 'attribute vec2 p; varying vec2 uv; void main() { uv = vec2(p.x, -p.y) * .5 + .5; gl_Position = vec4(p, 0., 1.); }';
  // AMD FidelityFX CAS (simplified), then the same contrast / saturation as `FILTER`.
  const FS = `precision mediump float;
uniform sampler2D t; uniform vec2 px; varying vec2 uv;
vec3 s(float x, float y) { return texture2D(t, uv + vec2(x, y) * px).rgb; }
void main() {
  vec3 a = s(-1., -1.), b = s(0., -1.), c = s(1., -1.), d = s(-1., 0.), e = s(0., 0.), f = s(1., 0.),
       g = s(-1., 1.), h = s(0., 1.), i = s(1., 1.);
  vec3 mn = min(min(min(d, e), min(f, b)), h), mx = max(max(max(d, e), max(f, b)), h);
  mn += min(mn, min(min(a, c), min(g, i)));
  mx += max(mx, max(max(a, c), max(g, i)));
  vec3 amp = sqrt(clamp(min(mn, 2. - mx) / max(mx, vec3(1e-4)), 0., 1.));
  vec3 w = amp * (-1. / mix(8., 5., .5));
  vec3 col = clamp((b * w + d * w + f * w + h * w + e) / (1. + 4. * w), 0., 1.);
  col = (col - .5) * 1.05 + .5;
  col = mix(vec3(dot(col, vec3(.2126, .7152, .0722))), col, 1.06);
  gl_FragColor = vec4(clamp(col, 0., 1.), 1.);
}`;
  let on = false, sharpen = false, observer = null;
  const videos = new Set();

  // Inline style through CSSOM (allowed under any page CSP); the page's own values come back off.
  function style(v) {
    if (v.__crowClarity) return;
    v.__crowClarity = { filter: v.style.getPropertyValue('filter'), prio: v.style.getPropertyPriority('filter') };
    v.style.setProperty('filter', FILTER, 'important');
    videos.add(v);
    v.addEventListener('play', onPlay);
    v.addEventListener('pause', onStop);
    v.addEventListener('ended', onStop);
    v.addEventListener('emptied', onStop);
    if (sharpen && !v.paused) startGl(v);
  }

  function unstyle(v) {
    const old = v.__crowClarity;
    if (!old) return;
    stopGl(v);
    if (old.filter) v.style.setProperty('filter', old.filter, old.prio); else v.style.removeProperty('filter');
    v.removeEventListener('play', onPlay);
    v.removeEventListener('pause', onStop);
    v.removeEventListener('ended', onStop);
    v.removeEventListener('emptied', onStop);
    delete v.__crowClarity;
    videos.delete(v);
  }

  function onPlay(e) { if (on && sharpen) startGl(e.currentTarget); }
  function onStop(e) { stopGl(e.currentTarget); }

  function program(gl) {
    const sh = (type, src) => { const s = gl.createShader(type); gl.shaderSource(s, src); gl.compileShader(s); return s; };
    const p = gl.createProgram();
    gl.attachShader(p, sh(gl.VERTEX_SHADER, VS));
    gl.attachShader(p, sh(gl.FRAGMENT_SHADER, FS));
    gl.linkProgram(p);
    return gl.getProgramParameter(p, gl.LINK_STATUS) ? p : null;
  }

  // Shader pass on a canvas laid exactly over the video's picture (after it in the DOM, so the
  // player's own controls stay on top; clicks go through). Falls back to the CSS filter when
  // WebGL is missing, the frames are not readable (cross-origin without CORS, DRM) or the
  // video itself is fullscreen.
  function startGl(v) {
    if (v.__crowGl || !v.parentNode || document.fullscreenElement === v) return;
    const cv = document.createElement('canvas');
    const gl = cv.getContext('webgl', { alpha: false, antialias: false, depth: false, stencil: false,
      premultipliedAlpha: false, preserveDrawingBuffer: false, powerPreference: 'low-power' });
    const prog = gl && program(gl);
    if (!prog) return;
    cv.style.cssText = 'position:absolute;pointer-events:none;margin:0;padding:0;border:0;z-index:auto';
    gl.useProgram(prog);
    gl.bindBuffer(gl.ARRAY_BUFFER, gl.createBuffer());
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 1, -1, -1, 1, 1, 1]), gl.STATIC_DRAW);
    const loc = gl.getAttribLocation(prog, 'p');
    gl.enableVertexAttribArray(loc);
    gl.vertexAttribPointer(loc, 2, gl.FLOAT, false, 0, 0);
    gl.bindTexture(gl.TEXTURE_2D, gl.createTexture());
    for (const [k, val] of [[gl.TEXTURE_MIN_FILTER, gl.LINEAR], [gl.TEXTURE_MAG_FILTER, gl.LINEAR],
      [gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE], [gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE]]) gl.texParameteri(gl.TEXTURE_2D, k, val);
    const px = gl.getUniformLocation(prog, 'px');
    v.after(cv);
    const st = { cv, gl, frame: 0, rvfc: false };
    v.__crowGl = st;
    const draw = () => {
      if (v.__crowGl !== st) return;
      const vw = v.videoWidth, vh = v.videoHeight;
      if (!vw || !vh || cv.offsetParent !== v.offsetParent) return schedule();
      // Picture rectangle inside the element (`object-fit: contain`, the default for video).
      const ew = v.offsetWidth, eh = v.offsetHeight, k = Math.min(ew / vw, eh / vh);
      const w = vw * k, h = vh * k;
      const css = `${v.offsetLeft + (ew - w) / 2}px|${v.offsetTop + (eh - h) / 2}px|${w}px|${h}px`;
      if (css !== st.css) {
        st.css = css;
        const [l, t, cw, ch] = css.split('|');
        Object.assign(cv.style, { left: l, top: t, width: cw, height: ch });
      }
      const s = Math.min(1, MAX_W / vw, MAX_H / vh);
      const tw = Math.round(vw * s), th = Math.round(vh * s);
      if (cv.width !== tw || cv.height !== th) { cv.width = tw; cv.height = th; gl.viewport(0, 0, tw, th); }
      try {
        gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGB, gl.RGB, gl.UNSIGNED_BYTE, v);
      } catch (e) {
        return stopGl(v); // tainted (cross-origin) frames: CSS filter only
      }
      gl.uniform2f(px, 1 / vw, 1 / vh);
      gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
      schedule();
    };
    const schedule = () => {
      if (v.__crowGl !== st) return;
      if (v.requestVideoFrameCallback) { st.rvfc = true; st.frame = v.requestVideoFrameCallback(draw); }
      else st.frame = requestAnimationFrame(draw);
    };
    draw();
  }

  // Frees the GPU context and its texture at once (pause, end, disable, fullscreen).
  function stopGl(v) {
    const st = v.__crowGl;
    if (!st) return;
    delete v.__crowGl;
    if (st.rvfc) v.cancelVideoFrameCallback(st.frame); else cancelAnimationFrame(st.frame);
    const lose = st.gl.getExtension('WEBGL_lose_context');
    if (lose) lose.loseContext();
    st.cv.width = st.cv.height = 0;
    st.cv.remove();
  }

  function scan() {
    for (const v of document.querySelectorAll('video')) style(v);
    for (const v of [...videos]) if (!v.isConnected) unstyle(v);
  }

  document.addEventListener('fullscreenchange', () => {
    for (const v of videos) if (document.fullscreenElement === v) stopGl(v); else if (on && sharpen && !v.paused) startGl(v);
  }, true);
  window.addEventListener('pagehide', () => { for (const v of videos) stopGl(v); }, true);

  // `sharpen`: also run the shader pipeline (off for sites with WebGL off). Returns the state.
  window.__crowClarity = (enable, sharp) => {
    on = !!enable;
    sharpen = on && !!sharp;
    if (observer) { observer.disconnect(); observer = null; }
    if (!on) {
      for (const v of [...videos]) unstyle(v);
      return 'off';
    }
    for (const v of videos) { if (sharpen && !v.paused) startGl(v); else if (!sharpen) stopGl(v); }
    scan();
    let queued = false;
    observer = new MutationObserver(() => {
      if (queued) return;
      queued = true;
      requestAnimationFrame(() => { queued = false; scan(); });
    });
    observer.observe(document.documentElement, { childList: true, subtree: true });
    return 'on';
  };
})();
