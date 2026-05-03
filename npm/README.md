# lava-watch

A lava lamp in your terminal — the engine behind [ssh lava.watch][repo],
compiled to WebAssembly and run locally.

```sh
npx lava-watch          # default palette
npx lava-watch uv       # ultraviolet
npx lava-watch --ascii  # start in ASCII mode
npx lava-watch --help   # help text
```

In session: `←` / `→` cycle palettes, `i` inverts colors, `a` toggles the
ASCII renderer, `q` (or `ctrl-c`) quits, left-click anywhere on the lamp
to heat that spot.

No network, no telemetry, no terminal emulator — just `tick()` and `render()`
shipped as a wasm bundle. Requires Node ≥ 18.

[repo]: https://github.com/robherley/lava.watch
