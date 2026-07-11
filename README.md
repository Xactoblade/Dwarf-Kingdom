# Dwarf Kingdom

A deep colony-simulation game in the spirit of the great fortress sims:
a procedurally generated 3D world, autonomous dwarves with personalities,
and physics-driven stories of triumph and collapse.

Built in Rust + Bevy. See `BLUEPRINT.md` for the full design and phased roadmap.

## Status: Phase 2 — Survive a Year

- [x] Phase 0: workspace, data-driven raws, world gen, z-level renderer, sim clock
- [x] Phase 1: agents, mine/stairs designations, A* + region-gated jobs,
      ramps, stockpiles/hauling, deterministic versioned saves
- [x] Farming: farm plots, seasonal crop growth from plant raws, plant/harvest
- [x] Workshops: still (brewing) and kitchen (cooking) with standing orders
- [x] Needs with stakes: dwarves eat/drink on their own; starvation is lethal
- [x] Thoughts & happiness: readable thought logs explain every mood swing
- [x] Skills that speed up work; seasonal migrant waves gated on food stocks
- [x] Fullscreen + mouse: wheel zoom, right-drag pan, click to move cursor
- [x] Headless exit tests: a 7-dwarf embark survives to year 2 with a working
      food industry; without food, dwarves demonstrably starve

## Run

```sh
cargo run -p dk_app
```

| Key | Action |
|---|---|
| Mouse | Left-click: move cursor · wheel: zoom · right-drag: pan |
| Arrow keys | Move cursor (tile/dwarf info in HUD) |
| `[` / `]` | Z-level down / up |
| `d` / `x` | Designate mine / stairs (press to anchor, again to apply) |
| `p` / `f` | Place stockpile / farm plot (same two-press flow) |
| `v` / `k` | Build still / kitchen at cursor |
| `c` | Cancel designations (two-press rect) |
| Esc | Exit designation mode |
| Space | Pause · `.` single-step while paused |
| 1 / 2 / 3 | Sim speed |
| W A S E | Pan camera (`d` is taken by designate) |
| `-` / `=` | Zoom out / in |
| F5 / F9 | Save / load world |
| Q | Quit |
