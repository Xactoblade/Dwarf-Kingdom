//! Physical simulation systems. Phase 3: water as a cellular automaton.
//!
//! Water lives on tiles as a depth 0-7 (`Tile::water`). The automaton keeps
//! an *active set* — only tiles that changed recently are stepped, so settled
//! lakes cost nothing (BLUEPRINT.md hard-part #2).

use dk_world::path::Pos;
use dk_world::{Map, MAX_WATER};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, VecDeque};

/// Deterministic, fixed neighbor order for the automaton.
const FLOW_ORDER: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct WaterSim {
    /// Tiles that refill to full depth every step (natural springs).
    pub springs: BTreeSet<Pos>,
    /// Tiles that might change next step. Rebuilt on load via `wake_all`.
    #[serde(skip)]
    active: BTreeSet<Pos>,
}

impl WaterSim {
    /// Mark a tile (and its flow neighbors) as needing simulation — call
    /// after any terrain edit that could let water move.
    pub fn wake(&mut self, p: Pos) {
        self.active.insert(p);
        for (dx, dy) in FLOW_ORDER {
            self.active.insert(Pos::new(p.x + dx, p.y + dy, p.z));
        }
        self.active.insert(Pos::new(p.x, p.y, p.z + 1));
        self.active.insert(Pos::new(p.x, p.y, p.z - 1));
    }

    /// Scan the whole map for water and springs (used after deserialization).
    pub fn wake_all(&mut self, map: &Map) {
        self.active.clear();
        for z in 0..map.depth as i32 {
            for y in 0..map.height as i32 {
                for x in 0..map.width as i32 {
                    let p = Pos::new(x, y, z);
                    if map.water_at(p) > 0 {
                        self.wake(p);
                    }
                }
            }
        }
        for &s in self.springs.clone().iter() {
            self.wake(s);
        }
    }

    /// Advance the automaton one step. Returns true if any tile's depth
    /// crossed the deep-water threshold (callers then mark regions dirty).
    pub fn step(&mut self, map: &mut Map) -> bool {
        let mut crossed_threshold = false;
        let current: Vec<Pos> = self.active.iter().copied().collect();
        self.active.clear();

        // Springs first: they push water into the system.
        let springs: Vec<Pos> = self.springs.iter().copied().collect();
        for s in springs {
            let Some(tile) = map.tile_at(s) else { continue };
            if tile.holds_water() && tile.water < MAX_WATER {
                map.set_water(s, MAX_WATER);
                self.wake(s);
                crossed_threshold |= crossed(tile.water, MAX_WATER);
            }
        }

        for &p in &current {
            let Some(tile) = map.tile_at(p) else { continue };
            if !tile.holds_water() {
                continue;
            }
            let mut w = tile.water;
            if w == 0 {
                continue;
            }

            // 1. Fall: everything possible goes straight down.
            let below = Pos::new(p.x, p.y, p.z - 1);
            if let Some(bt) = map.tile_at(below) {
                if bt.holds_water() && bt.water < MAX_WATER {
                    let space = MAX_WATER - bt.water;
                    let moved = w.min(space);
                    let new_below = bt.water + moved;
                    map.set_water(below, new_below);
                    w -= moved;
                    map.set_water(p, w);
                    crossed_threshold |= crossed(bt.water, new_below);
                    crossed_threshold |= crossed(tile.water, w);
                    self.wake(below);
                    self.wake(p);
                    if w == 0 {
                        continue;
                    }
                }
            }

            // 2. Spread: equalize with lower orthogonal neighbors, one unit
            // per neighbor per step, in fixed order for determinism.
            for (dx, dy) in FLOW_ORDER {
                if w <= 1 {
                    break;
                }
                let q = Pos::new(p.x + dx, p.y + dy, p.z);
                let Some(qt) = map.tile_at(q) else { continue };
                if !qt.holds_water() || qt.water + 1 >= w {
                    continue;
                }
                let new_q = qt.water + 1;
                map.set_water(q, new_q);
                w -= 1;
                map.set_water(p, w);
                crossed_threshold |= crossed(qt.water, new_q);
                crossed_threshold |= crossed(w + 1, w);
                self.wake(q);
                self.wake(p);
            }
        }

        // 3. Pressure approximation: level each active connected body.
        // Spread alone stalls in a stable one-per-tile gradient, so a flood
        // could never reach drowning depth away from its source. Seed from
        // the step-start snapshot as well: leveling must keep advancing the
        // waterline even on steps where fall/spread had nothing to do.
        crossed_threshold |= self.level_bodies(map, &current);
        crossed_threshold
    }

