# War Thunder Wayland HUD

A small external, click-through text HUD for War Thunder. It reads only the local
`127.0.0.1:8111` telemetry API and creates a transparent `wlr-layer-shell`
overlay on one explicitly selected Wayland output.

## Requirements

- Linux/Wayland compositor with `zwlr_layer_shell_v1` (KWin, Sway, Hyprland, or
  another compatible wlroots compositor). GNOME/Mutter is not supported.
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

The renderer embeds Noto Sans Regular/Bold, including Cyrillic glyphs. The font
license is in `assets/fonts/LICENSE-Noto.txt`.

## Screenshot
<img width="1420" height="1145" alt="image" src="https://github.com/user-attachments/assets/4594740f-8089-44ee-be57-ac49feef7d0e" />

