# Mod API — Scope (Phase 7 "mod API + Workshop-style sharing")

Design note for DK's data-file modding. Ground truth is `BLUEPRINT.md`. Scoped
via a survey→design→adversarial-critique workflow; this doc is the **critique-
corrected** plan. DF-faithful modding is **data files (raws), not a scripting
API** — that is a hard line here.

**What ships this phase:** moddable **materials, plants, gems, (optionally)
weapons, and prices** via folder-drop — explicitly **not** new items, buildings,
workshops, recipes, creatures, biomes, or civilizations (those sit behind the
deferred `ItemKind`/`CraftKind`/`BuildingKind`/`dk_history` enum wall). Set
expectations accordingly: this is the modding *foundation* plus two proofs that a
compiled content axis can become a data registry — not yet the content most
players picture.

---

## 1. The two hard walls (why this is staged the way it is)

1. **The enum-vs-data wall.** Most content is Rust enums the sim matches on
   exhaustively — `ItemKind`, `CraftKind`, `BuildingKind` are one coupled recipe
   web (~6 exhaustive matches + 13 `craft_*_pair` helpers + ~8 render matches).
   The economy scope already deferred migrating `ItemKind`; we keep it deferred.
   **Gems** are the one content axis with *zero* exhaustive matches (only
   `gem_name`/`gem_color`/`gem_value` lookups), so they're the safe first
   enum-to-registry proof.
2. **Determinism.** The RNG is one positional ChaCha8 stream per subsystem; any
   draw whose *count* is a content-list length shifts every later draw. Adding a
   material changes `indices_in_category().len()` → shifts ore-vein/strata/
   per-dwarf draws. **So content changes are an RNG input, and a modded world
   must be *detected* (rebuilt), not silently mis-generated.** The critique
   killed the original "Slice 1 touches no RNG" framing — it was false.

**Mitigation = a content hash, and it must hash the RNG-relevant fields, not
just ids.** A last-wins override that keeps an id but re-tags a stone's
`category` leaves ids identical yet shifts the stream — so the hash covers
**(id, category)** for materials, ids for plants/gems, and the resolved mod
list. Detection (rebuild/refuse), not prevention; true per-domain RNG
sub-streams are a later, larger effort.

---

## 2. Non-goals (this phase)

- **No scripting** — no Lua/WASM/executable mod code. Declarative RON only. The
  security model reduces to data validation.
- **No live hot-reload** — raws load once at startup; the map bakes live raws at
  embark.
- **No `ItemKind`/`CraftKind`/`BuildingKind` migration** — the recipe engine is
  its own future phase. `ItemKind::key` stays the join key into `prices.ron`, so
  mods can still retune prices of existing items.
- **No `dk_history`/worldgen content modding** (races, biomes, beasts, spheres,
  name corpora) — all one compiled RNG stream.
- **No Steam Workshop** — folder-drop (`mods/`) only; Workshop is a later line.
- **No new `MaterialCategory`** — closed enum; mods add within existing
  categories only.

---

## 3. Phased plan (critique-reordered: detection ships *with* the loader)

### Slice 1 — External mod folders (materials & plants), additive, with content-hash detection  ← THIS SLICE
The minimal *safe* unit. Adding content is an RNG input, so detection cannot lag
the loader.
- `ModManifest` RON (`mod.ron`, single struct): `id, name, version,
  target_game_version, load_after`.
- `mods/` discovery beside `data/` + `DK_MODS` env; base `data/` always first;
  load order = alphabetical by mod dir (explicit `load_order.ron` deferred).
- `Raws::load_with_mods(base, roots)` — additive merge of materials/plants across
  roots. **Duplicate id across roots = hard error** (no silent override this
  slice — that's the correctness hazard the critique flagged).
- `Raws::content_hash()` over materials **(id, category)** + plant ids + resolved
  mod (id, version); folded into the **world save** guard (`WORLDGEN_VERSION`
  bump) so a modded world with a different hash is **rebuilt**, not silently
  mis-generated. Fort saves are unchanged — their id-manifest+remap already
  tolerates added materials.
- **Exit test:** drop `mods/better_stone/{mod.ron, materials/orichalcum.ron}`;
  launch → the new stone is in the registry and renders with its own color; log
  reads `loaded mod "Better Stone" v1.0.0`. Generate a world with the mod, then
  relaunch without it → the game detects the content change and rebuilds the
  world rather than loading a mismatched map. No mods → vanilla unaffected.

### Slice 2 — Validator registry + mod-named errors + declared overrides
- `Vec<fn(&Raws)->Result<()>>` generalizing `validate_economy`; per-def
  provenance so every failure names the mod.
- Introduce **declared** override: a mod lists `overrides: [ids]` in its manifest
  → last-wins; an *undeclared* duplicate stays a hard error naming both mods.
- Non-empty guards for worldgen-required categories; relax the
  `EconomyConfig::Default` parity test to the base file so a mod's `prices.ron`
  is legal.

### Slice 3 — Fort-save mod-stamping + full determinism gate
- Record the active mod set (id+version) + `content_hash` in the fort save; bump
  `SAVE_VERSION`; on load, refuse a mod-set-mismatched fort with a named message.
  (Honest: bincode's exact-version reject means each such bump orphans older
  saves — stated, not hidden.)

### Slice 4 — Gems as the first data-driven enum (`GemRegistry`)
- `GEM_KINDS` const → `GemRegistry` from `data/gems/*.ron`; the 3 helpers become
  lookups; spawn draw reads `raws.gems.len()`; `gem_remap` added to `remap_item`
  so `RoughGem`/`CutGem` `stuff` survives reorder/add; **guard the gem spawn with
  `if !raws.gems.is_empty()`** (data-driving turns a const `.len()` into a
  possible `gen_range(0..0)` panic — the critique's crash finding).

### Slice 5 (optional) — `WeaponKind` → `WeaponDef`
Contained to combat (5 methods + the `MELEE_WEAPONS` subset draw).

---

## 4. Open questions (designer's call)
1. **Fort-save mod-mismatch: refuse vs best-effort remap?** (Slice 3.)
2. **Id namespacing** — flat ids + declared-override now, `modid:itemid` later?
   Hard to change post-release.
3. **Determinism: detect-only now, or start the `rng_for(seed, domain, salt)`
   sub-stream migration?** The single biggest long-term fork.
4. **Field-level patching** (DF's `COPY_TAGS_FROM`) vs whole-def replace.
5. **Load-order source of truth** when explicit list, `load_after`, and
   alphabetical disagree — one must be canonical (it's an RNG input).
