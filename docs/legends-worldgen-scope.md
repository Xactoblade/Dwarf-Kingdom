# Legends & World-Building — Authoritative Scope

Status: design ground truth for implementation. Derived from `BLUEPRINT.md` ("One
world, many lenses… Legends mode is just a reader over this table", :174/:274) and
verified against the current tree (commit `6b6a19f`). Produced by a scoping
workflow (4 parallel code-mappers → 3 independent design framings → synthesis),
every load-bearing citation checked against real code.

## The one fact that governs every slice

The fortress save (`SaveOut`, `crates/dk_agents/src/lib.rs:10690`) serializes
**only `Sim`** plus registry id-lists and mod stamps. It does **not** serialize
`dk_history::World` (figures, sites, events, family links) — the World is
**regenerated from `seed` on every launch** via `sync_world`. `SAVE_VERSION = 84`
(`dk_agents:10687`), hard-rejected on mismatch (`:10749`).

Two consequences, true for every slice below:

1. **Reshaping any `dk_history` struct does NOT bump `SAVE_VERSION`** — those types
   are never in the save. A bump to 85 is required **only** if a slice adds a
   serialized field to `Sim`. As scoped, **no slice does.** (Verified: `Sim` holds
   history only as flattened `SiegeLeader{name,grudge}` strings, not
   `World`/`Figure`/`Site` snapshots.)
2. **But the World is regenerated from seed**, so ChaCha-stream position is a
   *de-facto* compatibility contract: any new `gen_range` inserted **before**
   existing worldgen draws re-rolls a *different world* under an existing fort's
   seed and breaks byte-identical headless tests. Determinism discipline (append
   enum variants at the END; add new draws only *after* existing ones or in a
   separately-salted pass) is mandatory **even without a version field.** The
   regression assertion is "generate twice from one seed → byte-identical," and for
   read-only slices "generate → byte-equal to a pre-slice golden."

---

## 1. Goal & non-goals

**Goal.** Make the generated world legible and alive at the earliest screens: a
geology line on the embark panel, rich cross-linked Legends dossiers (figure
race/role/deeds/kin), stored lineage & succession, and a handful of new non-civ
"adventure" site types on the map — all DF-faithful, all deterministic from seed.

**Non-goals (explicitly out of scope).**
- No new *saved* game state — nothing here touches the `Sim` serialization surface,
  so no `SAVE_VERSION` bump. (If any future extension persists a chosen-site or heir
  onto `Sim`, that single change bumps to 85 — flagged where it could arise.)
- No per-hover full-map geology preview by default (running `generate_terrain` on
  every cursor move is a deliberately deferred, gated tier).
- No new worldgen *simulation* behavior for existing entities (no new
  marriage/war/beast logic); we render existing data and add strictly-appended
  entities.
- No dynasty/house grouping struct, no figure→artifact/title arrays (open
  questions, not committed).
- No non-ASCII glyphs in any displayed string (Bevy default font tofus `·—→` and
  emoji — use `|`, `-`, `>` only).

---

## 2. The four sub-features

### A. Geology & embark readout

**Design intent.** BLUEPRINT.md:329/:46 wants "geology/biome to actually matter" at
site selection. The code map's hard finding: **no coarse geology is stored on
`Region`** — only `volcanism`/`elevation` are intrinsic; stone/flux/ore emerge only
from running `generate_terrain` on the region seed. So we ship the cheap,
already-derivable facts first, and gate the real preview behind a second slice with
strict stream isolation.

**Slice A1 — Coarse geology line (Tier A).** *(least risk in the whole doc)*
- **What it does.** Adds one line to the embark panel from facts already derivable
  pre-embark: surface **soil style** (from biome), **aquifer** y/n, **volcano** y/n,
  elevation/relief hint. No terrain generation.
- **Touch-points.** New pure helper `Region::geology_summary(&self) -> String` on
  `impl Region` in `dk_history` (engine-agnostic, testable, Bevy-free). Consume
  stored fields at `dk_history/src/lib.rs:291-323` (`drainage`, `rainfall`,
  `volcanism:305`, `elevation`) plus `hilly()` (`:252`) and `surroundings()`
  (`:235`). Call it from the embark `format!` — add one `\n {}` line + one arg at
  `crates/dk_app/src/main.rs:6340`+; the aquifer (`main.rs:6320-6325`) and volcano
  (`main.rs:6333`) booleans are already computed in scope and reused. Soil word via
  the existing `surface_style(region.biome)` helper. Do **not** call
  `generate_terrain` here.
- **Determinism.** Read-only over stored `Region` fields; zero `gen_range`; touches
  no serialized shape.
- **SAVE_VERSION.** No bump.
- **dk_app UI.** New line on `Screen::Embark`, ASCII, pipe-delimited, e.g.
  `geology: loam soil | aquifer | no volcano`.
- **Done / exit test.** `dk_history` unit test on `Region::geology_summary`: a
  Region with `drainage=30, rainfall=60` → contains `aquifer`; `volcanism=100` →
  contains `volcano`; assert `s.is_ascii()`. Manual: `DK_SCREENSHOT=1 cargo run -p
  dk_app` shows the line on the embark panel.

**Slice A2 — Real stone/flux/ore preview (Tier B).** *(deferred, gated)*
- **What it does.** Names likely stones + flux presence + probable ore set for the
  hovered region.
- **Touch-points.** New fn `dk_world::preview_geology(seed) -> GeoSummary` near
  `generate_terrain` (`crates/dk_world/src/lib.rs:755`), reusing
  `reg.indices_in_category(...)` (`lib.rs:763-767`) and the strata material picks
  (`lib.rs:882-931`) + `is_flux` flag (`dk_raws/src/lib.rs:45`). Called from the
  embark bindings block (`main.rs:6269-6299`) and folded into the A1 line.
- **Determinism — critical.** Must run on its **own**
  `dk_core::rng_from_seed(world.seed ^ ((rx<<32)|ry))` instance (the exact embark
  derivation), so it cannot perturb the real embark generation stream. Same seed →
  identical readout every hover.
- **SAVE_VERSION.** No bump. Do **not** cache the summary onto `Sim` (that would add
  a serialized field and force a bump — explicitly avoid).
- **dk_app UI.** Extends the A1 line, e.g.
  `geology: loam | stone granite, limestone (flux) | ore hematite | aquifer`.
- **Done / exit test.** `dk_world` test: `preview_geology(seed)` is stable across
  repeated calls; its stone set matches a column sampled from a full
  `generate_terrain(seed)`; assert calling the preview does **not** change a
  subsequent `generate_terrain(seed)` output (stream isolation).

---

### B. Legends drill-down enrichment

**Design intent.** DF Legends lets you "browse every historical figure and follow
relationships" (BLUEPRINT.md:144). Drill-down already exists one level deep
(`LegendsState.detail: Option<usize>`, `main.rs:145`; Enter opens at
`main.rs:3960-3963`; `legend_detail` at `main.rs:1265` already renders `facts` +
substring-matched deeds under a "- Chronicled deeds -" header). We enrich the
detail page, then fix the brittle substring event-matching with a typed spine, then
make it a navigable web.

**Slice B1 — Figure/Site dossier.** *(reuses existing data)*
- **What it does.** Turns the thin detail page into a real dossier. Figure: race
  (via `civ` → `civs[civ].race`), lifespan (`born_year`/`died_year`, ASCII
  `b.-d.`), role, kills, worshipped deity (`worships` → `deities[i].name`),
  necromancer flag, grudges. Site: kind noun, civ, founded year, population, ruined
  status.
- **Touch-points.** New engine-agnostic renderer `World::figure_lines(id) ->
  Vec<String>` in `dk_history` near the other `*_lines` helpers (`~lib.rs:2284-2433`)
  — none exists today. Reuse existing `site_lines()` (`lib.rs:2401`). Consume
  `Figure` fields (`dk_history:1242-1270`). Call from `legend_detail`
  (`main.rs:1265`), prepending the header block before the existing deed loop;
  enrich the Figures-arm `facts` in `legend_entries` (`main.rs:1146-1192`). Render
  path unchanged (`legends_view` detail branch `main.rs:1298-1304`, scroll handles
  overflow within `LEGENDS_PAGE = 30`, `main.rs:57`).
- **Determinism.** Pure read + display formatting; no RNG, no serialized shape.
- **SAVE_VERSION.** No bump.
- **Done / exit test.** `dk_history` test on a fixed seed: pick a `Role::Leader`,
  assert `figure_lines(id)` contains the race noun and a `b.`/`d.` span and is
  all-ASCII. Manual: open Legends (`y`), Figures tab, Enter → dossier renders.

**Slice B2 — Typed event spine (id-linked deeds).** *(the structural fix; larger)*
- **What it does.** Replaces the brittle `text.contains(&entry.key)` deed-matching
  (`legend_detail:1271`) with id references, so a figure named "Oddom" no longer
  pulls in unrelated events that merely contain the substring.
- **Touch-points.** Add `#[serde(default)]` fields to `HistoricalEvent`
  (`dk_history:1352-1356`): `subjects: Vec<usize>`, `site: Option<usize>`, `kind:
  EventKind`; keep `text` for display. New `EventKind` enum (Birth, Marriage, Death,
  Slaying, Founding, Sacking, Ascension, …) — **append variants at the END only**,
  never ordinal-indexed. Add `World::event_typed(...)` alongside `World::event`
  (`lib.rs:1424`) and route the dozen call sites through it: marriage (`lib.rs:1872`),
  birth (`lib.rs:1904`), death/succession (`lib.rs:2090-2113`), `found_site_kind`,
  beast slaying, artifact creation, tower raids (`lib.rs:1809-1820`); `record_deed`
  (`lib.rs:2269`) keeps the untyped free-text path. Add `LegendEntry.fig:
  Option<usize>` (`main.rs:1137`, UI-only struct, unserialized) so drill-down
  resolves by id. Rewrite `legend_detail` to filter `world.events` by
  `subjects.contains(&fig_id)` / `site == Some(id)`, falling back to text-match for
  legacy lines.
- **Determinism — critical.** `EventKind` is set **explicitly per call site**, never
  via `gen_range % len`; adding fields and populating ids consumes **no** RNG, so
  worldgen RNG-derived values stay byte-identical. Test enforces stream-invariance.
- **SAVE_VERSION.** No bump.
- **Done / exit test.** `dk_history` test: a figure with a known slaying is retrieved
  by `subjects.contains(id)`; a differently-named figure whose name is a substring of
  the text is NOT falsely attributed. Assert worldgen RNG values byte-equal a
  pre-slice golden.

**Slice B3 — Cross-navigation (nav stack).** *(UI payoff, data-cheap)*
- **What it does.** Turns index→detail into a navigable web: from a figure page,
  jump to spouse/parent/child/heir figures and to founded/razed sites.
- **Touch-points.** Replace `LegendsState.detail: Option<usize>` (`main.rs:145`)
  with a nav stack `nav: Vec<LegendRef>` (`{cat, id}`) plus a `link_cursor` (init at
  `main.rs:1809-1815`). Detail render (`legend_detail:1265`, `legends_view:1298-1304`)
  emits a cursored link list at the page foot. Input (`handle_input` legends block
  `main.rs:3916-3990`): in detail mode Up/Down move `link_cursor`; Enter **pushes**
  the linked ref (extends `:3960-3963`); Esc **pops** (extends `:3981-3984`), closing
  to `from` only when the stack empties; tab-switching stays gated on `!in_detail`.
  Push/pop must mark `LegendsState` changed so `update_hud` repaints
  (`main.rs:6384-6402`).
- **Determinism.** None — pure Bevy `Resource` view/input state, never serialized.
- **SAVE_VERSION.** No bump.
- **Done / exit test.** Input-driven test: from a figure detail, Enter on a "spouse"
  link pushes to depth 2 and renders the spouse; Esc pops to depth 1; a second Esc
  closes to `from`; a "founded: SITE" link jumps to the Sites detail.

---

### C. Family, lineage & succession

**Design intent.** DF "Relationships: family, children, grudges"
(BLUEPRINT.md:62/:339). The stored data already exists and is populated
deterministically in `family_year` (`dk_history:1850-1904`): `spouse`
(bidirectional), `parent` (single), `children` (list). Gaps: co-parent, persisted
heir, dynasty. We render first, then persist the safe derivations, and defer the
rest.

**Slice C1 — Lineage render (read-only).** *(no data-model change)*
- **What it does.** Appends an ASCII "Kin" block to the B1 dossier: parent line,
  spouse line, children list, and a computed heir note (eldest living child `>=16`,
  mirroring the live succession query at `dk_history:2090-2113`, read-only).
- **Touch-points.** Extend `World::figure_lines` (B1) with a private
  `World::lineage_of(id) -> Vec<String>` helper in `dk_history`, walking existing
  `spouse`/`parent`/`children` and resolving ids to names.
- **Determinism.** Pure read of existing fields; zero `gen_range`; must not move
  worldgen output.
- **SAVE_VERSION.** No bump.
- **Done / exit test.** `dk_history` test on a fixed seed: find a figure with
  non-empty `children`, assert `lineage_of(id)` names the spouse and >=1 child and
  marks an heir when a child is `>=16` and living; assert ASCII-only. Regression:
  `legends_lines()` byte-output unchanged vs a stored golden.

**Slice C2 — Persist co-parent + heir.** *(safe data-model change; optional)*
- **What it does.** Closes two gaps as *stored* data: co-parent and persisted heir.
- **Touch-points.** Append `#[serde(default)]` fields to `Figure`
  (`dk_history:1242-1270`): `parent2: Option<usize>`, `heir: Option<usize>`. In
  `family_year` at birth (`lib.rs:1893-1895`), set `child.parent2 = Some(spouse)` —
  the co-parent is already in hand, **pure assignment, no new draw**. In succession
  (`lib.rs:2090-2113`), keep the identical child-selection logic and additionally
  write `deceased.heir = Some(chosen)`. Init both `None` in `spawn_figure`
  (`lib.rs:1543`).
- **Determinism — critical.** **Zero new `gen_range` anywhere** — both links are
  deterministic derivations of data already drawn, so the ChaCha stream is
  byte-for-byte unchanged.
- **SAVE_VERSION.** No bump.
- **Done / exit test.** `dk_history` test on a fixed seed: a `Role::Leader` who died
  of old age with a grown child → `figures[dead].heir == Some(child)`; a figure born
  in-history has both `parent` and `parent2` set, each a spouse of the other.
  Regression: `generate(seed)` byte-equals a pre-slice capture except the added
  figure fields.

---

### D. New site types (adventure sites)

**Design intent.** DF's non-civ "z-level dungeons (catacombs, labyrinths, towers,
vaults)" (BLUEPRINT.md:137-138). Adds Tomb / Cave / Vault / Labyrinth as non-civ
lairs/ruins, appearing as new embark glyphs and Legends Sites entries, reusing the
B1/C1 detail pages for free. Highest work (sprites + a placement path + the widest
determinism radius) and it *pays off* the earlier slices.

