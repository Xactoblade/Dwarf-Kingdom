# Dwarf Kingdom

A deep colony-simulation game in the spirit of the great fortress sims:
a procedurally generated 3D world, autonomous dwarves with personalities,
and physics-driven stories of triumph and collapse.

Built in Rust + Bevy. See `BLUEPRINT.md` for the full design and phased roadmap.

## Status: Phase 4 — A World Outside

- [x] Phase 0: workspace, data-driven raws, world gen, z-level renderer, sim clock
- [x] Phase 1: agents, mine/stairs designations, A* + region-gated jobs,
      ramps, stockpiles/hauling, deterministic versioned saves
- [x] Phase 2: farming, still/kitchen workshops, lethal needs, thoughts &
      happiness, skills, migrants, fullscreen + mouse controls
- [x] Water: cellular automaton with depth 0–7, falling/spreading/pressure
      leveling, springs, active-set sleeping (settled lakes cost nothing)
- [x] Channels, floodgates, and levers — dig trenches, hold water back,
      flood on command; deep water blocks paths and drowns
- [x] Raiders: seasonal raiding parties scale with fort wealth; hostile AI
      chases your dwarves; your dwarves fight back
- [x] Anatomical combat v1: body parts, bleeding, narrated combat log,
      rest-healing for the wounded
- [x] Headless exit tests: raiders lured into a kill chamber, sealed in by
      floodgate, drowned by lever; a 3v1 brawl won, wounds healed by rest
- [x] Overworld: biomes from elevation/temperature/rainfall, rendered as an
      embark map with civilization sites
- [x] History: civilizations of four races simulate 80 years — sites founded,
      raids fought, named figures earning kills and swearing grudges
- [x] Embark screen: pick your region; each region is its own deterministic
      local map, and the nearest goblin civ becomes your enemy
- [x] Named sieges: raiding parties are led by historical figures whose
      grudge against you is readable in the Legends viewer (press `y`)
- [x] Headless exit tests: a siege leader's name and personal grudge are
      findable in Legends; different seeds breed different worlds and enemies

## Run

```sh
cargo run -p dk_app
```

| Key | Action |
|---|---|
| Mouse | Left-click: move cursor · wheel: zoom · right-drag: pan |
| Enter | (Embark screen) found the fortress at the cursor's region |
| `y` | Open/close the Legends viewer (world history) |
| Arrow keys | Move cursor (tile/dwarf info in HUD) |
| `[` / `]` | Z-level down / up |
| `d` / `x` / `h` | Designate mine / stairs / channel (press to anchor, again to apply) |
| `p` / `f` | Place stockpile / farm plot (same two-press flow) |
| `v` / `k` | Build still / kitchen at cursor |
| `g` / `l` / `t` | Build floodgate / lever (links nearest gate) / pull lever |
| `c` | Cancel designations (two-press rect) |
| Esc | Exit designation mode |
| Space | Pause · `.` single-step while paused |
| 1 / 2 / 3 | Sim speed |
| W A S E | Pan camera (`d` is taken by designate) |
| `-` / `=` | Zoom out / in |
| F5 / F9 | Save / load world |
| Q | Quit |
