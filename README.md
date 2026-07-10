# Dwarf Kingdom

A deep colony-simulation game in the spirit of the great fortress sims:
a procedurally generated 3D world, autonomous dwarves with personalities,
and physics-driven stories of triumph and collapse.

Built in Rust + Bevy. See `BLUEPRINT.md` for the full design and phased roadmap.

## Status: Phase 1 — Dig & Haul

- [x] Phase 0: workspace, data-driven raws, world gen, z-level renderer, sim clock
- [x] Dwarven agents with needs (hunger/thirst/fatigue), wandering, napping
- [x] Designations: mine and carve stairs, rectangle selection UI
- [x] Job system: nearest-job assignment gated by an O(1) region-connectivity
      check, A* pathfinding, automatic retry/abandon handling
- [x] Terrain with ramps — the whole surface is one walkable region
- [x] Mining drops stone boulders; haulers store them in stockpiles
- [x] Deterministic simulation (tested), versioned saves of the entire sim
- [x] Headless exit test: dig a staircase + room, haul every boulder, unattended

## Run

```sh
cargo run -p dk_app
```

| Key | Action |
|---|---|
| Arrow keys | Move cursor (tile info in HUD) |
| `[` / `]` | Z-level down / up |
| `d` / `x` | Designate mine / stairs (press to anchor, again to apply) |
| `p` / `c` | Place stockpile / cancel designations (same two-press flow) |
| Esc | Exit designation mode |
| Space | Pause · `.` single-step while paused |
| 1 / 2 / 3 | Sim speed |
| W A S E | Pan camera (`d` is taken by designate) |
| `-` / `=` | Zoom out / in |
| F5 / F9 | Save / load world |
| Q | Quit |
