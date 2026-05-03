// lava.js — canvas-based bootstrap. The Rust engine renders directly to an
// RGBA pixel buffer via wasm; the browser blits it onto a <canvas>. No
// terminal emulator, no ANSI parser — `ctx.putImageData` and we're done.
//
//   wasm.tick(dt) → wasm.renderRgba() → ctx.putImageData → frame on screen
//   user keys/clicks → wasm.cycleNext/Prev/clickPixel
//   palette switch  → fade in a badge in the bottom-left for ~3s
//
// `a` swaps in the ASCII renderer: same engine, same canvas, but each
// frame becomes wasm.render() (ANSI bytes) drawn as text via `fillText`
// after a tiny SGR parser walks the truecolor escapes.

// `?v=` query-string hashes are substituted by the server at startup so a
// new binary build invalidates each asset's client cache (immutable + 1y).
import init, { LavaSession } from "/static/lava_wasm.js?v=__WASM_JS_HASH__";

// RGBA path: internal pixel resolution. Higher = nicer metaballs, more wasm
// work per frame. The canvas is CSS-stretched to viewport so this is purely
// a quality/performance knob.
const PIXEL_HEIGHT = 360;

// ASCII path: cell size in canvas-internal pixels. The canvas internal
// dimensions are sized to a whole number of cells so chars stay crisp.
const ASCII_CHAR_W = 9;
const ASCII_CHAR_H = 18;
const ASCII_FONT = `${ASCII_CHAR_H}px ui-monospace, "SF Mono", Menlo, Consolas, monospace`;

const BADGE_MS = 3000;

