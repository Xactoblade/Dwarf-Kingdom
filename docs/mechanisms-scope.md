# Mechanisms & Fluid Engineering — Authoritative Scope

Status: design ground truth for implementation. Grounded in a five-agent code map
of the real tree (commit at time of writing) and `BLUEPRINT.md` (bridges/linked
mechanisms are listed as planned: :85, :305, :319). Cited `file:line` refs are
into `crates/dk_agents/src/lib.rs` unless another crate is named.

## The facts that govern every slice

1. **Fluid already lives on the tiles.** `dk_world::Tile.water: u8` / `.magma: u8`
   (0–7, `dk_world/src/lib.rs:61,63`). `dk_sim::FluidSim` is a *controller* over
   `&mut Map`, not a fluid container — there is no separate grid. You move fluid by
   `map.set_water(p, w)` then `sim.water.wake(p)`.
2. **The fluid CA moves fluid DOWN and sideways only — never up.** `step`
   (`dk_sim/src/lib.rs:118`) does Fall (`z→z−1`), Spread (same-z, 4 neighbours),
   and `level_bodies` (a *single-z* horizontal equaliser mislabelled "pressure").
   There is **no hydrostatic head / communicating vessels** — water will not self-
   rise up a connected shaft. So **a pump must be game-layer logic** that reads an
   intake tile, decrements it, increments the output tile one z up, and `wake()`s
   both. Springs (`dk_sim:124`) are the proven "inject fluid each step" pattern.
3. **The fluid CA is wake-list driven** (`#[serde(skip)] active: BTreeSet<Pos>`,
   stepped over, `wake_all` only on load). **Every new mechanism sim MUST copy
   this** — step only a dirty-set, never sweep the whole map per tick. This is the
   FPS contract.
4. **Runtime passability = `Map::walkable`** (`dk_world:157`) = `shape.is_walkable()
   && water < DEEP_WATER && magma == 0`. Both the reachability cache and A* derive
   from it, so a device only has to (a) make its tiles return the right `walkable`
   and (b) set `regions.dirty = true`. The `Regions` cache (`dk_world/src/path.rs:88`)
   is a full-BFS rebuild throttled every `REGION_REBUILD_INTERVAL = 20` ticks
   (`:91`); **A\* is the authority in the gap** — no incremental update needed.
5. **The lever↔floodgate link is the exact template** for every device. `Lever
   { target: Pos }` (`:543`) → `pull_lever` (`:2797`) → `toggle_floodgate` (`:2808`):
   flips `TileShape::Gate ↔ Floor`, `displace_water`, sets `regions.dirty` +
   `map_changed`, wakes both CAs. Gate open/closed state lives on the **tile
   shape**, not the `Building` record.
6. **Machine state belongs in a `BTreeMap<Pos, _>` overlay on `Sim`**, NOT on
   `Tile` (which is `Copy`, 4 fields, no spare byte, multiplied across the whole
   3D volume). The precedent is `constructions: BTreeMap<Pos, bool>` (`:1884`) and
   the `aquifers`/`cavern_floors`/`blood` overlays (`:1910–1921`) — ordered
   `BTree*` (deterministic iteration), `#[serde(default)]`, empty by default.
7. **Determinism & save discipline:** `SAVE_VERSION = 84` (`:10687`), bincode,
   positional, hard-rejected on mismatch. New `Sim` fields append at the END,
   `#[serde(default)]`, and bump `SAVE_VERSION` with a changelog line (`:10668`).
   New enum variants append at the END (bincode discriminants + `gen_range % len`
   are both order-sensitive). Keep new overlays **empty by default** so the
   headless suite stays byte-identical (RNG stream untouched).

---

## 1. Goal & non-goals

**Goal.** Bring DF's engineering pillar to the fort: player-linked **drawbridges**
and **pressure plates**, **cage traps** alongside the existing weapon trap, and a
**pump + power** chain (water-wheel → gears/axles → screw pump) that lifts water up
z-levels — the flood-defence and irrigation fantasy — all deterministic and
FPS-safe on the existing wake-list architecture.

**Non-goals (this phase).**
- No minecarts / hauling routes, no rollers, no complex logic gates (repeaters,
  memory) — DF's advanced computing layer.
- No windmills (wind power) in the first pass — water-wheels only; wind is a trivial
  follow-on once the power network exists.
- No cave-ins / structural collapse (a separate physics system).
- No fluid pressure/communicating-vessels rewrite of the CA — the pump is the
  deliberate, bounded way water goes up.
- No new build-JOB/materials pipeline unless a slice needs it — `add_building`
  places instantly today (`:2730`); mechanisms follow suit until a slice justifies
  otherwise (flagged in Open Questions).

