// lava.js — canvas-based bootstrap. The Rust engine renders directly to an
// RGBA pixel buffer via wasm; the browser blits it onto a <canvas>. No
// terminal emulator, no ANSI parser — `ctx.putImageData` and we're done.
//
//   wasm.tick(dt) → wasm.renderRgba() → ctx.putImageData → frame on screen
//   user keys/clicks → wasm.cycleNext/Prev/clickPixel
//   palette switch  → fade in a badge in the bottom-left for ~3s

// `?v=` query-string hashes are substituted by the server at startup so a
// new binary build invalidates each asset's client cache (immutable + 1y).
import init, { LavaSession } from "/static/lava_wasm.js?v=__WASM_JS_HASH__";

// Internal pixel resolution. Higher = nicer metaballs, more wasm work per
// frame. The visible canvas is CSS-stretched to viewport, so this is purely
// a quality/performance knob.
const PIXEL_HEIGHT = 360;
const BADGE_MS = 3000;

(async () => {
  // Pass the wasm URL explicitly so the version query survives — the
  // wasm-pack-generated `init()` would otherwise resolve relative to its
  // own (already-versioned) URL and lose the `?v=` on the .wasm file.
  await init({ module_or_path: "/static/lava_wasm_bg.wasm?v=__WASM_HASH__" });

  const canvas = document.getElementById("lava");
  const badgeEl = document.getElementById("badge");
  const ctx = canvas.getContext("2d");
  // We're stretching to viewport with CSS — disable canvas smoothing so the
  // metaball gradients stay crisp at any scale.
  ctx.imageSmoothingEnabled = true;

  // Aspect-aware engine sizing. The engine treats `rows` as pixel-pairs
  // (half-block legacy), so the actual pixel grid is cols × 2*rows.
  function dimsFromViewport() {
    const ratio = window.innerWidth / window.innerHeight;
    const h = PIXEL_HEIGHT;
    const w = Math.max(20, Math.round(h * ratio));
    // engine "rows" = h/2 because it doubles internally.
    return { w, h, rows: Math.max(1, Math.floor(h / 2)) };
  }

  let { w, h, rows } = dimsFromViewport();
  canvas.width = w;
  canvas.height = h;

  const palette = window.location.pathname.replace(/^\/+|\/+$/g, "") || null;
  let session = new LavaSession(w, rows, palette);

  // Reusable clamped view onto the wasm-returned buffer; ImageData wants
  // Uint8ClampedArray.
  let imageData = ctx.createImageData(w, h);

  let badgeTimer = null;
  function flashBadge() {
    const name = session.currentPaletteName();
    const [ar, ag, ab] = session.currentPaletteAccent();
    const [br, bg, bb] = session.currentPaletteAccentBg();
    badgeEl.textContent = name;
    badgeEl.style.color = `rgb(${ar}, ${ag}, ${ab})`;
    badgeEl.style.background = `rgb(${br}, ${bg}, ${bb})`;
    badgeEl.classList.add("show");
    clearTimeout(badgeTimer);
    badgeTimer = setTimeout(() => badgeEl.classList.remove("show"), BADGE_MS);
  }

  // Keyboard: ←/→ cycle palettes, i flips invert.
  document.addEventListener("keydown", (e) => {
    if (e.key === "ArrowRight") {
      session.cycleNext();
      flashBadge();
    } else if (e.key === "ArrowLeft") {
      session.cyclePrev();
      flashBadge();
    } else if (e.key === "i" || e.key === "I") {
      session.toggleInverted();
    }
  });

  // Mouse: click anywhere on the canvas → heat that pixel.
  canvas.addEventListener("mousedown", (e) => {
    if (e.button !== 0) return;
    const rect = canvas.getBoundingClientRect();
    const x = ((e.clientX - rect.left) / rect.width) * w;
    const y = ((e.clientY - rect.top) / rect.height) * h;
    session.clickPixel(x, y);
  });

  // Resize: debounce, recompute dims, recreate ImageData + resize session.
  let resizeTimer = null;
  window.addEventListener("resize", () => {
    clearTimeout(resizeTimer);
    resizeTimer = setTimeout(() => {
      const next = dimsFromViewport();
      if (next.w === w && next.h === h) return;
      w = next.w;
      h = next.h;
      rows = next.rows;
      canvas.width = w;
      canvas.height = h;
      imageData = ctx.createImageData(w, h);
      session.resize(w, rows);
    }, 100);
  });

  // Frame loop. dt is wall-clock-derived so the simulation runs at the same
  // speed regardless of the browser's actual refresh rate.
  let last = performance.now();
  const frame = () => {
    const now = performance.now();
    const dt = (now - last) / 1000;
    last = now;
    session.tick(dt);
    const rgba = session.renderRgba();
    imageData.data.set(rgba);
    ctx.putImageData(imageData, 0, 0);
    requestAnimationFrame(frame);
  };
  requestAnimationFrame(frame);
})();
