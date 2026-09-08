# Dwarf Kingdom

A deep colony-simulation game in the spirit of the great fortress sims:
a procedurally generated 3D world, autonomous dwarves with personalities,
and physics-driven stories of triumph and collapse.

Built in Rust + Bevy. See `BLUEPRINT.md` for the full design and phased roadmap.

## Status: Phase 7 — Depth & Culture (in progress)

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
- [x] Personalities: every dwarf rolls cheer/diligence/social/bravery facets
      that change how they work, chat, grieve, and fight
- [x] Friendships from idle chatter; grief (and stress) when a friend dies
- [x] Stress pipeline: bad thoughts accumulate; boiling over means a public
      tantrum or a dark gloom, depending on temperament
- [x] Strange moods: a dwarf seizes a workshop and a boulder and creates a
      named legendary artifact the whole fort admires
- [x] Biographies: put the cursor on any dwarf and the HUD tells their story
      — temperament, tastes, best friend, craft, masterworks, latest sorrow
- [x] Headless exit tests: individuals differ, friendship→grief works,
      stress episodes fire and resolve, moods yield named artifacts, and
      every dwarf's story is unique
- [x] Adventure mode ('a' on the embark screen): play one hero turn-by-turn
      in the same simulation — the world only moves when you do
- [x] A quest from history: your civ's nemesis stalks the same map; hunt
      them down, and the deed is recorded in your saga
- [x] Headless exit tests: time is turn-based, the nemesis falls to a
      played hunt, stairs climb, the deed is logged

### The living fortress (industry, society, and the deep)

- [x] Caverns & magma: deep caverns, a magma sea, obsidian where water meets
      fire; forgotten beasts rise from the depths once you dig too greedily
- [x] Nobles & justice: a baron is appointed, issues mandates, and metes out
      punishment when they go unmet
- [x] Ghosts & burial: the unquiet unburied dead walk until laid to rest in a
      tomb; their friends find peace when they are
- [x] Trade: caravans arrive from friendly civs; a trade screen to barter your
      crafts, cloth, cut gems, and weapons for their goods
- [x] Industries: craftsdwarf's workshop, loom & textiles, jeweler & cut gems,
      fishing, animal husbandry (pastures, breeding, culling), and a **forge**
      that arms your soldiers with weapons
- [x] Temples & taverns: dwarves worship for solace and drink to shed stress;
      weather turns with the seasons and rain speeds the crops
- [x] Military: enlist soldiers who hunt raiders; **combat veterancy** makes a
      fighter deadlier the more blood they draw; forged weapons hit harder
- [x] **Animal training**: war dogs that charge raiders and guard the gates,
      trained by a handler and rendered as their own sprite
- [x] **Weapon traps** (Shift+T): a static defense — hidden blades shred any
      raider or beast that treads onto the trap
- [x] **Item quality tiers**: a skilled crafter turns out finer, dearer goods
      (up to a masterwork worth 3.5×), so skills matter to the economy
- [x] **Glass industry** (Shift+G): a furnace melts stone into blown glass, the
      fort's finest ordinary trade good
- [x] **Constructed walls** (Shift+B): masons haul stone and raise walls — seal
      a breach, wall off a burrow, or funnel raiders — not just dig
- [x] **Barracks** (Shift+I): soldiers drill between battles to become veterans
      before the first raid, not only by bleeding for it
- [x] **Hospital** (Shift+H): the wounded seek the ward and mend far faster
- [x] **Burrows & the alarm** (Shift+Z / F2): sound the alarm and civilians flee
      to a safe room while the soldiers hold the line
- [x] **Library** (Shift+L): scholars set down treatises — the fort's knowledge,
      read in the Legends viewer
- [x] The fortress can fall: when the last citizen dies it leaves an epitaph

### Adventure, culture, and the wider world

- [x] Region travel ('g'): walk your hero from one land into the next, body,
      skills, deeds, gear, and pursuing nemesis all carried along
- [x] Companions ('c'): recruit townsfolk who fight at your side and journey on
- [x] **Spoils of war**: a slain raider drops their blade; take it up ('p') and
      wield it to strike harder
- [x] Deeds → Legends: an adventurer's feats are inscribed into world history
- [x] Retire & reclaim fortresses (F8): a fort endures in the world and can be
      re-entered exactly as it was
