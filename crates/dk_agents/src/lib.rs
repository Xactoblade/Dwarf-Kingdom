//! The living simulation: dwarves, needs, designations, jobs, hauling.
//!
//! Engine-agnostic and fully deterministic — `Sim::step()` advances one fixed
//! tick, so the whole game loop can run (and be tested) headlessly.

use anyhow::{Context, Result};
use dk_core::Calendar;
use dk_raws::{MaterialCategory, MaterialRegistry};
use dk_world::path::{self, Pos, Regions};
use dk_world::{Map, Tile, TileShape};
use rand::Rng;
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path as FsPath;

pub mod names;

/// Ticks of digging to mine out one tile.
pub const MINE_WORK: u16 = 40;
/// Ticks between two steps of a walking dwarf.
pub const WALK_COOLDOWN: u8 = 3;
/// How often (in ticks) idle dwarves look for work.
pub const ASSIGN_INTERVAL: u64 = 5;
/// Pathfinding safety valve.
pub const MAX_ASTAR_NODES: usize = 50_000;
/// Ticks before an unreachable designation is reconsidered.
pub const RETRY_DELAY: u64 = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DesignationKind {
    /// Dig the tile out into a floor.
    Mine,
    /// Carve stairs (into solid rock, or down from an existing floor).
    Stairs,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Designation {
    pub kind: DesignationKind,
    pub assigned: bool,
    pub retry_at: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ItemState {
    OnGround,
    Carried { by: usize },
    Stored { stockpile: usize },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Item {
    pub material: u16,
    pub pos: Pos,
    pub state: ItemState,
    /// Dwarf index that has claimed this item for hauling.
    pub reserved_by: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stockpile {
    pub z: i32,
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl Stockpile {
    pub fn contains(&self, p: Pos) -> bool {
        p.z == self.z && p.x >= self.x0 && p.x <= self.x1 && p.y >= self.y0 && p.y <= self.y1
    }

    pub fn cells(&self) -> impl Iterator<Item = Pos> + '_ {
        let (x0, x1, y0, y1, z) = (self.x0, self.x1, self.y0, self.y1, self.z);
        (y0..=y1).flat_map(move |y| (x0..=x1).map(move |x| Pos::new(x, y, z)))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Task {
    Idle { wander_cd: u16 },
    Sleep { remaining: u16 },
    Mine { target: Pos, path: Vec<Pos>, progress: u16 },
    Haul { item: usize, dest: Pos, path: Vec<Pos>, carrying: bool },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dwarf {
    pub name: String,
    pub pos: Pos,
    pub hunger: f32,
    pub thirst: f32,
    pub fatigue: f32,
    pub task: Task,
    move_cd: u8,
}

impl Dwarf {
    pub fn is_idle(&self) -> bool {
        matches!(self.task, Task::Idle { .. })
    }

    pub fn task_name(&self) -> &'static str {
        match self.task {
            Task::Idle { .. } => "idle",
            Task::Sleep { .. } => "sleeping",
            Task::Mine { .. } => "mining",
            Task::Haul { .. } => "hauling",
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct Sim {
    pub map: Map,
    pub dwarves: Vec<Dwarf>,
    pub items: Vec<Item>,
    pub stockpiles: Vec<Stockpile>,
    pub designations: BTreeMap<Pos, Designation>,
    pub clock: Calendar,
    rng: ChaCha8Rng,
    /// Set whenever terrain changes; the renderer reads and clears it.
    #[serde(skip)]
    pub map_changed: bool,
    #[serde(skip, default)]
    regions: Regions,
}

impl Sim {
    pub fn new(map: Map, reg: &MaterialRegistry, mut rng: ChaCha8Rng, dwarf_count: usize) -> Self {
        let _ = reg;
        let cx = map.width as i32 / 2;
        let cy = map.height as i32 / 2;
        let regions = Regions::new(&map);

        // Spawn dwarves on walkable ground near the map center.
        let mut dwarves = Vec::new();
        'spawn: for radius in 0..(map.width as i32 / 2) {
            for dy in -radius..=radius {
                for dx in -radius..=radius {
                    if dwarves.len() >= dwarf_count {
                        break 'spawn;
                    }
                    let (x, y) = (cx + dx, cy + dy);
                    if x < 0 || y < 0 || x >= map.width as i32 || y >= map.height as i32 {
                        continue;
                    }
                    if let Some(z) = map.walk_surface_z(x as usize, y as usize) {
                        let pos = Pos::new(x, y, z as i32);
                        if dwarves.iter().any(|d: &Dwarf| d.pos == pos) {
                            continue;
                        }
                        dwarves.push(Dwarf {
                            name: names::dwarf_name(&mut rng),
                            pos,
                            hunger: rng.gen_range(0.0..20.0),
                            thirst: rng.gen_range(0.0..20.0),
                            fatigue: rng.gen_range(0.0..30.0),
                            task: Task::Idle { wander_cd: rng.gen_range(20..80) },
                            move_cd: 0,
                        });
                    }
                }
            }
        }
        assert!(!dwarves.is_empty(), "no walkable spawn tiles found");

        Sim {
            map,
            dwarves,
            items: Vec::new(),
            stockpiles: Vec::new(),
            designations: BTreeMap::new(),
            clock: Calendar::default(),
            rng,
            map_changed: true,
            regions,
        }
    }

    /// Rebuild caches after deserialization.
    pub fn rebuild_caches(&mut self) {
        self.regions = Regions::new(&self.map);
        self.map_changed = true;
    }

    // ------------------------------------------------------------- commands

    /// Designate a rectangle at one z-level. Only tiles that can actually be
    /// worked (solid, or walkable-for-stairs) are accepted.
    pub fn designate_rect(&mut self, kind: DesignationKind, a: Pos, b: Pos) -> usize {
        assert_eq!(a.z, b.z, "designations are per z-level");
        let mut added = 0;
        for y in a.y.min(b.y)..=a.y.max(b.y) {
            for x in a.x.min(b.x)..=a.x.max(b.x) {
                let p = Pos::new(x, y, a.z);
                let Some(tile) = self.map.tile_at(p) else { continue };
                let workable = match kind {
                    DesignationKind::Mine => tile.is_solid(),
                    DesignationKind::Stairs => {
                        tile.is_solid()
                            || matches!(tile.shape, TileShape::Floor | TileShape::Ramp)
                    }
                };
                if workable && !self.designations.contains_key(&p) {
                    self.designations.insert(
                        p,
                        Designation { kind, assigned: false, retry_at: 0 },
                    );
                    added += 1;
                }
            }
        }
        added
    }

    /// Remove designations in a rectangle (unassigns any dwarf working them).
    pub fn cancel_rect(&mut self, a: Pos, b: Pos) -> usize {
        assert_eq!(a.z, b.z);
        let mut removed = 0;
        for y in a.y.min(b.y)..=a.y.max(b.y) {
            for x in a.x.min(b.x)..=a.x.max(b.x) {
                let p = Pos::new(x, y, a.z);
                if self.designations.remove(&p).is_some() {
                    removed += 1;
                    for d in &mut self.dwarves {
                        if matches!(d.task, Task::Mine { target, .. } if target == p) {
                            d.task = Task::Idle { wander_cd: 10 };
                        }
                    }
                }
            }
        }
        removed
    }

    pub fn add_stockpile(&mut self, a: Pos, b: Pos) {
        assert_eq!(a.z, b.z);
        self.stockpiles.push(Stockpile {
            z: a.z,
            x0: a.x.min(b.x),
            y0: a.y.min(b.y),
            x1: a.x.max(b.x),
            y1: a.y.max(b.y),
        });
    }

    pub fn stockpile_at(&self, p: Pos) -> Option<usize> {
        self.stockpiles.iter().position(|s| s.contains(p))
    }

    /// Demo/test helper: tile flat 3x3 stockpile patches near (cx, cy),
    /// closest first, until combined capacity reaches `target_cells`.
    /// Returns the number of cells placed.
    pub fn place_flat_stockpiles(&mut self, cx: i32, cy: i32, target_cells: usize) -> usize {
        let mut candidates: Vec<(u32, i32, i32, i32)> = Vec::new();
        for y in 0..self.map.height.saturating_sub(2) {
            for x in 0..self.map.width.saturating_sub(2) {
                let Some(z) = self.map.walk_surface_z(x, y) else { continue };
                let uniform = (0..3).all(|dy| {
                    (0..3).all(|dx| self.map.walk_surface_z(x + dx, y + dy) == Some(z))
                });
                if uniform {
                    let d = (x as i32 - cx).unsigned_abs() + (y as i32 - cy).unsigned_abs();
                    candidates.push((d, x as i32, y as i32, z as i32));
                }
            }
        }
        candidates.sort();
        let mut cells = 0;
        for (_, x, y, z) in candidates {
            if cells >= target_cells {
                break;
            }
            let (x1, y1) = (x + 2, y + 2);
            let overlaps = self
                .stockpiles
                .iter()
                .any(|s| x <= s.x1 && s.x0 <= x1 && y <= s.y1 && s.y0 <= y1);
            if !overlaps {
                self.add_stockpile(Pos::new(x, y, z), Pos::new(x1, y1, z));
                cells += 9;
            }
        }
        cells
    }

    // ------------------------------------------------------------- queries

    /// A stockpile cell is free if it's walkable and no item occupies or is
    /// heading to it.
    fn cell_free(&self, cell: Pos) -> bool {
        if !self.map.walkable(cell) {
            return false;
        }
        if self
            .items
            .iter()
            .any(|it| it.pos == cell && matches!(it.state, ItemState::Stored { .. } | ItemState::OnGround))
        {
            return false;
        }
        !self.dwarves.iter().any(
            |d| matches!(d.task, Task::Haul { dest, .. } if dest == cell),
        )
    }

    fn find_free_cell(&self, near: Pos, from_region: u32) -> Option<Pos> {
        self.stockpiles
            .iter()
            .flat_map(|s| s.cells())
            .filter(|&c| self.regions.id(c) == from_region && self.cell_free(c))
            .min_by_key(|&c| c.manhattan(near))
    }

    pub fn pending_designations(&self) -> usize {
        self.designations.len()
    }

    pub fn stored_items(&self) -> usize {
        self.items
            .iter()
            .filter(|i| matches!(i.state, ItemState::Stored { .. }))
            .count()
    }

    // ------------------------------------------------------------- stepping

    pub fn step(&mut self, reg: &MaterialRegistry) {
        self.clock.advance();
        if self.regions.dirty {
            self.regions.rebuild(&self.map);
        }
        if self.clock.tick % ASSIGN_INTERVAL == 0 {
            self.assign_jobs();
        }
        for i in 0..self.dwarves.len() {
            self.update_dwarf(i, reg);
        }
    }

    fn assign_jobs(&mut self) {
        for i in 0..self.dwarves.len() {
            if self.dwarves[i].is_idle() {
                self.assign_one(i);
            }
        }
    }

    fn assign_one(&mut self, i: usize) {
        let dwarf_pos = self.dwarves[i].pos;
        let my_region = self.regions.id(dwarf_pos);
        if my_region == 0 {
            return; // stranded somewhere unwalkable; shouldn't happen
        }
        let tick = self.clock.tick;
        let mut scratch = Vec::with_capacity(6);

        // Best mining job: nearest designation with a reachable work spot.
        let mut best_mine: Option<(u32, Pos, Pos)> = None; // (dist, target, work_pos)
        for (&target, des) in &self.designations {
            if des.assigned || des.retry_at > tick {
                continue;
            }
            path::work_positions(&self.map, target, &mut scratch);
            let reachable = scratch
                .iter()
                .filter(|&&w| self.regions.id(w) == my_region)
                .min_by_key(|&&w| w.manhattan(dwarf_pos));
            if let Some(&work) = reachable {
                let dist = work.manhattan(dwarf_pos);
                if best_mine.is_none_or(|(bd, _, _)| dist < bd) {
                    best_mine = Some((dist, target, work));
                }
            }
        }

        // Best hauling job: nearest unreserved ground item with a free,
        // reachable stockpile cell.
        let mut best_haul: Option<(u32, usize, Pos)> = None; // (dist, item, dest)
        if !self.stockpiles.is_empty() {
            for (idx, item) in self.items.iter().enumerate() {
                // Anything loose on the ground wants storing — including
                // items dropped inside a stockpile when their cell was taken.
                if item.state != ItemState::OnGround || item.reserved_by.is_some() {
                    continue;
                }
                if self.regions.id(item.pos) != my_region {
                    continue;
                }
                let Some(dest) = self.find_free_cell(item.pos, my_region) else {
                    continue;
                };
                let dist = item.pos.manhattan(dwarf_pos);
                if best_haul.is_none_or(|(bd, _, _)| dist < bd) {
                    best_haul = Some((dist, idx, dest));
                }
            }
        }

        // Mining wins ties; otherwise take the closer job.
        let take_mine = match (best_mine, best_haul) {
            (Some((md, _, _)), Some((hd, _, _))) => md <= hd,
            (Some(_), None) => true,
            _ => false,
        };

        if take_mine {
            let (_, target, work) = best_mine.unwrap();
            match path::astar(&self.map, dwarf_pos, work, MAX_ASTAR_NODES) {
                Some(p) => {
                    self.designations.get_mut(&target).unwrap().assigned = true;
                    self.dwarves[i].task = Task::Mine { target, path: p, progress: 0 };
                }
                None => {
                    // Region said reachable but A* gave up — back off.
                    self.designations.get_mut(&target).unwrap().retry_at = tick + RETRY_DELAY;
                }
            }
        } else if let Some((_, item_idx, dest)) = best_haul {
            let item_pos = self.items[item_idx].pos;
            match path::astar(&self.map, dwarf_pos, item_pos, MAX_ASTAR_NODES) {
                Some(p) => {
                    self.items[item_idx].reserved_by = Some(i);
                    self.dwarves[i].task =
                        Task::Haul { item: item_idx, dest, path: p, carrying: false };
                }
                None => {}
            }
        }
    }

    fn update_dwarf(&mut self, i: usize, reg: &MaterialRegistry) {
        // Needs tick slowly upward; consequences arrive in Phase 2.
        {
            let d = &mut self.dwarves[i];
            d.hunger = (d.hunger + 0.002).min(100.0);
            d.thirst = (d.thirst + 0.003).min(100.0);
            d.fatigue = (d.fatigue + 0.0015).min(100.0);
        }

        let task = self.dwarves[i].task.clone();
        match task {
            Task::Idle { wander_cd } => {
                // Exhausted dwarves nap where they stand.
                if self.dwarves[i].fatigue >= 100.0 {
                    self.dwarves[i].task = Task::Sleep { remaining: 1200 };
                    return;
                }
                if wander_cd > 0 {
                    self.dwarves[i].task = Task::Idle { wander_cd: wander_cd - 1 };
                } else {
                    let pos = self.dwarves[i].pos;
                    let mut opts = Vec::with_capacity(6);
                    path::neighbors(&self.map, pos, &mut opts);
                    if !opts.is_empty() {
                        let n = opts[self.rng.gen_range(0..opts.len())];
                        self.dwarves[i].pos = n;
                        self.carry_item_along(i);
                    }
                    let cd = self.rng.gen_range(40..160);
                    self.dwarves[i].task = Task::Idle { wander_cd: cd };
                }
            }
            Task::Sleep { remaining } => {
                if remaining == 0 {
                    self.dwarves[i].fatigue = 0.0;
                    self.dwarves[i].task = Task::Idle { wander_cd: 10 };
                } else {
                    self.dwarves[i].task = Task::Sleep { remaining: remaining - 1 };
                }
            }
            Task::Mine { target, mut path, progress } => {
                // Designation cancelled underneath us?
                if !self.designations.contains_key(&target) {
                    self.dwarves[i].task = Task::Idle { wander_cd: 5 };
                    return;
                }
                if !path.is_empty() {
                    if self.step_along(i, &mut path) {
                        self.dwarves[i].task = Task::Mine { target, path, progress };
                    } else {
                        self.abandon_mine(i, target);
                    }
                    return;
                }
                // At the work spot: dig.
                let progress = progress + 1;
                if progress < MINE_WORK {
                    self.dwarves[i].task = Task::Mine { target, path, progress };
                    return;
                }
                self.complete_mine(i, target, reg);
            }
            Task::Haul { item, dest, mut path, carrying } => {
                if !carrying {
                    // Walking to the item.
                    if !path.is_empty() {
                        if self.step_along(i, &mut path) {
                            self.dwarves[i].task = Task::Haul { item, dest, path, carrying };
                        } else {
                            self.abandon_haul(i, item);
                        }
                        return;
                    }
                    // Pick up (item may have been grabbed/moved meanwhile).
                    let here = self.dwarves[i].pos;
                    if self.items[item].pos != here
                        || self.items[item].state != ItemState::OnGround
                    {
                        self.abandon_haul(i, item);
                        return;
                    }
                    self.items[item].state = ItemState::Carried { by: i };
                    match path::astar(&self.map, here, dest, MAX_ASTAR_NODES) {
                        Some(p) => {
                            self.dwarves[i].task =
                                Task::Haul { item, dest, path: p, carrying: true };
                        }
                        None => {
                            self.drop_carried(i, item);
                            self.abandon_haul(i, item);
                        }
                    }
                } else {
                    if !path.is_empty() {
                        if self.step_along(i, &mut path) {
                            self.dwarves[i].task = Task::Haul { item, dest, path, carrying };
                        } else {
                            self.drop_carried(i, item);
                            self.abandon_haul(i, item);
                        }
                        return;
                    }
                    // Arrived: store (cell may have been taken; then just drop —
                    // a new haul job will pick it up for another cell).
                    let here = self.dwarves[i].pos;
                    let taken = self
                        .items
                        .iter()
                        .enumerate()
                        .any(|(j, it)| j != item && it.pos == here && it.state != ItemState::Carried { by: i });
                    self.items[item].pos = here;
                    self.items[item].reserved_by = None;
                    self.items[item].state = if !taken {
                        match self.stockpile_at(here) {
                            Some(s) => ItemState::Stored { stockpile: s },
                            None => ItemState::OnGround,
                        }
                    } else {
                        ItemState::OnGround
                    };
                    self.dwarves[i].task = Task::Idle { wander_cd: 5 };
                }
            }
        }
    }

    /// Move one step along `path` (respecting walk cooldown). Returns false
    /// if the next tile stopped being walkable (terrain changed).
    fn step_along(&mut self, i: usize, path: &mut Vec<Pos>) -> bool {
        if self.dwarves[i].move_cd > 0 {
            self.dwarves[i].move_cd -= 1;
            return true;
        }
        let next = path[0];
        if !self.map.walkable(next) {
            return false;
        }
        path.remove(0);
        self.dwarves[i].pos = next;
        self.dwarves[i].move_cd = WALK_COOLDOWN;
        self.carry_item_along(i);
        true
    }

    fn carry_item_along(&mut self, i: usize) {
        let pos = self.dwarves[i].pos;
        for item in &mut self.items {
            if item.state == (ItemState::Carried { by: i }) {
                item.pos = pos;
            }
        }
    }

    fn complete_mine(&mut self, i: usize, target: Pos, reg: &MaterialRegistry) {
        let Some(des) = self.designations.remove(&target) else {
            self.dwarves[i].task = Task::Idle { wander_cd: 5 };
            return;
        };
        let tile = self.map.tile_at(target).expect("designated tile in bounds");
        let new_shape = match des.kind {
            DesignationKind::Mine => TileShape::Floor,
            DesignationKind::Stairs => TileShape::Stairs,
        };
        self.map.set_at(target, Tile { material: tile.material, shape: new_shape });
        self.regions.dirty = true;
        self.map_changed = true;

        // Stone-bearing tiles drop a boulder; soil digs away clean.
        if tile.is_solid() && reg.get(tile.material).category != MaterialCategory::Soil {
            self.items.push(Item {
                material: tile.material,
                pos: target,
                state: ItemState::OnGround,
                reserved_by: None,
            });
        }
        self.dwarves[i].task = Task::Idle { wander_cd: 2 };
    }

    fn abandon_mine(&mut self, i: usize, target: Pos) {
        if let Some(des) = self.designations.get_mut(&target) {
            des.assigned = false;
            des.retry_at = self.clock.tick + RETRY_DELAY;
        }
        self.dwarves[i].task = Task::Idle { wander_cd: 5 };
    }

    fn abandon_haul(&mut self, i: usize, item: usize) {
        if self.items[item].reserved_by == Some(i) {
            self.items[item].reserved_by = None;
        }
        self.dwarves[i].task = Task::Idle { wander_cd: 5 };
    }

    fn drop_carried(&mut self, i: usize, item: usize) {
        if self.items[item].state == (ItemState::Carried { by: i }) {
            self.items[item].pos = self.dwarves[i].pos;
            self.items[item].state = ItemState::OnGround;
        }
    }
}

// ------------------------------------------------------------------- saves

const SAVE_MAGIC: u32 = 0x444B_5331; // "DKS1"
const SAVE_VERSION: u32 = 2;

#[derive(Serialize)]
struct SaveOut<'a> {
    magic: u32,
    version: u32,
    material_ids: Vec<String>,
    sim: &'a Sim,
}

#[derive(Deserialize)]
struct SaveIn {
    magic: u32,
    version: u32,
    material_ids: Vec<String>,
    sim: Sim,
}

pub fn save_sim(sim: &Sim, path: &FsPath, reg: &MaterialRegistry) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let out = SaveOut {
        magic: SAVE_MAGIC,
        version: SAVE_VERSION,
        material_ids: reg.id_manifest(),
        sim,
    };
    let file = std::fs::File::create(path)
        .with_context(|| format!("creating {}", path.display()))?;
    bincode::serialize_into(std::io::BufWriter::new(file), &out)?;
    Ok(())
}

pub fn load_sim(path: &FsPath, reg: &MaterialRegistry) -> Result<Sim> {
    let file = std::fs::File::open(path)
        .with_context(|| format!("opening {}", path.display()))?;
    let save: SaveIn = bincode::deserialize_from(std::io::BufReader::new(file))
        .with_context(|| format!("deserializing {}", path.display()))?;
    anyhow::ensure!(save.magic == SAVE_MAGIC, "not a Dwarf Kingdom save file");
    anyhow::ensure!(
        save.version == SAVE_VERSION,
        "save version {} unsupported (expected {})",
        save.version,
        SAVE_VERSION
    );
    let mut sim = save.sim;
    sim.map.validate()?;
    sim.map.remap_materials(&save.material_ids, reg)?;
    let remap = dk_world::build_remap(&save.material_ids, reg)?;
    for item in &mut sim.items {
        let old = item.material as usize;
        anyhow::ensure!(old < remap.len(), "corrupt save: item material {old} out of range");
        item.material = remap[old];
    }
    sim.rebuild_caches();
    Ok(sim)
}
