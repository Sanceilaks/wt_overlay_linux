# Repository Guidelines

## Project Structure & Module Organization

This repository contains a single Rust 2024 binary crate. `src/main.rs` is the entry point, while `src/app.rs` coordinates the application. Domain code is grouped by responsibility: `telemetry/` polls and normalizes War Thunder data, `scripting/` evaluates Steel HUD scripts, `layout/` positions scene nodes, and `overlay/` plus `render/` manage the Wayland surface and drawing. Shared scene and redraw state live in top-level modules under `src/`.

The test telemetry server is `examples/telemetry_mock.rs`. JSON fixtures belong in `tests/fixtures/telemetry/`. Embedded fonts and their license are under `assets/fonts/`. Treat `config.example.toml` and `hud.example.scm` as versioned templates; local `config.toml` and `hud.scm` are intentionally ignored.

## Build, Test, and Development Commands

- `cargo run -- config.toml` builds and runs the HUD in debug mode.
- `cargo run --example telemetry_mock` serves test telemetry on `127.0.0.1:8111`; stop it before launching War Thunder.
- `cargo build --release` creates `target/release/wt-overlay-linux`.
- `cargo test --locked --all-targets` runs unit tests across the crate and example.
- `cargo fmt --all -- --check` verifies formatting.
- `cargo clippy --locked --all-targets -- -D warnings` applies the same lint policy as CI.
- `cargo check --locked --all-targets` is the fastest full compile check.

Linux development requires Wayland headers and `pkg-config`; CI installs `libwayland-dev` and `pkg-config`.

## Coding Style & Naming Conventions

Use standard `rustfmt` output (four-space indentation) and keep Clippy warning-free. Name modules, functions, and variables in `snake_case`; types and traits in `UpperCamelCase`; constants in `SCREAMING_SNAKE_CASE`. Keep platform/rendering concerns separated from telemetry and script evaluation. Return contextual errors with `anyhow` at application boundaries and typed errors where callers need to branch.

## Testing Guidelines

Place focused unit tests beside their module in `mod tests`; larger Steel tests live in `src/scripting/steel/tests.rs`. Name tests after observable behavior, for example `missing_fields_use_defaults`. Add or reuse JSON fixtures for telemetry parsing cases. There is no formal coverage threshold, but new parsing, state-transition, and layout behavior should include regression tests.

## Commit & Pull Request Guidelines

History does not enforce a strict commit convention. Use a short imperative subject describing one logical change, such as `Hide HUD while in hangar`. Before opening a PR, run formatting, Clippy, and tests. PR descriptions should explain behavior changes and verification steps, link relevant issues, and include a screenshot or recording for visible HUD/layout changes.
