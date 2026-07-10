# Dwarf Kingdom

A colony-simulation game inspired by the mechanics of classic fortress sims,
built in Rust with Bevy. **`BLUEPRINT.md` is the design ground truth** — read the
relevant phase before implementing anything.

## Build & run

Rust is installed via rustup; ensure `~/.cargo/bin` is on PATH
(`source ~/.cargo/env`).

```sh
cargo run -p dk_app            # run the game (from the workspace root!)
cargo test --workspace         # unit tests
DK_SCREENSHOT=1 cargo run -p dk_app   # auto-capture phase0.png and exit (~3s)
```

Run from the workspace root so the app can find `data/`.

## Layout

- `crates/dk_core` — time/calendar, seeded RNG. Pure Rust, no engine deps.
- `crates/dk_raws` — data-file ("raws") schema and loader. Content lives in `data/`.
- `crates/dk_world` — tile map, mapgen, save/load. Pure Rust, no engine deps.
- `crates/dk_app` — Bevy binary: rendering, input, HUD.
- `data/` — RON content files (materials, later creatures/plants/items).

## Conventions

- Simulation crates stay engine-agnostic (no Bevy deps); `dk_app` wraps them in
  newtype `Resource`s.
- All randomness flows from seeded ChaCha streams (`dk_core::rng_from_seed`) —
  never `thread_rng()` in simulation code.
- Bevy is pinned to 0.16 — do not bump without a dedicated migration pass.
- New content categories get their own RON schema in `dk_raws` + files in `data/`.