**Slice D1 — Kinds + noun + population + glyphs (no placement yet).**
- **What it does.** Adds the four variants and all their exhaustive-match arms and
  sprite rows, so they compile and render, before wiring any placement.
- **Touch-points (three exhaustive matches make the compiler enumerate the edits).**
  1. `SiteKind` — **append Tomb/Cave/Vault/Labyrinth after `Tower` at
     `dk_history:1113`** (END only; serde is variant-name-keyed, so appending is
     disk-safe and, since kind is never `gen_range`-indexed, shifts no stream).
  2. `SiteKind::noun()` — `dk_history:1116-1125` (exhaustive).
  3. `found_site_kind` population `match kind` — `dk_history:1509-1516`; each new arm
     draws its `gen_range` **exactly once** to keep per-call stream cost uniform
     (Tomb/Vault small `20..120` like Tower; Cave/Labyrinth uninhabited — a
     documented no-draw arm).
  4. Embark glyph/color `match site.kind` — `main.rs:5641` (exhaustive, no
     wildcard); give each a glyph+tint.
  5. `data/tileset.ron:38-39` — add `"m_tomb"/"m_cave"/"m_vault"/"m_labyrinth"` glyph
     rows, list each in `tinted:` (`:49-50`), add four grayscale silhouette cells to
     the sprite sheet. **The real manual work** (art-gen billing-blocked — hand-draw
     / reuse silhouettes, or a Forge art-pipeline candidate; keep RNG placement
     Claude-authored).
