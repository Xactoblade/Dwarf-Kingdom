# Dwarf Kingdom

A deep colony-simulation game in the spirit of the great fortress sims:
a procedurally generated 3D world, autonomous dwarves with personalities,
and physics-driven stories of triumph and collapse.

Built in Rust + Bevy. See `BLUEPRINT.md` for the full design and phased roadmap.

## Status: Phase 0 — engine skeleton

- [x] Cargo workspace (`dk_core`, `dk_raws`, `dk_world`, `dk_app`)
- [x] Data-driven material raws (RON files in `data/`)
- [x] Seeded, deterministic world generation (strata, soil, ore veins)
- [x] Z-level tile renderer with depth fog, cursor, camera pan/zoom
- [x] Fixed-timestep sim clock (calendar: years/seasons/days)
- [x] Save/load of the world map

## Run

```sh
cargo run -p dk_app
```

| Key | Action |
|---|---|
| Arrow keys | Move cursor (tile info in HUD) |
| `[` / `]` | Z-level down / up |
| W A S D | Pan camera |
| `-` / `=` | Zoom out / in |
| F5 / F9 | Save / load world |
| Esc | Quit |
