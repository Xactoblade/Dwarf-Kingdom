# Economy Layer — Scope (Phase 7 "economy depth")

Design note for the first economy pass in Dwarf Kingdom. Ground truth is
`BLUEPRINT.md`; this scopes the "economy depth" line of Phase 7. Grounded in a
survey of the real code — cited `file:line` references are into
`crates/dk_agents/src/lib.rs` unless another crate is named.

**Chosen scope: Slices 1–3 — a fort with a running economic life, no currency.**
Minted coins and noble economic demands are explicitly deferred (see §6).

---

## 1. Goal & non-goals

Turn today's stateless barter check into a fort whose *created wealth* the world
reacts to, whose trade relationship carries value between caravans, and whose
prices are data-driven so they can be tuned and modded without recompiling. The
north star is DF's own arc — created wealth drives migration and sieges; trade
accrues standing goodwill — at DK's current scale (single fort, one
`trade_partner`, ~22 recipes).

**In scope**

- A true `fortress_wealth()` aggregate (summed value, cached) replacing the item
  **count** proxy at `lib.rs:4870`.
- A persistent `trade_credit` balance so over-payment becomes goodwill, not loss.
- Wealth-reactive world: siege scaling and migrant pull driven by *value*, not count.
- A data-driven price layer: `item_value` per-kind constants (`lib.rs:1773`) and
  `TRADE_MARGIN` (`lib.rs:1757`) move into a RON `EconomyConfig` in `dk_raws`.

**Non-goals (this phase)**

- **No per-dwarf wallets or wages.** The `Dwarf` struct (`lib.rs:1471`) is
  economically empty; dwarves work for food/drink as today. Personal purchasing
  needs an ownership model that is out of scope.
- **No item/room ownership rework.** Gear is rank-derived; the only real personal
  property is `Dwarf.bed`. Rooms stay unowned `Vec<Rect>` zones.
- **No minted coins.** Deferred — see §6. Currency has no demand side until wallets
  or par-priced purchasing exist.
- **No dynamic markets** — no supply/demand curves or per-good scarcity pricing
  beyond one static margin.
- **No multi-civ market, no justice/jail overhaul, no `ItemKind` → data migration**
  (we data-drive *prices*, not item definitions).

---

## 2. Key design decisions

| Fork | Decision | Rationale |
|---|---|---|
| Currency model | **Abstract fort-collective wealth + trade credit. No coins.** | Smallest change that fixes the broken wealth signal and the discard-on-trade gap; coins add cost with no demand side yet (§6). |
| Fortress wealth | **Summed item value, cached** | The count proxy (`lib.rs:4870`) treats a masterwork gold statue as a rock; DF drives everything off created value. |
| Wealth compute | **O(items) sweep on the season boundary, cached** | Wealth-reactive triggers already fire on season boundaries; avoids threading running-total increments through every spawn/consume path (dodges the incremental-cache FPS trap, BLUEPRINT §6.1). |
| Trade continuity | **Persist surplus as `trade_credit`** | Trade currently consumes the whole offered stack and forgets it; banking goodwill makes trade feel continuous. |
| Price data | **RON `EconomyConfig` in `dk_raws`** | CLAUDE.md: new content categories get a RON schema + `data/` files. |

---

## 3. Data model

### `dk_raws` — new RON-backed schema (`data/economy/prices.ron`)

Follow the `PlantDef`/`PlantRegistry` template (`dk_raws/src/lib.rs:77`, `:98`);
register in `Raws` (`:162`) and `Raws::load` (`:169`). `EconomyConfig` is a single
struct, so it loads like `TilesetDef` (single-file, `:148`) — no per-registry
`unknown` fallback needed.