    /// Redistribute each connected body of water at one z-level (including
    /// its dry shoreline) so depths differ by at most one across the body.
    /// Bodies are discovered from the wake sets, so settled water stays free.
    fn level_bodies(&mut self, map: &mut Map, carried_over: &[Pos]) -> bool {
        if self.active.is_empty() && carried_over.is_empty() {
            return false;
        }
        let mut changed = false;
        let seeds: Vec<Pos> = carried_over
            .iter()
            .chain(self.active.iter())
            .copied()
            .collect::<BTreeSet<Pos>>()
            .into_iter()
            .filter(|&p| map.water_at(p) > 0)
            .collect();
        let mut visited: BTreeSet<Pos> = BTreeSet::new();
        for seed in seeds {
            if visited.contains(&seed) {
                continue;
            }
            visited.insert(seed);
            let mut body: Vec<Pos> = Vec::new();
            let mut queue = VecDeque::from([seed]);
            while let Some(p) = queue.pop_front() {
                let Some(t) = map.tile_at(p) else { continue };
                if !t.holds_water() {
                    continue;
                }
                // A dry tile that would cascade water further down belongs to
                // the fall pass, not the standing body.
                let below = Pos::new(p.x, p.y, p.z - 1);
                let below_absorbs = map
                    .tile_at(below)
                    .is_some_and(|b| b.holds_water() && b.water < MAX_WATER);
                if below_absorbs && t.water == 0 {
                    continue;
                }
                body.push(p);
                if body.len() >= 4096 {
                    break; // safety valve for oceans
                }
                for (dx, dy) in FLOW_ORDER {
                    let q = Pos::new(p.x + dx, p.y + dy, p.z);
                    if visited.contains(&q) {
                        continue;
                    }
                    let Some(qt) = map.tile_at(q) else { continue };
                    // Expand through water, or from water onto dry shoreline
                    // (never dry-to-dry: that would flood-fill the world).
                    if qt.holds_water() && (qt.water > 0 || t.water > 0) {
                        visited.insert(q);
                        queue.push_back(q);
                    }
                }
            }
            if body.len() < 2 {
                continue;
            }
            let total: u32 = body.iter().map(|&p| map.water_at(p) as u32).sum();
            if total == 0 {
                continue;
            }
            let base = (total / body.len() as u32).min(MAX_WATER as u32) as u8;
            if base >= MAX_WATER {
                continue; // saturated; nothing to level
            }
            let mut rem = (total as usize).saturating_sub(base as usize * body.len());
            body.sort();
            for &p in &body {
                let target = if rem > 0 && base < MAX_WATER {
                    rem -= 1;
                    base + 1
                } else {
                    base
                };
                let cur = map.water_at(p);
                if cur != target {
                    map.set_water(p, target);
                    self.wake(p);
                    changed |= crossed(cur, target);
                }
            }
        }
        changed
    }
}

/// Did a depth change cross the pathability threshold?
fn crossed(before: u8, after: u8) -> bool {
    (before >= dk_world::DEEP_WATER) != (after >= dk_world::DEEP_WATER)
}

/// Total water on the map — used by conservation tests.
pub fn total_water(map: &Map) -> u64 {
    let mut sum = 0u64;
    for z in 0..map.depth as i32 {
        for y in 0..map.height as i32 {
            for x in 0..map.width as i32 {
                sum += map.water_at(Pos::new(x, y, z)) as u64;
            }
        }
    }
    sum
}

#[cfg(test)]
mod tests {
    use super::*;
    use dk_world::{Tile, TileShape};

    /// 8x8x4: solid floor at z0, walkable floor at z1, open above.
    fn basin() -> Map {
        let mut m = Map::new_air(8, 8, 4, 0);
        for y in 0..8 {
            for x in 0..8 {
                m.set(x, y, 0, Tile::solid(0));
                m.set(x, y, 1, Tile::floor(0));
            }
        }
        m
    }

