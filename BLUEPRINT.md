# Dwarf Kingdom — Blueprint for a Dwarf Fortress–Class Colony Simulator

> A ground-up design document for building a game with the full mechanical depth of
> Dwarf Fortress (Bay 12 Games), under our own name, art, and writing.
>
> **Legal ground rules:** game *mechanics* are not copyrightable, so cloning systems is
> fine. What we must NOT copy: the name "Dwarf Fortress", Bay 12/Kitfox art and
> tilesets, their written text/descriptions, or their raw data files verbatim. Everything
> here is our own implementation ("Dwarf Kingdom").

---

## 1. What the game actually is

Dwarf Fortress is three games sharing one simulation:

1. **Fortress Mode** — indirect-control colony sim. You designate work (dig, build,
   craft); autonomous dwarves with personalities decide how to do it. No win state —
   "losing is fun."
2. **Adventure Mode** — a turn-based roguelike where you play one character inside the
   same persistent world, including your own dead fortresses.
3. **Legends Mode** — a browsable historical record of everything the world simulation
   has ever generated.

The magic is that **one simulation feeds all three**. World history, creatures, items,
and sites persist across modes and playthroughs. That single decision drives most of the
architecture below.

### The core loop (Fortress Mode)
Embark with 7 dwarves + supplies → dig into a mountain → farm/brew/craft → attract
migrants → wealth attracts threats (thieves → ambushes → sieges → megabeasts) →
manage happiness, nobles, justice, military → thrive until an inevitable, memorable
collapse.

---

## 2. Complete feature inventory (what "feature-complete" means)

Compiled from the official feature page, the wiki, and design interviews.

### 2.1 World generation
- Fractal/midpoint-displacement heightmap; simulated **elevation, temperature,
  rainfall, drainage, savagery, mineral distribution** as separate world layers
- Biomes derived from those layers (each with its own flora/fauna tables)
- Rivers traced from mountains to oceans; lakes, volcanoes, aquifers
- ~200+ stone/mineral types placed in geologically plausible strata
  (igneous/sedimentary/metamorphic layers, veins, clusters)
- Three underground cavern layers, an underworld, magma sea
- Good/evil region alignment affecting creatures, weather, and plants
- **History simulation**: civilizations (dwarves, humans, elves, goblins, kobolds)
  found sites, wage wars, trade, produce named historical figures, artifacts, books,
  and monsters with kill-lists — run for a configurable number of in-world years
- Everything recorded as browsable historical events (feeds Legends Mode)

### 2.2 The dwarves (agent simulation)
- Individual **personality facets, values, beliefs, preferences** (favorite foods,
  materials, animals…) per creature
- **Needs**: food, drink (alcohol dependency), sleep, prayer, socializing, crafting…
- **Emotions/stress**: events generate thoughts → emotions → long-term stress;
  breakdown states: tantrum, depression, insanity; cascading "tantrum spirals"
- **Skills** (~100+): every action trains its skill; skill affects speed and quality
- **Relationships**: family, friendship, grudges, marriage, children
- **Strange moods**: possessed dwarves claim a workshop and forge a legendary artifact
- **Health**: body-part–level wounds, infection, medical care (diagnosis, surgery,
  sutures, splints, crutches, immobilization), scars, prosthetics
- Ghosts of unburied dead; memorials and tombs

### 2.3 Fortress management
- **Designations**: dig, channel, ramp, stairs (full 3D z-level mining), chop, gather,
  smooth/engrave stone
- **Zones**: bedrooms, dining halls, pastures, ponds, garbage, hospitals, temples,
  taverns, libraries, guildhalls
- **Stockpiles** with fine-grained filters, hauling routes, bins/barrels/wheelbarrows
- **Workshops & industry chains** — 30+ industries: farming (surface + underground),
  brewing, cooking, milling, cheese/egg/honey/wax, butchery, tanning, leather, wool &
  textiles, dye, wood, stone, glass (sand→glass), pottery/kilns, plaster, papermaking &
  bookbinding, soap, metal ore → smelt → alloys → forge, gem cutting & encrusting,
  mechanics, siege workshops
- **Quality tiers** on every crafted item (affects value and combat stats)
- **Manager/work orders** (conditional job automation), bookkeeper (inventory
  precision), broker (trade)
- **Nobles** with escalating demands, mandates, and punishments; barony → county →
  duchy → mountainhome progression