```rust
// crates/dk_raws/src/lib.rs
pub struct EconomyConfig {
    pub trade_margin: f32,           // was TRADE_MARGIN const, lib.rs:1757
    pub kind_prices: Vec<KindPrice>, // replaces item_value match arms, lib.rs:1775-1820
}
pub struct KindPrice {   // one row per material-derived ItemKind
    pub kind: String,    // matches ItemKind variant name
    pub mat_coeff: u32,  // e.g. Craft=12, Weapon=10, Statue=15
    pub flat: u32,       // e.g. Craft=4, Weapon=20, Statue=40
}
```

`MaterialDef.value` (`dk_raws/src/lib.rs:37`) is **unchanged** — it stays the base
multiplier per-kind prices denominate against.

**Load-time validation (do not skip):** assert every priced `ItemKind` has a
`KindPrice` row; fail fast on a missing row rather than silently pricing at 0.

**Known exception:** `RoughGem`/`CutGem` use `gem_value()` (`lib.rs:363`), not
`material.value`, so gems remain code-only for now. Document them as an explicit
special case; "data-driven prices" is partial by design.

### `dk_agents` — new state on `Sim`

```rust
// alongside the caravan/trade fields (~lib.rs:2020-2032)
pub trade_credit: i64,   // banked goodwill with trade_partner; +ve = fort is owed
pub cached_wealth: u32,  // last fortress_wealth(); recomputed on season boundary
```

```rust
// replaces the count at lib.rs:4870 — sum item_value, SKIP contained items
fn fortress_wealth(&self, raws: &Raws) -> u32 {
    self.items.iter().enumerate()
        .filter(|(_, it)| /* active, same predicate as :4870 */)
        .filter(|(_, it)| !matches!(it.state, ItemState::Inside { .. }))
        .map(|(idx, _)| self.item_value_at(idx, raws)) // NOT stack_value
        .sum()
}
```

> **Critical:** sum `item_value`, **not** `stack_value` (`lib.rs:3994`), and skip
> `ItemState::Inside { .. }`. `stack_value` adds `contents_of`, so summing it over
> all active items double-counts every good in a barrel/bin AND is O(items ×
> containers) — a real season-boundary hitch on a mature fort. Test: a fort with a
> barrel of wine has the same wealth whether the wine is loose or packed.

`SimStats` (`lib.rs:1622`): add `value_imported: u64`, `value_exported: u64`
next to `caravans_arrived`/`trades_completed`.

---

## 4. Subsystems touched

| Subsystem | Current state | Change | Risk |
|---|---|---|---|
| Valuation (`item_value` `:1773`) | Hardcoded `match`; no aggregate | Data-drive into `EconomyConfig`; add `fortress_wealth()`; load-time completeness check | **Med** — single valuation chokepoint; guard with a golden test |
| Wealth-reactive world (`:4870`) | Siege scales on item **count** | Re-point to a summed-value band; **re-tune the divisor inside this slice** | **Med** — untuned, a single masterwork instantly caps raiders |
| Migration | Food/drink-guarded pull | Wealth as an **additive** pull (never a floor that starves a poor fort) | **Med** — shapes early-game pacing; must be tested |
| Trade (`execute_trade` `:5935`) | Whole offered stack consumed; no persistence | Bank surplus into `trade_credit`; let credit pay; tally import/export | **Med** — settlement-path change + save bump |
| Data/raws (`dk_raws`, `data/`) | Prices in code | New `EconomyConfig` schema + `data/economy/prices.ron` | **Med** — new load path, well-templated |
| UI (`dk_app`) | No wealth/credit surfaces | Wealth in status panel; credit + import/export in trade HUD | **Med** — half the product (BLUEPRINT §6.5); part of each slice's DoD |

**Save/load reality (not append-only-safe):** `load_sim` hard-rejects any
`version != SAVE_VERSION`; bincode has no schema evolution. **Every
shape-changing slice bumps `SAVE_VERSION` and rejects prior saves by design.**
Forgetting the bump makes bincode read past the header and corrupt the load. New
enum variants (later phases) append at the end only.

---

## 5. Phased plan (least-risk-first; each ships independently)

