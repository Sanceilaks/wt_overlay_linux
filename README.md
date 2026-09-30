# War Thunder Wayland HUD

A small external, click-through text HUD for War Thunder. It reads only the local
`127.0.0.1:8111` telemetry API and creates a transparent `wlr-layer-shell`
overlay on one explicitly selected Wayland output.

## Requirements

- Linux/Wayland compositor with `zwlr_layer_shell_v1`. See [Compositor Support](https://wayland.app/protocols/wlr-layer-shell-unstable-v1#compositor-support).
- A working Vulkan or OpenGL wgpu backend.
- Rust toolchain for building from source.
- `wayland-info` is useful for finding the exact output name.

## Build and run

```bash
cargo build --release
cp config.example.toml config.toml
cp hud.example.scm hud.scm
wayland-info                 # find names such as DP-1 or HDMI-A-1
$EDITOR config.toml
./target/release/wt-overlay-linux config.toml
```

Start order does not matter: the process remains alive while War Thunder is not
running and reconnects through its telemetry polling loop. Set `RUST_LOG=debug`
for additional diagnostics.

## Test without War Thunder

Start the standalone mock in one terminal:

```bash
cargo run --example telemetry_mock
```

It listens only on `127.0.0.1:8111` and serves dynamic `/state` and
`/indicators` responses. AoA periodically crosses the example script warning
threshold, so color and blinking can be checked too. In another terminal, run
the HUD with the normal configuration:

```bash
cp config.example.toml config.toml
cp hud.example.scm hud.scm
# Set the real Wayland output name in config.toml first.
cargo run -- config.toml
```

Stop the mock with Ctrl+C before starting War Thunder because both use port
8111.

`config.toml` contains operational settings and requires a restart when changed.
The script path is resolved relative to the config file. `hud.scm` is watched and
hot-reloaded; an invalid edit leaves the last valid program and scene active.

## HUD script

The Steel script must define `(build-hud telemetry)` and return `text` nodes.
It controls values, semantic slots, order, visibility, color, size, normal/bold
weight, shadow, and blink frequency. Pixel coordinates and native plugins are
intentionally unavailable. See `hud.example.scm` for all six MVP values (IAS,
TAS, AoA, G-load, altitude, and vertical speed) and the Cyrillic AoA warning.
The complete scripting API and custom-metric examples are documented in
[`SCRIPTING.md`](SCRIPTING.md).

The renderer embeds Noto Sans Regular/Bold, including Cyrillic glyphs. The font
license is in `assets/fonts/LICENSE-Noto.txt`.

### Custom metrics from history

`(history telemetry name value window-ms)` records one explicitly named numeric
value and returns its samples from the requested window (up to 30 seconds),
ordered oldest to newest. `value` may come from a normalized field, any raw
`state`/`indicators` field, or an arbitrary Steel calculation. Only series named
by the script are retained; complete telemetry snapshots are not stored. Every
sample has `age-ms` and `value` fields, with `sample-value` as a convenient
accessor. Generic helpers `series-delta`, `series-rate`, `series-span-ms`,
`series-average`, `series-min`, and `series-max` accept samples and an accessor
function. Rates are per second; use the span helper when a metric requires a
sufficiently complete window. History resets when telemetry is inactive or
discontinuous. A series retained under the same name survives script reloads;
newly declared series start empty and warm up normally. `hud.example.scm`
demonstrates a two-second IAS-loss warning implemented entirely in Steel.

## Screenshot
<img width="1420" height="1145" alt="image" src="https://github.com/user-attachments/assets/4594740f-8089-44ee-be57-ac49feef7d0e" />