- **Justice system**: crimes, witnesses, convictions, jail
- **Traps & mechanisms**: pressure plates, levers, gears, linked bridges/floodgates/
  spikes/cage traps; water wheels, windmills, pumps, minecarts & rail physics
- **Trade**: seasonal caravans per civilization, trade agreements, requests, offense
  mechanics (killing traders has consequences)
- **Migration waves** scaled to fortress wealth/deaths

### 2.4 Physics & environment simulation
- **Z-level tile world**; every tile has material, shape (wall/floor/ramp/stairs),
  temperature, and contents
- **Fluids**: water & magma as cellular automata with 0–7 depth per tile; pressure,
  flow, evaporation, freezing/melting; floodgates and pump stacks; drowning; obsidian
  casting (water + magma)
- **Temperature**: items and creatures heat/cool, catch fire, melt; fires spread
  through grass and forests
- **Cave-ins**: unsupported terrain collapses with falling damage and dust
- **Weather**: rain, snow, blizzards; evil weather (undead-raising clouds)
- **Seasons & calendar**: 4 seasons, crop schedules, frozen rivers
- **Multi-tile trees** that grow over years and can be climbed; falling leaves/fruit
- **Light/subterranean status** (cave adaptation, surface crops vs cave crops)

### 2.5 Combat & military
- **Anatomical combat**: bodies of parts, parts of tissue layers (skin/fat/muscle/
  bone/organs); attacks target parts; damage types (blunt/edge/pierce) vs material
  properties of both weapon and armor
- Pain, bleeding, nausea, unconsciousness, poison/venom syndromes, severed limbs
- Wrestling (grabs, joint locks, chokes), charging, dodging, opportunity attacks,
  aimed attacks
- **Squads**: uniforms, equipment assignment, training schedules, barracks, alerts,
  patrol routes, burrows (civilian alerts)
- Ranged combat: crossbows, ammunition, archery training
- **Siege engines**: ballistae, catapults
- **Threat ladder**: wildlife → thieves/snatchers → ambushes → sieges (with mounts,
  siege ladders) → forgotten beasts, titans, dragons, hydras, necromancers, undead
- **Procedurally generated monsters** (forgotten beasts, titans, demons, night
  creatures, werebeasts, vampires) with generated names, bodies, materials, attacks
- Animal training (war dogs → exotic beasts), egg/breeding populations, grazing

### 2.6 Social & culture layer
- Taverns (visitors, mercenaries, monster slayers, spies), temples (prayer, priests),
  libraries (scholars, books, knowledge system), guildhalls (guild petitions)
- **Procedurally generated poetry, music forms, instruments, and dances** performed
  by dwarves
- Petitions for residency/citizenship from other races
- Artifact requests, museums (display cases), fortress reputation
- **The world reacts**: your artifacts get stolen, your history spawns quests,
  villains run intrigue plots against you

### 2.7 Adventure mode (phase 2 of the project — same sim, new lens)
- Character creation from any race/civ with point-buy attributes/skills
- Turn-based play in the same world; travel map + local map
- Quests from rumors and agreements; reputation system per civilization
- Stealth with vision arcs, sneaking, ambushes; tracking via footprints
- Climb/jump/sprint; swim; mounts; z-level dungeons (catacombs, labyrinths, towers,
  vaults)
- Recruit companions; party control; sleep/camp; butcher/cook/forage
- Retire characters into the world; visit/reclaim your old fortresses; your adventurer
  can later show up in Legends or as a fortress visitor

### 2.8 Meta features
- **Legends mode**: browse every historical figure, site, civilization, war, artifact
- **Data-driven content ("raws")**: creatures, materials, plants, weapons defined in
  plain-text data files → trivially moddable
- Saves that survive version upgrades; world persistence across many playthroughs
- Both ASCII/tile classic rendering and a modern sprite renderer + mouse UI (the
  Steam release proved the UI layer is what unlocked a mass audience)

---

## 3. Reality check (read this before writing code)

Dwarf Fortress is **~20 years of full-time work by a mathematics PhD** and roughly
700k+ lines of C++. A "perfect clone" is not a project — it's a career. The honest
version of this blueprint is:

- **A playable core (Rimworld-depth colony sim on a DF-style z-level world): 1–2 years
  of steady solo work.**