### Slice 1 — Data-drive prices (zero behavior change)
Move `item_value` constants (`:1775-1820`) and `TRADE_MARGIN` (`:1757`) into
`EconomyConfig` from `data/economy/prices.ron`; `item_value` becomes a table
lookup. Add the load-time completeness check.
- **Exit test:** `cargo test --workspace` green; a golden test asserts
  `item_value` for a masterwork gold craft, a steel weapon, a flat-valued kind
  (e.g. Glass), and a diamond equal the pre-refactor numbers. Edit `prices.ron`,
  rerun, and watch the trade screen quote change with no recompile.

### Slice 2 — True fortress wealth + reactive world
Add `fortress_wealth()` (per the §3 caveat) and `Sim.cached_wealth`, recomputed on
the season boundary. Re-point siege scaling from count to a summed-value band, and
**re-derive the band thresholds against realistic value ranges as part of this
slice** — not a follow-up. Add wealth as an additive migrant pull.
- **Exit test:** `DK_SCREENSHOT` run — a fort holding one masterwork gold statue
  shows higher wealth and draws larger raider waves than a fort of 50 rocks; the
  rock fort still gets migrants (poverty doesn't starve it). Wealth visible in the
  status panel. A migration assertion covers the additive-pull behavior.

### Slice 3 — Trade credit (goodwill carries between visits)
In `execute_trade` (`:5935`): when offered value exceeds `ceil(asked × margin)`,
credit the difference to `trade_credit`; fold `trade_credit` into the accept
condition so a later trade can draw it down; clamp so it can't go negative into
fort-owes-caravan debt. Tally `value_imported`/`value_exported`. **Count
`trade_credit` toward `fortress_wealth`** so it can't be used to launder
masterworks and hide wealth from raiders.
- **Exit test:** over-pay a caravan by a known margin; confirm `trade_credit` rises
  by exactly the surplus. Next visit, buy a good priced ≤ credit and confirm it
  clears drawing on the balance. Import/export totals show in the trade HUD, and
  fort wealth includes the banked credit.

---

## 6. Status of the two deferred items

- **Noble economic demands — DONE (Part 2a, `86267a4`).** Implemented as a
  DF-faithful **export ban**: a baron with a trade partner forbids selling a
  material to caravans; defying it is punished, honouring it pleases him. Not the
  "sell N value" quota (wrong) nor `PayTribute`-to-parent-civ (not DF-faithful).

- **Minted coins — DROPPED (not deferred).** Decided against, on the same grounds
  that killed Dwarf Fortress's own economy: **DF built a circulating coin economy
  (wages, rent, taxes, purchases) and then *disabled* it** — it death-spiralled
  (dwarves going broke and refusing to work). Modern DF coins are a mintable trade
  good only, not currency. For DK, coins with no demand side are exactly that trap;
  the only viable version (coins buy caravan goods at par, with a value-conserving
  mint) adds `ItemKind` ripple + an `execute_trade` rework for a thin payoff. So we
  do **not** build coins — the economy layer is complete without them.

**The economy layer is done:** data-driven prices (Slice 1), created-wealth
driving sieges/migration (Slice 2), trade credit (Slice 3), and noble export bans
(Part 2a). If per-dwarf ownership/wallets ever land in a later phase, revisit
coins then — with a real in-fort marketplace to give them a sink.

## 7. Open questions

1. **Siege band shape.** Current `n = (1 + wealth/150).min(5)` runs on a count; on
   summed value a lone masterwork statue (~1700) caps raiders instantly. What
   value thresholds map to which raider counts? (Resolve inside Slice 2.)
2. **`trade_credit` keying.** Global is fine with one `trade_partner` today; keep
   the field `i64` now and key per-civ only if multi-civ ever lands.
3. **Embargo interaction.** If a trader is killed (`trade_ban_until` `:2029`),
   is banked credit forfeit, frozen, or preserved? DF-flavored answer: frozen
   until relations resume.