(async () => {
  // Pass the wasm URL explicitly so the version query survives — the
  // wasm-pack-generated `init()` would otherwise resolve relative to its
  // own (already-versioned) URL and lose the `?v=` on the .wasm file.
  await init({ module_or_path: "/static/lava_wasm_bg.wasm?v=__WASM_HASH__" });

  const canvas = document.getElementById("lava");
  const badgeEl = document.getElementById("badge");
  const ctx = canvas.getContext("2d");
  ctx.imageSmoothingEnabled = true;

  // RGBA-mode dims: aspect-driven pixel grid; engine `rows` = pixel-height/2.
  function rgbaDims() {
    const ratio = window.innerWidth / window.innerHeight;
    const h = PIXEL_HEIGHT;
    const w = Math.max(20, Math.round(h * ratio));
    return { mode: "rgba", w, h, rows: Math.max(1, Math.floor(h / 2)) };
  }

  // ASCII-mode dims: pack as many `ASCII_CHAR_W × ASCII_CHAR_H` cells as fit
  // in the viewport. The canvas is sized to an exact multiple of cells.
  function asciiDims() {
    const cols = Math.max(20, Math.floor(window.innerWidth / ASCII_CHAR_W));
    const rows = Math.max(10, Math.floor(window.innerHeight / ASCII_CHAR_H));
    return {
      mode: "ascii",
      cols,
      rows,
      w: cols * ASCII_CHAR_W,
      // Engine `rows` is half the internal pixel height, so request `rows`
      // cells; the renderer emits `rows` lines.
      h: rows * ASCII_CHAR_H,
    };
  }

  function dimsForMode(mode) {
    return mode === "ascii" ? asciiDims() : rgbaDims();
  }

  let dims = rgbaDims();
  canvas.width = dims.w;
  canvas.height = dims.h;

  const palette = window.location.pathname.replace(/^\/+|\/+$/g, "") || null;
  let session = new LavaSession(dims.w, dims.rows, palette);
  // The web has its own UI chrome (DOM tips); suppress the engine's bottom
  // keybind strip so it doesn't appear in ASCII mode.
  session.toggleMenu();

  // RGBA-only: reusable ImageData onto the wasm-returned buffer.
  let imageData = ctx.createImageData(dims.w, dims.h);

  function applyDims(next) {
    dims = next;
    canvas.width = next.w;
    canvas.height = next.h;
    if (next.mode === "rgba") {
      imageData = ctx.createImageData(next.w, next.h);
      session.resize(next.w, next.rows);
    } else {
      session.resize(next.cols, next.rows);
    }
  }

  // Sync the engine's render mode → the JS render path. Called after any
  // input that might have flipped the mode (the 'a' keybind here, but the
  // engine could in theory flip it via feedInput too).
  function syncMode() {
    const wantAscii = session.isAsciiMode();
    if ((dims.mode === "ascii") !== wantAscii) {
      applyDims(dimsForMode(wantAscii ? "ascii" : "rgba"));
    }
  }

  let badgeTimer = null;
  function flashBadge() {
    // ASCII mode draws the badge on canvas via the engine — skip the DOM
    // copy so we don't double up.
    if (dims.mode === "ascii") return;
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

  // Keys: ←/→ cycle palettes, i invert, a toggle ASCII. (No 'h' on the web
  // — the bottom strip is permanently suppressed; the DOM tips serve as
  // chrome instead.)
  document.addEventListener("keydown", (e) => {
    if (e.key === "ArrowRight") {
      session.cycleNext();
      flashBadge();
    } else if (e.key === "ArrowLeft") {
      session.cyclePrev();
      flashBadge();
    } else if (e.key === "i" || e.key === "I") {
      session.toggleInverted();
    } else if (e.key === "a" || e.key === "A") {
      session.toggleAscii();
      syncMode();
    }
  });

  // Mouse: click anywhere on the canvas → heat that pixel. Coordinates are
  // converted to engine pixel space, which differs by mode (RGBA: w×h pixel
  // grid; ASCII: cols×rows*2 since the engine still doubles vertically).
  canvas.addEventListener("mousedown", (e) => {
    if (e.button !== 0) return;
    const rect = canvas.getBoundingClientRect();
    const fx = (e.clientX - rect.left) / rect.width;
    const fy = (e.clientY - rect.top) / rect.height;
    if (dims.mode === "rgba") {
      session.clickPixel(fx * dims.w, fy * dims.h);
    } else {
      session.clickPixel(fx * dims.cols, fy * dims.rows * 2);
    }
  });

  // Resize: debounce, recompute dims for the current mode, resize engine.
  let resizeTimer = null;
  window.addEventListener("resize", () => {
    clearTimeout(resizeTimer);
    resizeTimer = setTimeout(() => {
      const next = dimsForMode(dims.mode);
      if (next.w === dims.w && next.h === dims.h) return;
      applyDims(next);
    }, 100);
  });

  // Decode bytes once as UTF-8 so the parser works in chars (← / → · in the
  // bottom keybind strip are multibyte).
  const decoder = new TextDecoder();

  // ASCII path: walk the engine's truecolor SGR output and paint each char
  // onto the canvas. Cells with the same fg/bg coalesce into runs, mirroring
  // the renderer's own batching. Recognises SGR (`m`) and cursor positioning
  // (`H`) — the engine uses the latter for the centered badge and bottom
  // strip overlays.
  function drawAscii(bytes) {
    const text = decoder.decode(bytes);
    ctx.fillStyle = "#000";
    ctx.fillRect(0, 0, canvas.width, canvas.height);
    ctx.font = ASCII_FONT;
    ctx.textBaseline = "top";

    let curFg = "#fff";
    let curBg = null;
    let col = 0;
    let row = 0;
    let i = 0;
    const len = text.length;
    while (i < len) {
      const ch = text[i];
      if (ch === "\x1b" && text[i + 1] === "[") {
        let j = i + 2;
        while (j < len && text[j] !== "m" && text[j] !== "H") j++;
        const final = text[j];
        const params = text.slice(i + 2, j);
        if (final === "H") {
          // \x1b[<row>;<col>H — both 1-indexed, default to 1.
          if (params.length === 0) {
            row = 0;
            col = 0;
          } else {
            const [r, c] = params.split(";").map(Number);
            row = (r || 1) - 1;
            col = (c || 1) - 1;
          }
        } else {
          const parts = params.split(";");
          const head = +parts[0];
          if (head === 0) {
            curFg = "#fff";
            curBg = null;
          } else if (head === 38 && +parts[1] === 2) {
            curFg = `rgb(${+parts[2]},${+parts[3]},${+parts[4]})`;
          } else if (head === 48 && +parts[1] === 2) {
            curBg = `rgb(${+parts[2]},${+parts[3]},${+parts[4]})`;
          }
        }
        i = j + 1;
      } else if (ch === "\n") {
        col = 0;
        row++;
        i++;
      } else if (ch === "\r") {
        i++;
      } else {
        const x = col * ASCII_CHAR_W;
        const y = row * ASCII_CHAR_H;
        if (curBg) {
          ctx.fillStyle = curBg;
          ctx.fillRect(x, y, ASCII_CHAR_W, ASCII_CHAR_H);
        }
        ctx.fillStyle = curFg;
        ctx.fillText(ch, x, y);
        col++;
        i++;
      }
    }
  }

  // Frame loop. dt is wall-clock-derived so the simulation runs at the same
  // speed regardless of the browser's actual refresh rate.
  let last = performance.now();
  const frame = () => {
    const now = performance.now();
    const dt = (now - last) / 1000;
    last = now;
    session.tick(dt);
    if (dims.mode === "ascii") {
      drawAscii(session.render());
    } else {
      const rgba = session.renderRgba();
      imageData.data.set(rgba);
      ctx.putImageData(imageData, 0, 0);
    }
    requestAnimationFrame(frame);
  };
  requestAnimationFrame(frame);
})();