- [x] **Procedural engravings** (Shift+D): masons smooth walls and carve into
      them scenes from the fortress's own history
- [x] **Procedural poetry**: a fort with a tavern grows an anthology of titled
      works — its own living culture
- [x] Biome-influenced surfaces: deserts of sand, swamps of clay, greens of loam
- [x] Sound: event-driven audio cues for combat, sieges, moods, and mourning

## Run

```sh
cargo run -p dk_app
```

Press **F1** in-game for the full controls overlay — the table below is the
short version.

**Getting in**

| Key | Action |
|---|---|
| Mouse | Left-click: move cursor · wheel: zoom · right-drag: pan |
| Enter | (Embark screen) found the fortress at the cursor's region |
| `a` | (Embark screen) begin an adventure at the cursor's region |
| F1 | Controls overlay · Esc closes it |

**Camera & view**

| Key | Action |
|---|---|
| Arrow keys | Move cursor (tile/dwarf info in HUD) |
| W A S E | Pan camera (`d` is taken by designate) |
| `-` / `=` | Zoom out / in |
| `[` / `]` | Z-level down / up |
| Space | Pause · `.` single-step while paused |
| 1 / 2 / 3 | Sim speed |

**Dig & build** (two-press rectangles: press to anchor, again to apply)

| Key | Action |
|---|---|
| `d` / `x` / `h` | Designate mine / stairs / channel |
| Shift+X / Shift+G | Fell trees / forage wild shrubs (drag over a patch) |
| Shift+D | Smooth-and-engrave a wall |
| Shift+B | Build a constructed wall (masons haul the stone) |
| `v` / `k` / `m` | Still / kitchen / craftsdwarf's workshop |
| `j` / `;` / Shift+K | Loom / jeweler / mason's workshop |
| Shift+M / Shift+F | Smelter (ore → bars) / forge (bars → weapons & armor) |
| Shift+C / Shift+J / Shift+N | Clothier / carpenter / tanner |
| Shift+G / Shift+P | Glass furnace* / well |
| Shift+T / `b` | Weapon trap / tomb |
| `g` / `l` / `t` | Floodgate / lever (links nearest device) / pull lever |
| *(toolbar)* | **Bridge** — drag a span for a drawbridge |
| *(toolbar)* | **Plate** — a pressure plate; anything stepping on it fires the linked gate or bridge |

\* Known conflict: `Shift+G` is currently bound to *both* the Gather
designation and the glass furnace, and one press fires both. Use the toolbar
buttons to get one without the other until it's rebound.

**Zones, labor & military**

| Key | Action |
|---|---|
| `p` / `f` / `n` | Stockpile / farm plot / pasture |
| `o` / `'` / `z` | Tavern / temple / fishery |
| Shift+H / Shift+Z / Shift+L | Hospital / burrow (safe room) / library |
| Shift+I / `i` | Barracks / enlist-dismiss the soldier at the cursor |
| `u` / Shift+U | Cull an animal / war-train a dog |
| F2 | Sound or lift the alarm (civilians flee to burrows) |
| Tab | Pick which squad the toolbar's orders apply to (none = all) |
| *(toolbar)* | Defend / Station / Patrol / Train orders · Melee / Marks loadouts · Split & Assign squads |
| `c` | Cancel designations (two-press rect) |
| Esc | Exit designation mode |

**The fortress & the wider world**

| Key | Action |
|---|---|
| `r` | Trade with a caravan |
| `y` | Open/close the Legends viewer (world history & your fort's poetry) |
| F5 / F9 | Save / load world |
| F8 | Retire the fortress (it endures, and can be reclaimed) |
| Q | Quit |

**Adventure mode**

| Key | Action |
|---|---|
| Arrows / `[` `]` / `.` | Move & attack / climb stairs / wait |
| `p` / `c` | Take a fallen foe's weapon / recruit a companion |
| `g` | Journey to the next land |
| Esc | Abandon the quest |

## License

The source is [MIT](LICENSE) — use it, fork it, ship it.

The bundled art is not all under the same terms: the toolbar icons and
happiness faces in `assets/icons/` and `assets/faces/` are [OpenMoji](https://openmoji.org),
**CC-BY-SA 4.0**, which asks for attribution and share-alike, so keep the
`CREDITS.txt` beside them if you redistribute. The generated `assets/tileset.png`
is CC0.