---

## 2. Core abstractions (how each maps onto the code)

**A. Generalized trigger→device link.** Today the link is hard-wired: `target`
lives only on `Lever`, `pull_lever` calls only `toggle_floodgate`, `add_lever`
searches only for `Floodgate` (`:2781`). Generalize to: a **trigger** (lever,
pressure plate) holds a `target: Pos`; a single `activate_link(target)` dispatches
on **what device is at `target`** (floodgate → toggle; bridge → raise/lower; cage →
n/a). This decouples trigger from device and unblocks bridges/plates/cages with no
per-pair code.

**B. Machine overlay.** `machines: BTreeMap<Pos, Machine>` on `Sim` (beside
`constructions`, `:1884`), `#[serde(default)]`, empty by default. `Machine { kind:
MachineKind, powered: bool, .. }` where `MachineKind` is `WaterWheel | Gear | Axle |
ScrewPump { .. }`. This is the home for gears/axles/water-wheels/pumps and their
orientation/power state.

**C. Power network.** A connectivity pass over adjacent `machines`: BFS from each
`WaterWheel` (source) through `Gear`/`Axle` links to `ScrewPump` sinks, marking
`powered`. Recomputed **only when the network changes** (a machine built/removed, or
a wheel's water flow starts/stops) — cached in a `#[serde(skip)]` dirty flag, never
per tick. FPS-safe by construction.

**D. Pump fluid-transfer.** A powered `ScrewPump` on the fluid tick: read
`map.water_at(intake)`, if `>0` move one unit to the output tile one z up (respecting
`MAX_WATER`/`holds_water`), `wake` both. This is the only "water goes up" in the game
and it is bounded (one unit/pump/tick).

---

## 3. Phased slices (least-risk-first)

### Slice 1 — Generalize the trigger→device link *(refactor, no new behaviour)*
- **What.** Extract `pull_lever`→`toggle_floodgate` into `activate_link(&mut self,
  target: Pos) -> bool` that dispatches on the device at `target` (a `match` on the
  building/overlay there). `pull_lever` (`:2797`) calls it; floodgate behaviour is
  byte-for-byte unchanged.
- **Touch-points.** `pull_lever` (`:2797`), `toggle_floodgate` (`:2808`), `add_lever`
  (`:2781`). New private `activate_link`.
- **Determinism / SAVE.** No RNG; no shape change; `Lever { target }` unchanged →
  **no `SAVE_VERSION` bump.**
- **FPS / pathfinding.** None (same code path).
- **UI.** None — invisible refactor.
- **Done.** Existing lever/floodgate tests still pass; a unit test asserts
  `activate_link(gate_pos)` toggles the gate exactly as `toggle_floodgate` did.

### Slice 2 — Drawbridge *(first visible payoff; floodgate template)*
- **What.** A multi-tile bridge span, raised/lowered by a linked lever. Lowered =
  walkable `Floor` across a gap; raised = the span opens (tiles → `Empty`, not
  walkable). Reuses ALL the floodgate pathfinding machinery.
- **Touch-points.** New `bridges: BTreeMap<Pos, Bridge>` overlay on `Sim`
  (`~:1921`, init `~:2065`), where `Bridge { anchor: Pos, span: Vec<Pos>, raised:
  bool }`. New `BuildingKind::Bridge` (append at END, `:544`) + `name()` (`:546`).
  A place path like `add_building`'s floodgate branch (`:2737`). `activate_link`
  (Slice 1) gains a bridge arm that flips each span tile `Floor↔Empty`, sets
  `regions.dirty` + `map_changed`, wakes CAs (mirror `toggle_floodgate:2822`). The
  overlay remembers the span so raising/lowering restores the right tiles.
- **Determinism / SAVE.** No RNG. New overlay + enum variant → **bump
  `SAVE_VERSION` → 85**, `#[serde(default)]`, changelog line.
- **FPS / pathfinding.** One `regions.dirty` per toggle (already throttled); A*
  correct in the gap. No per-tick cost.
- **UI.** Toolbar Build tool "Bridge" (drag a span); a bridge glyph (new tileset
  cell — draw directly into a free cell like the D1 markers); lever links to it.
- **Done.** A test: build a bridge over a gap, link a lever, pull it — `walkable`
  flips for every span tile and `regions.same_region` across the gap changes
  accordingly. `DK_SCREENSHOT` shows raised vs lowered.

### Slice 3 — Pressure plate *(automatic trigger)*
- **What.** A plate that fires its linked device when an actor steps onto it — the
  first non-manual trigger. `BuildingKind::PressurePlate { target: Pos }` (append at
  END). A hook in the movement code (where `trap_at` is checked, `:9460`, but
  generalised beyond the Fight path) calls `activate_link(target)` on step-on.
- **Touch-points.** `BuildingKind` (`:544`), a `plate_at(pos)` like `trap_at`
  (`:5384`), a step-on dispatch added to the per-dwarf/creature move (near `:9460`
  but also on ordinary movement — see the trap coverage gap below). `activate_link`
  (Slice 1) does the rest.
- **Determinism / SAVE.** The trigger fires deterministically on movement (no RNG).
  Enum variant → **bump `SAVE_VERSION`.** Keep behaviour off in headless tests
  (no plates placed) → byte-identical.
- **FPS.** One `plate_at` lookup per move (linear over `buildings`, as `trap_at`
  already is — fine at fort scale; a `BTreeSet<Pos>` cache if it ever bites).
- **UI.** Build tool "Pressure Plate"; a glyph; optional trigger condition
  (creature vs friend) deferred (Open Q).
- **Done.** Test: a creature stepping on a plate linked to a bridge raises it;
  stepping off does not re-fire endlessly (edge-trigger, not level).

### Slice 4 — Cage trap + weapon-trap coverage *(trap variety)*
- **What.** A cage trap that **captures** a walking hostile (into a new caged state)
  rather than wounding it, alongside the existing weapon trap. Also fix the trap
  **coverage gap** — `spring_trap` fires only from the Fight-movement tail
  (`:9460`), so traps miss non-combat movement; route all hostile step-onto-trap
  through one dispatch.
- **Touch-points.** `BuildingKind::CageTrap` (append). `spring_trap` (`:5391`)
  stays for weapon traps; a new `spring_cage` captures (a `caged: bool`/`captured_by`
  on the creature, or a `caged: BTreeSet<usize>` overlay). Generalise the trigger
  site (`:9460`) so any hostile entering a trapped tile is caught. Building needs
  per-instance armed/loaded state → the first real case for a `Building` field or a
  `traps: BTreeMap<Pos, TrapState>` overlay.
- **Determinism / SAVE.** `spring_trap` already draws RNG (`:5392`) — keep the cage
  path RNG-light and appended. Enum variant + state → **bump `SAVE_VERSION`.**
- **FPS.** Same per-move lookup as Slice 3.
- **UI.** Build tool "Cage Trap"; captured creatures visible (a cage glyph / a
  stocks entry). Freeing/taming deferred (Open Q).
- **Done.** Test: a hostile walking onto a cage trap ends up caged (removed from the
  threat set), not merely wounded; a weapon trap still wounds.

### Slice 5 — Hand pump *(water goes UP — payoff without the power network)*
- **What.** A dwarf-operated screw pump: a building a dwarf works as a job, moving
  one unit of water from its intake tile up one z to its output each work-tick. The
  bounded, deterministic answer to "the CA never lifts water."
- **Touch-points.** `BuildingKind::ScrewPump` (append). A pump job in the dwarf AI
  loop (like Well-seeking, `:6569`). The transfer runs game-layer: `map.water_at`
  intake → decrement, `map.set_water` output (respect `MAX_WATER`/`holds_water`),
  `self.water.wake(intake)` + `wake(output)`. Reuses the exact `wake` pattern at
  `:2745`.
- **Determinism / SAVE.** No RNG (the pump moves a fixed unit). Enum variant →
  **bump `SAVE_VERSION`.** Headless test forts can build a pump and step
  deterministically.
- **FPS.** The transfer is O(1) per pump per work-tick; only wakes 2 tiles.
- **UI.** Build tool "Screw Pump" (intake/output orientation); a pump glyph; the
  water visibly rises a level.
- **Done.** Test: a pump with a full intake and empty output, worked N ticks, has
  moved N units up one z (conserving total water); a headless golden fingerprint of
  the fluid field is stable across runs.

### Slice 6 — Power network: water-wheels, gears, axles *(the big subsystem)*
- **What.** Mechanical power: a **water-wheel** over flowing water is a power source;
  **gears/axles** transmit it; a powered **screw pump** runs itself (no dwarf). The
  full DF engineering chain.
- **Touch-points.** The `machines: BTreeMap<Pos, Machine>` overlay (§2B). New
  `MachineKind` arms (append). A `recompute_power()` BFS from wheels through
  gear/axle adjacency to pumps, cached behind a `#[serde(skip)] power_dirty` flag,
  run only on a machine edit or a wheel's flow change — NOT per tick. A wheel's
  "spun" state reads `map.water_at` of its tile each fluid tick but flips
  `power_dirty` only on a 0↔flowing transition. Powered pumps run their Slice-5
  transfer automatically in the fluid tick (`:4732` block).
- **Determinism / SAVE.** BFS over the ordered `machines` map (deterministic). New
  overlay + `MachineKind` enum → **bump `SAVE_VERSION`.** Empty by default →
  headless byte-identical.
- **FPS.** The ONLY real risk in the feature. Power recompute is edit-triggered, not
  per-tick; the per-tick cost is just powered pumps doing O(1) transfers. Must be
  wake/dirty-driven (the FluidSim contract, fact #3) — a full-machine sweep per tick
  is the failure mode to avoid.
- **UI.** Build tools "Water Wheel", "Gear Assembly", "Axle"; glyphs; a powered/
  unpowered tint; the pump runs with no dwarf when powered.
- **Done.** Test: wheel-over-flow + axle + gear + pump ⇒ pump `powered` and lifts
  water with no dwarf; cutting the axle (remove a machine) drops power and the pump
  stops within one recompute; power BFS is deterministic across runs.

---

## 4. Recommended FIRST slice

**Build Slice 1 (generalize the trigger→device link) first.**

- **Smallest & safest.** A pure refactor of an existing, tested path
  (`pull_lever`→`toggle_floodgate`); floodgate behaviour is preserved byte-for-byte.
  No RNG, no new overlay, **no `SAVE_VERSION` bump**, no FPS or pathfinding change —
  none of the failure modes this project guards.
- **Highest leverage.** It is the keystone abstraction every later slice depends on
  (bridges, pressure plates, cages all dispatch through `activate_link`). Doing it
  first means Slices 2–4 add a device or a trigger, not a new bespoke link each time.
- **Then Slice 2 (drawbridge)** is the first *visible* milestone and the natural
  demo — it reuses the floodgate template end to end and proves the overlay + link +
  pathfinding-invalidation loop before the fluid/power slices raise the stakes.

If you'd rather lead with something the player sees, do **Slice 1 + Slice 2
together** as the opening milestone (the refactor is small enough to fold in).

---

## 5. Dependency ordering

```
S1 generalize link  ── [no deps — BUILD FIRST]
  ├─> S2 drawbridge         (dispatches via S1; first visible)
  ├─> S3 pressure plate     (auto-trigger via S1; wants a device to fire — pairs with S2)
  └─> S4 cage trap          (uses the S3 step-onto-tile dispatch; trap coverage fix)

S5 hand pump  ── [no deps — the "water goes up" payoff, independent of S1–S4]
  └─> S6 power network      (water-wheel→gears→axle→pump; powers the S5 pump automatically)
```

Recommended build sequence: **S1 → S2 → S3 → S4 → S5 → S6.** S1–S4 are the
trigger/device family (safe, incremental, floodgate-shaped). S5 delivers the fluid
payoff on its own. S6 is last — the only FPS-sensitive subsystem, and it *powers*
S5's pump, so S5 de-risks it.

| Slice | RNG | SAVE_VERSION | FPS/path risk | Work |
|---|---|---|---|---|
| S1 link generalize | none | none | none | S |
| S2 drawbridge | none | +1 | one dirty/toggle | M |
| S3 pressure plate | none | +1 | per-move lookup | M |
| S4 cage trap | light (existing) | +1 | per-move lookup | M+ |
| S5 hand pump | none | +1 | O(1)/pump | M |
| S6 power network | none | +1 | edit-triggered BFS | L |

---

## 6. Open questions

1. **Build-job pipeline.** `add_building` places instantly today. Do mechanisms
   (pumps, water-wheels) need a haul-materials build job, or stay instant like every
   current building? (Instant is consistent; a job is more DF-faithful. Recommend
   instant for now, revisit if a materials economy for mechanisms is wanted.)
2. **Bridge raise = fling/crush?** DF drawbridges raise to fling or crush occupants.
   Include in S2 or defer? (Defer — the walkable toggle is the core; crushing is a
   combat-adjacent enhancement.)
3. **Pressure-plate trigger conditions.** Creature-only vs friend-triggered, weight/
   water-depth thresholds — DF has all. Ship a plain "any actor steps on" edge
   trigger in S3 and add conditions later?
4. **Caged-creature lifecycle.** After S4 capture: release, tame (war animals
   already exist), or execute? Where does a caged creature's state live (a `caged`
   overlay vs a field on the creature)?
5. **Wind power.** Once S6's network exists, a windmill is a trivial second source.
   In scope or a follow-on?
6. **Bridge tile representation.** Overlay + `Floor↔Empty` flip (proposed) vs a new
   `TileShape::Bridge`. The overlay avoids a `Tile` serialization change beyond the
   `Sim` field; confirm it renders cleanly (raised span shows as open gap).