- **Determinism.** Appending the variant + arms shifts nothing (no placement yet).
- **SAVE_VERSION.** No bump.
- **Done / exit test.** Compile-time coverage from the exhaustive matches. `main.rs`
  test: each new glyph key resolves in `atlas.index` to a **non-zero** cell.
  `noun()` round-trip per kind.

**Slice D2 — Salted placement post-pass.** *(the determinism crux)*
- **What it does.** Places the new sites so they appear on the map and in Legends
  with their own founding events.
- **Touch-points.** A dedicated founder run **after the historical simulation
  completes**, driven by its **own salted stream** `dk_core::rng_from_seed(seed ^
  SALT)` (a fresh named constant), appending sites + events at the tail. Mirror the
  necromancer-`Tower` template (`found_site_kind(..., SiteKind::Tomb, false)`,
  announce=false). Cave placement may key off geology (Slice A). **Audit the
  tower-raid filters:** source `s.kind == SiteKind::Tower` (`dk_history:1809`)
  correctly excludes them; **target `s.kind != SiteKind::Tower` (`dk_history:1820`)
  would wrongly make uninhabited Tombs/Caves raid targets** — add `&& s.population >
  0` (or a kind allowlist).
- **Determinism — critical.** Do **NOT** insert draws into `advance_year`
  (`dk_history:2002-2020`) — that shifts every downstream draw for all seeds. The
  salted post-pass leaves the existing ChaCha stream **byte-identical**: old seeds
  regenerate their old worlds unchanged and only gain the new tail entities. New
  sites must take the **highest ids** (append-only) — never renumber existing
  sites/civs, since `Sim` may hold indices.
