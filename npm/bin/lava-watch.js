#!/usr/bin/env node

const wasm = require("../pkg/lava_wasm.js");

const ENTER_ALT = "\x1b[?1049h\x1b[?25l\x1b[2J\x1b[H";
const LEAVE_ALT = "\x1b[?25h\x1b[?1049l";
const MOUSE_ON = "\x1b[?1000h\x1b[?1006h";
const MOUSE_OFF = "\x1b[?1006l\x1b[?1000l";

// Crude argv split: any `--ascii` flag toggles ASCII mode at startup; the
// remaining positional (if any) is the palette name.
const argv = process.argv.slice(2);
const asciiFlag = argv.includes("--ascii");
const positional = argv.filter((a) => !a.startsWith("-"));
const arg = positional[0];

if (argv.includes("--help") || argv.includes("-h")) {
  const bytes = wasm.paletteHelp(
    "lava — pick a palette as a command argument:",
    "npx lava-watch uv"
  );
  process.stdout.write(Buffer.from(bytes));
  process.exit(0);
}

if (!process.stdout.isTTY) {
  process.stderr.write(
    "lava-watch: stdout is not a TTY (try running it directly in a terminal)\n"
  );
  process.exit(1);
}

const cols = process.stdout.columns || 80;
const rows = process.stdout.rows || 24;

let session;
try {
  session = new wasm.LavaSession(cols, rows, arg, wasm.randomSeed());
} catch (err) {
  process.stderr.write(`lava-watch: failed to start: ${err?.message || err}\n`);
  process.exit(1);
}

if (asciiFlag) session.toggleAscii();

let exiting = false;
function cleanup(code) {
  if (exiting) return;
  exiting = true;
  try {
    process.stdout.write(MOUSE_OFF);
  } catch (_) {}
  try {
    process.stdout.write(LEAVE_ALT);
  } catch (_) {}
  try {
    if (process.stdin.isTTY) process.stdin.setRawMode(false);
  } catch (_) {}
  process.stdin.pause();
  process.exit(code || 0);
}

process.on("SIGINT", () => cleanup());
process.on("SIGTERM", () => cleanup());
process.on("SIGHUP", () => cleanup());
process.on("uncaughtException", (err) => {
  cleanup(1);
  console.error(err);
});

process.stdout.write(ENTER_ALT);
process.stdout.write(MOUSE_ON);

if (process.stdin.isTTY) process.stdin.setRawMode(true);
process.stdin.resume();
process.stdin.on("data", (chunk) => {
  try {
    if (session.feedInput(chunk)) cleanup();
  } catch (_) {
    cleanup(1);
  }
});

process.stdout.on("resize", () => {
  try {
    session.resize(process.stdout.columns, process.stdout.rows);
  } catch (_) {}
});

const FRAME_MS = 33;
const MAX_DT = 0.25;
let last = process.hrtime.bigint();

setInterval(() => {
  if (exiting) return;
  const now = process.hrtime.bigint();
  const dt = Math.min(Number(now - last) / 1e9, MAX_DT);
  last = now;
  session.tick(dt);
  process.stdout.write(Buffer.from(session.render()));
}, FRAME_MS);