- **DF-class depth (history gen, adventure mode, anatomical combat, full industry
  web): open-ended, 5+ years.**

The good news: DF's own history shows the right order. Tarn built world gen and a
roguelike first, then the fortress on top. The architecture below is designed so every
phase is a *playable game*, and depth is added system-by-system without rewrites.

Key design principles taken from Tarn Adams' talks (Game AI Pro ch. 41, GDC/interviews):
1. **Simulate causes, not appearances.** Don't fake outcomes with random rolls; model
   the underlying state (a dwarf is sad *because* their friend died *because* a
   forgotten beast got in *because* you dug too deep). Stories fall out for free.
2. **Data-driven everything.** Creatures/materials/plants live in data files, not code.
   One combat engine + material properties = thousands of distinct weapons for free.
3. **One world, many lenses.** Fortress/adventure/legends are views over the same
   database. Design the world state as the product; the modes are UIs.
4. **Don't over-engineer generality up front.** Tarn writes the specific thing, then
   generalizes when a second use appears. ECS discipline helps us here, but resist
   building frameworks before features.

---

## 4. Technical architecture

### 4.1 Recommended stack

| Layer | Choice | Why |
|---|---|---|
| Language | **Rust** (alt: C++ or C# ) | Simulation perf matters (millions of tiles, thousands of agents); Rust's ECS ecosystem is the best available |
| Architecture | **ECS** via `bevy_ecs` or `hecs`/`legion` | DF itself is OOP spaghetti by Tarn's own admission; ECS gives us cache-friendly agent simulation and clean system separation |
| Rendering | **Bevy** (or macroquad/SDL2 if you want thinner) | 2D sprite/tile rendering, cross-platform, wgpu-based |
| UI | egui / bevy_ui | The Steam release proved good UI is half the product |
| Data files | **RON/TOML "raws"** | Moddability from day one |
| Saves | serde + binary (postcard/bincode) + zstd | Worlds are big |
| Scripting (later) | Lua or WASM hooks | Events, mods |

If you'd rather prototype fast and port later: **Godot 4 + C#** is a legitimate
alternative — but the fluid/pathfinding/agent hot loops will eventually want native code.

### 4.2 World representation

```
World
├── OverworldMap  (e.g. 257×257 region tiles)
│     each region: elevation, temp, rainfall, drainage, savagery,
│     biome, civilization sites, armies, geology column
├── Active LocalMap (the embark / adventure site)
│     e.g. 96×96 tiles × ~150 z-levels, chunked 16×16×16
│     Tile: { material_id, shape, designation_flags, temp,
│             fluid {type, depth 0–7}, vegetation, light,
│             occupancy list }  — pack into ≤ 8–16 bytes
└── Offscreen world  (sites simulated statistically, not tile-by-tile)
```

- **Chunking** is mandatory: dirty-flag chunks for rendering, fluid sim, and
  pathfinding invalidation.
- **The offscreen world runs on abstractions** — armies move as units, sites have
  population numbers, not tiles. Only the loaded site is fully simulated. This is
  exactly how DF stays tractable.

### 4.3 The simulation tick

Fixed-timestep tick (DF runs ~10–100 ticks/"day" scaled). Systems in order:

1. Calendar/seasons/weather
2. Fluid cellular automaton (only *active* fluid tiles — keep an active set, sleep
   settled water; this is the #1 perf trap)
3. Temperature diffusion (lazy: only tiles near heat sources/fires)
4. Plant/tree growth (slow tick, e.g. daily)
5. Item decay, fire spread, cave-in checks (event-driven, not per-tile scans)
6. Creature AI (needs → goal selection → pathfind → action)
7. Job system (match designations/work orders to idle workers)
8. Combat resolution
9. Events (migrants, caravans, sieges — scheduled off fortress wealth/date)

### 4.4 Pathfinding (the classic DF bottleneck — design it right on day one)

- A* on walkable tiles, 3D (ramps/stairs connect z-levels)
- **Connectivity cache**: flood-fill region IDs per walkable component; before any A*,
  check `region(start) == region(goal)` — O(1) rejection of impossible jobs (dwarves
  endlessly trying to path to an unreachable item will melt your CPU)
- Incrementally update region IDs when tiles change (mining, construction, doors,
  fluids)
- Upgrade path: HPA* (hierarchical) per 16×16×16 chunk when maps get big
- Separate movement capability masks: walk / climb / fly / swim → separate
  connectivity layers

### 4.5 Data-driven raws (do this before content, not after)

```ron
// data/materials/stone.ron
Material(id: "granite", category: Stone, layer: Igneous,
  density: 2650, melt_point: 12061, impact_yield: ..., value: 1, color: ...)

// data/creatures/dwarf.ron
Creature(id: "dwarf",
  body: "humanoid_std",          // references a body plan
  tissues: ["skin","fat","muscle","bone"],
  size: 60000, attributes: {...},
  needs: {alcohol: Dependent, ...},
  can_learn: true, can_speak: true)

// data/body_plans/humanoid_std.ron  — parts graph with relations
// data/plants/plump_helmet.ron, data/items/weapons/short_sword.ron ...
```

The engine knows *mechanisms* (a material has yield points; a body is a graph of
parts made of tissue layers); the data supplies *everything else*. This is the single
highest-leverage DF design decision — copy it (the idea, with our own data).

### 4.6 The event/history substrate

Every significant sim occurrence emits a **HistoricalEvent** row
(`{year, tick, type, actors[], site, details}`) into an append-only log, both during
worldgen and live play. Legends mode is just a reader over this table. Dwarf memories,
engraving subjects, art, quests, and reputation all query it. Build this in Phase 1 —
retrofitting it is misery.

### 4.7 Determinism & saves

- Single seeded RNG stream per system (worldgen reproducibility, bug reports)
- Save = serialized ECS world + tile chunks + history log; autosave on season ticks

---

## 5. Build order — 7 phases, each one playable

### Phase 0 — Skeleton (2–4 weeks)
Window, tile renderer with z-level view (show current z, dimmed z-1), camera,
fixed-tick loop, ECS scaffold, raws loader, seeded RNG, save/load of a dummy map.
**Exit test:** walk a cursor around a 3D map read from data files at 60fps.

### Phase 1 — Dig & Haul (the toy that proves the loop) (1–2 months)
- Local map gen: layered stone strata, soil, one cavern, ores in veins
- Dwarves as agents: idle wander, hunger/thirst/sleep meters
- Designations: mine, channel, stairs; mined tiles drop stone items
- Job system v1: job board → nearest capable idle dwarf → path → work → haul to
  stockpile
- Stockpiles v1, pause/step time controls, A* + region connectivity from day one
- **Exit test:** designate a 3-level staircase and a stockpile; dwarves dig it out and
  haul the stone with zero babysitting. If this feels satisfying, the game works.

### Phase 2 — Survive a Year (3–4 months)
- Farming (plots, seasons, crops), brewing, cooking; food/booze consumption
- Workshops v1 (carpenter, mason, still, kitchen, craftsdwarf), item quality tiers
- Buildings: doors, beds, tables, bridges; zones (bedroom, dining, pasture)
- Skills that improve with use; thoughts/happiness v1
- Calendar, seasons, weather, temperature v1 (freeze/thaw)
- Wildlife, hunting, butchery; fishing
- Migrant waves; basic trading caravan (menu-based)
- **Exit test:** a 7-dwarf embark survives to year 2 with a working food industry, and
  a new player can tell *why* a dwarf is unhappy.

### Phase 3 — Blood & Water (3–4 months)  ← this is where it becomes "DF-like"
- Fluid CA: water 0–7 depths, pressure, wells, floodgates, drowning; then magma
- Anatomical combat v1: body-part graph, tissue layers, material-vs-material damage,
  bleeding/pain/unconsciousness, combat log generator (our own prose)
- Military: squads, uniforms, training, alerts, burrows
- Threats: thieves → ambushes → seasonal sieges scaled to wealth
- Traps, levers, mechanisms, linked bridges; cave-ins
- Health care v1 (rest, diagnosis, sutures, splints)
- **Exit test:** a player drowns a goblin siege with a lever-operated moat, and a
  survivor is stitched up in the hospital. That's the fantasy delivered.

### Phase 4 — A World Outside (4–6 months)
- Overworld gen: heightmap → temperature/rainfall/drainage → biomes → erosion,
  rivers → geology columns → good/evil regions
- Civilizations placed; site founding; **history simulation loop** (years of wars,
  births, deaths, artifacts, rumors) writing HistoricalEvents
- Embark screen: pick a site on the real world map (geology/biome actually matter)
- Legends viewer (even a plain searchable list ships value immediately)
- Caravans/sieges now come *from real civs* with real relationships
- Deep underground: 3 cavern layers, magma sea, underworld; forgotten-beast generator
  (procedural body plans + materials + attacks)
- **Exit test:** two worlds with different seeds feel like different places, and a
  siege leader has a name you can find in Legends with a personal reason to hate you.

### Phase 5 — Hearts & Minds (3–5 months)
- Full personality model (facets/values/beliefs), preferences, relationships,
  grudges, family
- Stress pipeline: events → thoughts → emotions → stress → tantrum/insanity spirals
- Strange moods & legendary artifacts (generated names/images — our own generator)
- Nobles, mandates, justice system; taverns/temples/libraries with visitors and
  petitions; procedural art: engravings, poetry/music/dance descriptors (our own
  text corpus)
- Ghosts, burials, memorials; animal training/breeding; full industry web
  (glass, pottery, paper/books, soap, wax, dye, textiles…)
- **Exit test:** a player tells you an unprompted story about a specific dwarf.

### Phase 6 — The Second Lens: Adventure Mode (4–6 months)
- Turn-based controller over the same local-map engine; travel map
- Character creation from any civ; companions, quests from history/rumors, reputation
- Stealth (vision arcs), tracking, climbing/jumping/swimming
- Retire/unretire characters and fortresses; visit your dead forts
- **Exit test:** kill (in adventure mode) the named beast that destroyed your Phase-4
  fortress, then read the whole saga in Legends.

### Phase 7 — Forever Game (ongoing)
Sites that develop offscreen, intrigue/villain plots, necromancers & secrets,
werebeasts/vampires, economy depth, mod API + Steam Workshop–style sharing,
procedural music, multiplayer-adjacent ideas (shared worlds). This phase never ends —
by design.

---

## 6. The hard parts (budget extra respect for these)

1. **Pathfinding under constant terrain mutation** — the region-connectivity cache and
   its incremental updates. Get it wrong and mid-game FPS dies (DF's own famous "FPS
   death").
2. **Fluid performance** — never scan all tiles; keep an active-fluid set and put
   settled water to sleep.
3. **Job-system deadlocks** — item reserved by a hauler who died; job needs item
   inside a burrow the worker can't enter; circular material dependencies. Build a
   job-diagnostics debug view early.
4. **The emotion/stress balance** — DF spent years tuning tantrum spirals between
   "nothing matters" and "everyone goes insane." Expose every constant in data files
   and expect to iterate.
5. **UI for depth** — DF's classic UI gated it for 16 years; the Steam UI made it a
   hit. Budget real time for tutorialization, tooltips, alerts, and a "why is this
   dwarf idle/sad/dead" inspector.
6. **Combat text generation** — the log is the storytelling organ. Grammar-based
   generator with material/part/verb slots; write our own phrasing.
7. **Save compatibility** across dev versions — version your serialization from day 1.

---

## 7. Suggested repo layout

```
dwarf-kingdom/
├── crates/
│   ├── dk_core/        # ECS components, tick scheduler, RNG, calendar
│   ├── dk_world/       # tiles, chunks, overworld, geology, mapgen
│   ├── dk_sim/         # fluids, temperature, plants, cave-ins
│   ├── dk_agents/      # needs, personality, jobs, skills, health
│   ├── dk_combat/      # bodies, tissues, damage, wrestling
│   ├── dk_history/     # civs, worldgen history, events, legends
│   ├── dk_raws/        # data-file schema + loader + validation
│   ├── dk_ui/          # rendering, input, menus, inspector
│   └── dk_app/         # binary: fortress mode (later: adventure)
├── data/               # the raws: materials, creatures, plants, items, names
├── docs/               # this blueprint, per-system design notes
└── tools/              # worldgen visualizer, legends dump viewer, balance harness
```

---

## 8. First week of actual work

1. `cargo new` the workspace, pick Bevy vs macroquad (recommend **Bevy 0.14+**)
2. Render a 64×64×20 map of colored tiles with z-level switching and mouse cursor
3. Define `Tile` (≤16 bytes), chunk storage, and the raws loader for materials
4. One dwarf entity that wanders and paths (A* + region cache stub)
5. Mine designation → dig job → stone item on ground

Then follow Phase 1. Every phase exit-test is a demo you can put in front of a friend.