    fn run(sim: &mut WaterSim, map: &mut Map, steps: usize) {
        for _ in 0..steps {
            sim.step(map);
        }
    }

    #[test]
    fn water_is_conserved_without_springs() {
        let mut m = basin();
        m.set_water(Pos::new(4, 4, 1), 7);
        m.set_water(Pos::new(4, 4, 2), 7); // a column poured from above
        let before = total_water(&m);
        let mut sim = WaterSim::default();
        sim.wake_all(&m);
        run(&mut sim, &mut m, 200);
        assert_eq!(total_water(&m), before, "no water may vanish or appear");
    }

    #[test]
    fn water_settles_and_spreads() {
        let mut m = basin();
        m.set_water(Pos::new(4, 4, 1), 7);
        let mut sim = WaterSim::default();
        sim.wake_all(&m);
        run(&mut sim, &mut m, 300);
        // 7 units over a flat basin: nothing deeper than 2 after settling.
        for y in 0..8 {
            for x in 0..8 {
                assert!(m.water_at(Pos::new(x, y, 1)) <= 2);
            }
        }
        assert_eq!(total_water(&m), 7);
    }

    #[test]
    fn gate_blocks_flow_until_opened() {
        // Two chambers split by a wall with a gate at (4,4,1).
        let mut m = basin();
        for y in 0..8 {
            m.set(4, y, 1, Tile::solid(0));
        }
        m.set(4, 4, 1, Tile { material: 0, shape: TileShape::Gate, water: 0 });
        // Flood the west chamber.
        for y in 0..8 {
            for x in 0..4 {
                m.set_water(Pos::new(x, y, 1), 6);
            }
        }
        let mut sim = WaterSim::default();
        sim.wake_all(&m);
        run(&mut sim, &mut m, 200);
        assert_eq!(m.water_at(Pos::new(6, 4, 1)), 0, "gate must hold water back");

        // Open the gate.
        m.set(4, 4, 1, Tile::floor(0));
        sim.wake(Pos::new(4, 4, 1));
        run(&mut sim, &mut m, 800);
        assert!(m.water_at(Pos::new(6, 4, 1)) > 0, "water must flow once open");
    }

    #[test]
    fn springs_refill() {
        let mut m = basin();
        let mut sim = WaterSim::default();
        sim.springs.insert(Pos::new(2, 2, 1));
        sim.wake_all(&m);
        run(&mut sim, &mut m, 400);
        assert!(total_water(&m) > 20, "spring should keep feeding the pool");
    }

    #[test]
    fn settled_water_goes_to_sleep() {
        let mut m = basin();
        m.set_water(Pos::new(4, 4, 1), 4);
        let mut sim = WaterSim::default();
        sim.wake_all(&m);
        run(&mut sim, &mut m, 300);
        // After settling, the active set drains to nothing (springs aside).
        let mut quiet = false;
        for _ in 0..50 {
            if !sim.step(&mut m) && sim.active.is_empty() {
                quiet = true;
                break;
            }
        }
        assert!(quiet, "a settled pool must stop consuming simulation time");
    }
}

#[cfg(test)]
mod leveling_tests {
    use super::*;
    use dk_world::Tile;

    /// A 1-wide channel: tank of full water at one end, long dry run.
    #[test]
    fn connected_body_levels_out() {
        let mut m = Map::new_air(20, 1, 3, 0);
        for x in 0..20 {
            m.set(x, 0, 0, Tile::solid(0));
            m.set(x, 0, 1, Tile::floor(0));
        }
        // 5 tiles of full water at the west end.
        for x in 0..5 {
            m.set_water(Pos::new(x, 0, 1), 7);
        }
        let mut sim = WaterSim::default();
        sim.wake_all(&m);
        for _ in 0..500 {
            sim.step(&mut m);
        }
        // 35 units over 20 tiles = 1.75 -> every tile 1 or 2, none at 3+.
        let depths: Vec<u8> = (0..20).map(|x| m.water_at(Pos::new(x, 0, 1))).collect();
        assert_eq!(total_water(&m), 35);
        assert!(
            depths.iter().all(|&d| (1..=2).contains(&d)),
            "gradient should level out, got {depths:?}"
        );
    }
}