- **SAVE_VERSION.** No bump.
- **Done / exit test.** `dk_history` test on a fixed seed: `world.sites` gains >=1 of
  each new kind with valid `region`/`id`; and the **pre-existing**
  `legends_lines()`/site count for the main stream is byte-identical to a stored
  golden (proves the salted pass didn't shift the main stream). `dk_app`: glyph
  `match` compiles with no wildcard; `DK_SCREENSHOT=1` on a seed known to place one
  shows a new-kind glyph.

---

## 3. Recommended FIRST slice

**Build Slice A1 (coarse geology embark readout) first.**

Justification — it wins on all three axes the constraints care about:
- **Smallest & safest.** One pure `Region::geology_summary` helper + one `format!`
  line at `main.rs:6340`. Read-only over stored `Region` fields; zero `gen_range`;
  touches no serialized shape; no `SAVE_VERSION` risk and no ChaCha-stream risk — the
  two failure modes this project guards hardest. All three candidate plans
  independently ranked it least-risk/least-work.
- **Highest signal per byte.** It lands on the **first screen the player sees** and
  validates the whole workflow: a new engine-agnostic `dk_history` helper, an
  ASCII-only displayed string, an embark-panel edit, and a `is_ascii()` +
  `DK_SCREENSHOT` exit test — the exact muscles every later slice reuses.
- **Zero prerequisites.** Nothing blocks it; it unblocks the geology-aware Cave
  placement in D2 later.

---

## 4. Dependency ordering

```
A1 (coarse geology)                      [no deps — BUILD FIRST]
  └─> A2 (real preview, dk_world)         needs A1's panel line; optional/deferred
      └─> (feeds) D2 Cave placement       geology-keyed placement, optional

B1 (dossier render)                      [no deps]
  ├─> B2 (typed event spine)              enriches B1's deed list; adds LegendEntry.fig
  │     └─> B3 (cross-nav stack)          uses id-links exposed by B2
  └─> C1 (lineage render)                 appends to B1's dossier
        └─> C2 (persist parent2/heir)     stores what C1 renders
              └─> B3 (heir/kin links)     cross-nav also consumes C2's stored links

D1 (kinds + noun + glyphs + sprites)     [no deps — the art long-pole]
  └─> D2 (salted placement post-pass)     needs D1's variants; reuses B1/C1 detail;
                                          optionally A2 for geology-keyed Caves
```

Recommended build sequence (least-to-most risk, respecting deps):
**A1 → B1 → C1 → B2 → C2 → B3 → D1 → A2 → D2.** A1/B1/C1 are pure read-only UI with
no determinism or save risk; B2 and C2 must each prove **zero** stream drift; D2 is
isolated last because it is the only slice that adds worldgen entities and needs
sprite assets.

| Slice | RNG/worldgen | SAVE_VERSION | Work | Notes |
|---|---|---|---|---|
| A1 coarse geology | none | none | XS | build first |
| A2 stone preview | isolated own-rng | none | M | do NOT cache onto Sim |
| B1 dossier | none | none | S | new `figure_lines` |
| B2 typed events | none (explicit kinds) | none | L | EventKind appends at END |
| B3 cross-nav | none (Bevy Resource) | none | M | nav stack replaces `detail` |
| C1 lineage render | none | none | M | read-only |
| C2 persist heir/parent2 | none (assignment only) | none | S | prove byte-equal |
| D1 kinds + sprites | none (no placement) | none | M+art | 3 exhaustive matches |
| D2 salted placement | isolated salted stream | none | L | fix raid-target filter `:1820` |

---

## 5. Open questions

1. **Dynasty / house grouping.** No bloodline/house struct exists; a "dynasty" is
   only the implicit `parent`/`children` chain. Add a `house: Option<usize>` on
   `Figure` + a lightweight `Vec<House>` on `World`, or keep dynasty purely derived
   at render time?
2. **A2 default vs opt-in.** Should the real stone/flux preview run on every embark
   hover, or only on a keypress (e.g. `g`)? Recommend opt-in.
3. **New-site population semantics.** Are Cave/Labyrinth strictly uninhabited
   (population 0), or do they host beasts/bandits with a small population? Decides the
   `found_site_kind:1509` arm and raid-target validity.
4. **Placement rate & biome gating for D2.** How many of each kind per world, keyed
   off what (evil/savage regions, geology from A2, elevation)?
5. **Beast/artifact id-linking under B2.** `Megabeast.slayer` and `Artifact.creator`
   are *name strings*, not figure ids (`dk_history:1314/1331`). Upgrade to ids in
   B2's typed-event pass, or leave name-matched for now?
6. **Legacy event back-fill.** Since `World` is regenerated every launch there are no
   persisted legacy events — confirm we can populate ids at generation for *all*
   events and drop the text-match fallback entirely.
