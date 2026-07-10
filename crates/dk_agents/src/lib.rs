//! The living simulation: dwarves, needs, designations, jobs, farming,
//! workshops, hauling, happiness.
//!
//! Engine-agnostic and fully deterministic — `Sim::step()` advances one fixed
//! tick, so the whole game loop can run (and be tested) headlessly.

use anyhow::{Context, Result};
use dk_core::{Calendar, DAYS_PER_SEASON, TICKS_PER_DAY};
use dk_raws::{MaterialCategory, Raws};
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
/// Ticks of work to harvest a grown crop.
pub const HARVEST_WORK: u16 = 60;
/// Ticks of work at a workshop to brew/cook.
pub const CRAFT_WORK: u16 = 150;
/// Ticks between two steps of a walking dwarf.
pub const WALK_COOLDOWN: u8 = 3;
/// How often (in ticks) idle dwarves look for work.
pub const ASSIGN_INTERVAL: u64 = 5;
/// Pathfinding safety valve.
pub const MAX_ASTAR_NODES: usize = 50_000;
/// Ticks before an unreachable designation/item is reconsidered.
pub const RETRY_DELAY: u64 = 200;
/// Need level at which a dwarf goes looking for food/drink.
pub const NEED_AT: f32 = 60.0;
/// Ticks at a maxed-out need before it kills.
pub const NEED_DEATH_TICKS: u64 = 6 * TICKS_PER_DAY;
/// Population cap for Phase 2.
pub const POP_CAP: usize = 15;
/// Outputs per brew/cook batch.
pub const BATCH: usize = 3;

const HUNGER_RATE: f32 = 0.004;
const THIRST_RATE: f32 = 0.005;
const FATIGUE_RATE: f32 = 0.0015;

// ------------------------------------------------------------ designations

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DesignationKind {
    Mine,
    Stairs,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Designation {
    pub kind: DesignationKind,
    pub assigned: bool,
    pub retry_at: u64,
}

// ------------------------------------------------------------------- items

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ItemKind {
    /// Mined stone. `stuff` = material index.
    Boulder,
    /// Plantable seed. `stuff` = plant index.
    Seed,
    /// Harvested crop. `stuff` = plant index.
    Crop,
    /// Prepared food. `stuff` = plant index it was cooked from.
    Meal,
    /// Brewed drink. `stuff` = plant index it was brewed from.
    Drink,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ItemState {
    OnGround,
    Carried { by: usize },
    Stored { stockpile: usize },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Item {
    pub kind: ItemKind,
    /// Material index for boulders, plant index for everything else.
    pub stuff: u16,
    pub pos: Pos,
    pub state: ItemState,
    /// Dwarf index that has claimed this item.
    pub reserved_by: Option<usize>,
    /// Consumed items stay in the vec (indices are load-bearing) but are
    /// invisible to every query. Arena/slotmap refactor is planned Phase 3.
    pub consumed: bool,
}

impl Item {
    pub fn active(&self) -> bool {
        !self.consumed
    }
}

// --------------------------------------------------------------- buildings

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BuildingKind {
    Still,
    Kitchen,
}

impl BuildingKind {
    pub fn name(self) -> &'static str {
        match self {
            BuildingKind::Still => "Still",
            BuildingKind::Kitchen => "Kitchen",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Building {
    pub kind: BuildingKind,
    pub pos: Pos,
}

// ------------------------------------------------------------------- farms

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FarmState {
    Fallow,
    Growing { progress: u32 },
    Grown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct FarmTile {
    pub crop: u16,
    pub state: FarmState,
    pub reserved: bool,
}

// -------------------------------------------------------------- stockpiles

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

// ---------------------------------------------------------------- thoughts

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThoughtKind {
    AteMeal,
    HadDrink,
    AteRawFood,
    Hungry,
    Thirsty,
    Starving,
    Dehydrated,
    HarvestedCrop,
    BrewedDrink,
    CookedMeal,
    ArrivedAtFort,
}

impl ThoughtKind {
    pub fn delta(self) -> f32 {
        match self {
            ThoughtKind::AteMeal | ThoughtKind::HadDrink => 4.0,
            ThoughtKind::AteRawFood => -1.0,
            ThoughtKind::Hungry | ThoughtKind::Thirsty => -3.0,
            ThoughtKind::Starving | ThoughtKind::Dehydrated => -10.0,
            ThoughtKind::HarvestedCrop
            | ThoughtKind::BrewedDrink
            | ThoughtKind::CookedMeal => 2.0,
            ThoughtKind::ArrivedAtFort => 3.0,
        }
    }

    pub fn text(self) -> &'static str {
        match self {
            ThoughtKind::AteMeal => "enjoyed a proper meal",
            ThoughtKind::HadDrink => "had a good drink",
            ThoughtKind::AteRawFood => "ate raw food, joylessly",
            ThoughtKind::Hungry => "is hungry",
            ThoughtKind::Thirsty => "is thirsty",
            ThoughtKind::Starving => "is starving!",
            ThoughtKind::Dehydrated => "is dying of thirst!",
            ThoughtKind::HarvestedCrop => "took pride in a harvest",
            ThoughtKind::BrewedDrink => "brewed a fine batch",
            ThoughtKind::CookedMeal => "cooked a hearty meal",
            ThoughtKind::ArrivedAtFort => "arrived at the fortress",
        }
    }
}

// ------------------------------------------------------------------ skills

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Skill {
    Mining,
    Farming,
    Brewing,
    Cooking,
}

// ------------------------------------------------------------------- tasks

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FetchStage {
    ToInput,
    ToStation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CraftKind {
    Brew,
    Cook,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Task {
    Idle { wander_cd: u16 },
    Sleep { remaining: u16 },
    Mine { target: Pos, path: Vec<Pos>, progress: u16 },
    Haul { item: usize, dest: Pos, path: Vec<Pos>, carrying: bool },
    Eat { item: usize, path: Vec<Pos> },
    Drink { item: usize, path: Vec<Pos> },
    Plant { tile: Pos, seed: usize, path: Vec<Pos>, stage: FetchStage },
    Harvest { tile: Pos, path: Vec<Pos>, progress: u16 },
    Craft { shop: Pos, input: usize, kind: CraftKind, path: Vec<Pos>, stage: FetchStage, progress: u16 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dwarf {
    pub name: String,
    pub pos: Pos,
    pub alive: bool,
    pub hunger: f32,
    pub thirst: f32,
    pub fatigue: f32,
    pub happiness: f32,
    pub thoughts: Vec<(u64, ThoughtKind)>,
    pub skills: BTreeMap<Skill, u32>,
    pub task: Task,
    move_cd: u8,
    /// Tick a fully maxed need started, for death countdowns.
    starving_since: Option<u64>,
    dehydrated_since: Option<u64>,
}

impl Dwarf {
    pub fn is_idle(&self) -> bool {
        matches!(self.task, Task::Idle { .. })
    }

    pub fn skill_level(&self, s: Skill) -> u32 {
        (self.skills.get(&s).copied().unwrap_or(0) / 100).min(6)
    }

    pub fn task_name(&self) -> &'static str {
        match self.task {
            Task::Idle { .. } => "idle",
            Task::Sleep { .. } => "sleeping",
            Task::Mine { .. } => "mining",
            Task::Haul { .. } => "hauling",
            Task::Eat { .. } => "getting food",
            Task::Drink { .. } => "getting a drink",
            Task::Plant { .. } => "planting",
            Task::Harvest { .. } => "harvesting",
            Task::Craft { kind: CraftKind::Brew, .. } => "brewing",
            Task::Craft { kind: CraftKind::Cook, .. } => "cooking",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct SimStats {
    pub crops_harvested: u32,
    pub meals_cooked: u32,
    pub drinks_brewed: u32,
    pub migrants_arrived: u32,
    pub deaths: u32,
}

// --------------------------------------------------------------------- sim

#[derive(Serialize, Deserialize)]
pub struct Sim {
    pub map: Map,
    pub dwarves: Vec<Dwarf>,
    pub items: Vec<Item>,
    pub stockpiles: Vec<Stockpile>,
    pub buildings: Vec<Building>,
    pub farms: BTreeMap<Pos, FarmTile>,
    pub designations: BTreeMap<Pos, Designation>,
    pub stats: SimStats,
    pub clock: Calendar,
    rng: ChaCha8Rng,
    /// Per-item back-off after a failed haul pathfind (item index -> tick).
    haul_retry: BTreeMap<usize, u64>,
    /// Set whenever terrain changes; the renderer reads and clears it.
    #[serde(skip)]
    pub map_changed: bool,
    #[serde(skip, default)]
    regions: Regions,
}

impl Sim {
    pub fn new(map: Map, raws: &Raws, mut rng: ChaCha8Rng, dwarf_count: usize) -> Self {
        let _ = raws;
        let cx = map.width as i32 / 2;
        let cy = map.height as i32 / 2;
        let regions = Regions::new(&map);

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
                        dwarves.push(new_dwarf(&mut rng, pos));
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
            buildings: Vec::new(),
            farms: BTreeMap::new(),
            designations: BTreeMap::new(),
            stats: SimStats::default(),
            clock: Calendar::default(),
            rng,
            haul_retry: BTreeMap::new(),
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
                    self.designations
                        .insert(p, Designation { kind, assigned: false, retry_at: 0 });
                    added += 1;
                }
            }
        }
        added
    }

    pub fn cancel_rect(&mut self, a: Pos, b: Pos) -> usize {
        assert_eq!(a.z, b.z);
        let mut removed = 0;
        for y in a.y.min(b.y)..=a.y.max(b.y) {
            for x in a.x.min(b.x)..=a.x.max(b.x) {
                let p = Pos::new(x, y, a.z);
                if self.designations.remove(&p).is_some() {
                    removed += 1;
                    for i in 0..self.dwarves.len() {
                        if matches!(self.dwarves[i].task, Task::Mine { target, .. } if target == p)
                        {
                            self.abandon_task(i);
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

    /// Turn every walkable tile in the rect into a fallow farm tile.
    pub fn add_farm(&mut self, a: Pos, b: Pos, crop: u16) -> usize {
        assert_eq!(a.z, b.z);
        let mut added = 0;
        for y in a.y.min(b.y)..=a.y.max(b.y) {
            for x in a.x.min(b.x)..=a.x.max(b.x) {
                let p = Pos::new(x, y, a.z);
                if self.map.walkable(p) && !self.farms.contains_key(&p) {
                    self.farms
                        .insert(p, FarmTile { crop, state: FarmState::Fallow, reserved: false });
                    added += 1;
                }
            }
        }
        added
    }

    pub fn add_building(&mut self, kind: BuildingKind, pos: Pos) -> bool {
        if !self.map.walkable(pos)
            || self.buildings.iter().any(|b| b.pos == pos)
            || self.farms.contains_key(&pos)
        {
            return false;
        }
        self.buildings.push(Building { kind, pos });
        true
    }

    pub fn stockpile_at(&self, p: Pos) -> Option<usize> {
        self.stockpiles.iter().position(|s| s.contains(p))
    }

    pub fn building_at(&self, p: Pos) -> Option<&Building> {
        self.buildings.iter().find(|b| b.pos == p)
    }

    /// Scatter starting supplies on walkable ground near the map center.
    pub fn add_embark_supplies(&mut self, raws: &Raws) {
        let cx = self.map.width as i32 / 2;
        let cy = self.map.height as i32 / 2;
        let mut supplies: Vec<(ItemKind, u16)> = Vec::new();
        for _ in 0..25 {
            supplies.push((ItemKind::Meal, 0));
            supplies.push((ItemKind::Drink, 0));
        }
        for (idx, _) in raws.plants.iter() {
            for _ in 0..8 {
                supplies.push((ItemKind::Seed, idx));
            }
        }
        let mut placed = 0usize;
        'place: for radius in 1..(self.map.width as i32 / 2) {
            for dy in -radius..=radius {
                for dx in -radius..=radius {
                    if placed >= supplies.len() {
                        break 'place;
                    }
                    if dx.abs() != radius && dy.abs() != radius {
                        continue; // ring only
                    }
                    let (x, y) = (cx + dx, cy + dy);
                    if x < 1 || y < 1 || x >= self.map.width as i32 - 1 || y >= self.map.height as i32 - 1 {
                        continue;
                    }
                    if let Some(z) = self.map.walk_surface_z(x as usize, y as usize) {
                        let (kind, stuff) = supplies[placed];
                        self.items.push(Item {
                            kind,
                            stuff,
                            pos: Pos::new(x, y, z as i32),
                            state: ItemState::OnGround,
                            reserved_by: None,
                            consumed: false,
                        });
                        placed += 1;
                    }
                }
            }
        }
    }

    /// Demo/test helper: tile flat 3x3 stockpile patches near (cx, cy),
    /// closest first, until combined capacity reaches `target_cells`.
    pub fn place_flat_stockpiles(&mut self, cx: i32, cy: i32, target_cells: usize) -> usize {
        let mut cells = 0;
        for (x, y, z) in self.flat_patches(cx, cy) {
            if cells >= target_cells {
                break;
            }
            let (x1, y1) = (x + 2, y + 2);
            let overlaps = self
                .stockpiles
                .iter()
                .any(|s| x <= s.x1 && s.x0 <= x1 && y <= s.y1 && s.y0 <= y1);
            let farmed = (y..=y1)
                .any(|yy| (x..=x1).any(|xx| self.farms.contains_key(&Pos::new(xx, yy, z))));
            if !overlaps && !farmed {
                self.add_stockpile(Pos::new(x, y, z), Pos::new(x1, y1, z));
                cells += 9;
            }
        }
        cells
    }

    /// Flat 3x3 patch origins sorted by distance to (cx, cy).
    fn flat_patches(&self, cx: i32, cy: i32) -> Vec<(i32, i32, i32)> {
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
        candidates.into_iter().map(|(_, x, y, z)| (x, y, z)).collect()
    }

    /// Demo/test helper: nearest flat 3x3 patch not already used by a farm,
    /// stockpile, or building.
    pub fn find_flat_patch(&self, cx: i32, cy: i32) -> Option<(Pos, Pos)> {
        for (x, y, z) in self.flat_patches(cx, cy) {
            let (x1, y1) = (x + 2, y + 2);
            let clash = (y..=y1).any(|yy| {
                (x..=x1).any(|xx| {
                    let p = Pos::new(xx, yy, z);
                    self.farms.contains_key(&p)
                        || self.stockpile_at(p).is_some()
                        || self.building_at(p).is_some()
                })
            });
            if !clash {
                return Some((Pos::new(x, y, z), Pos::new(x1, y1, z)));
            }
        }
        None
    }

    // ------------------------------------------------------------- queries

    fn cell_free(&self, cell: Pos) -> bool {
        if !self.map.walkable(cell) {
            return false;
        }
        if self.items.iter().any(|it| {
            it.active()
                && it.pos == cell
                && matches!(it.state, ItemState::Stored { .. } | ItemState::OnGround)
        }) {
            return false;
        }
        !self
            .dwarves
            .iter()
            .any(|d| d.alive && matches!(d.task, Task::Haul { dest, .. } if dest == cell))
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
            .filter(|i| i.active() && matches!(i.state, ItemState::Stored { .. }))
            .count()
    }

    pub fn count_kind(&self, kind: ItemKind) -> usize {
        self.items.iter().filter(|i| i.active() && i.kind == kind).count()
    }

    pub fn alive_dwarves(&self) -> usize {
        self.dwarves.iter().filter(|d| d.alive).count()
    }

    /// Is an item claimable as a consumable/ingredient right now?
    fn item_takeable(&self, it: &Item) -> bool {
        it.active()
            && it.reserved_by.is_none()
            && matches!(it.state, ItemState::OnGround | ItemState::Stored { .. })
    }

    // ------------------------------------------------------------- stepping

    pub fn step(&mut self, raws: &Raws) {
        self.clock.advance();
        if self.regions.dirty {
            self.regions.rebuild(&self.map);
        }
        self.grow_farms(raws);
        if self.clock.tick % ASSIGN_INTERVAL == 0 {
            self.assign_jobs(raws);
        }
        for i in 0..self.dwarves.len() {
            if self.dwarves[i].alive {
                self.update_dwarf(i, raws);
            }
        }
        // Season boundary: migrants may arrive.
        let season_ticks = TICKS_PER_DAY * DAYS_PER_SEASON;
        if self.clock.tick % season_ticks == 0 && self.clock.tick > 0 {
            self.maybe_migrants(raws);
        }
    }

    fn grow_farms(&mut self, raws: &Raws) {
        let season = self.clock.season_index();
        for tile in self.farms.values_mut() {
            if let FarmState::Growing { progress } = tile.state {
                let plant = raws.plants.get(tile.crop);
                if !plant.grows_in(season) {
                    continue; // dormant out of season
                }
                let done = plant.grow_days as u64 * TICKS_PER_DAY;
                let next = progress as u64 + 1;
                tile.state = if next >= done {
                    FarmState::Grown
                } else {
                    FarmState::Growing { progress: next as u32 }
                };
            }
        }
    }

    fn maybe_migrants(&mut self, raws: &Raws) {
        let _ = raws;
        let alive = self.alive_dwarves();
        if alive == 0 || alive >= POP_CAP {
            return;
        }
        let food = self.count_kind(ItemKind::Meal) + self.count_kind(ItemKind::Crop);
        if food < alive {
            return; // word gets out that the fort is starving
        }
        let Some(anchor) = self.dwarves.iter().find(|d| d.alive).map(|d| d.pos) else {
            return;
        };
        let anchor_region = self.regions.id(anchor);
        let count = self.rng.gen_range(1..=3usize).min(POP_CAP - alive);
        let mut spawned = 0;
        'outer: for y in 1..self.map.height - 1 {
            for x in [1usize, self.map.width - 2] {
                if spawned >= count {
                    break 'outer;
                }
                if let Some(z) = self.map.walk_surface_z(x, y) {
                    let pos = Pos::new(x as i32, y as i32, z as i32);
                    if self.regions.id(pos) == anchor_region {
                        let mut d = new_dwarf(&mut self.rng, pos);
                        d.thoughts.push((self.clock.tick, ThoughtKind::ArrivedAtFort));
                        d.happiness += ThoughtKind::ArrivedAtFort.delta();
                        self.dwarves.push(d);
                        self.stats.migrants_arrived += 1;
                        spawned += 1;
                    }
                }
            }
        }
    }

    // ---------------------------------------------------------- assignment

    fn assign_jobs(&mut self, raws: &Raws) {
        let alive = self.alive_dwarves();
        let want_drinks = self.count_kind(ItemKind::Drink) < alive * 3;
        let want_meals = self.count_kind(ItemKind::Meal) < alive * 3;
        for i in 0..self.dwarves.len() {
            if self.dwarves[i].alive && self.dwarves[i].is_idle() {
                self.assign_one(i, raws, want_drinks, want_meals);
            }
        }
    }

    fn assign_one(&mut self, i: usize, raws: &Raws, want_drinks: bool, want_meals: bool) {
        let dwarf_pos = self.dwarves[i].pos;
        let my_region = self.regions.id(dwarf_pos);
        if my_region == 0 {
            return;
        }
        let tick = self.clock.tick;

        // --- Needs come first.
        if self.dwarves[i].hunger >= NEED_AT {
            if let Some(item) = self.nearest_food(dwarf_pos, my_region) {
                if self.start_goto_item(i, item, |it, p| Task::Eat { item: it, path: p }) {
                    return;
                }
            }
        }
        if self.dwarves[i].thirst >= NEED_AT {
            if let Some(item) = self.nearest_kind(ItemKind::Drink, dwarf_pos, my_region) {
                if self.start_goto_item(i, item, |it, p| Task::Drink { item: it, path: p }) {
                    return;
                }
            }
        }

        // --- Work candidates, nearest wins (insertion order breaks ties).
        enum Cand {
            Mine { target: Pos, work: Pos },
            Plant { tile: Pos, seed: usize },
            Harvest { tile: Pos },
            Craft { shop: Pos, input: usize, kind: CraftKind },
            Haul { item: usize, dest: Pos },
        }
        let mut best: Option<(u32, Cand)> = None;
        let mut consider = |dist: u32, c: Cand, best: &mut Option<(u32, Cand)>| {
            if best.as_ref().is_none_or(|(bd, _)| dist < *bd) {
                *best = Some((dist, c));
            }
        };

        // Mining.
        let mut scratch = Vec::with_capacity(6);
        for (&target, des) in &self.designations {
            if des.assigned || des.retry_at > tick {
                continue;
            }
            path::work_positions(&self.map, target, &mut scratch);
            if let Some(&work) = scratch
                .iter()
                .filter(|&&w| self.regions.id(w) == my_region)
                .min_by_key(|&&w| w.manhattan(dwarf_pos))
            {
                consider(work.manhattan(dwarf_pos), Cand::Mine { target, work }, &mut best);
            }
        }

        // Farming: plant fallow tiles in season, harvest grown ones.
        let season = self.clock.season_index();
        for (&tile, farm) in &self.farms {
            if farm.reserved || self.regions.id(tile) != my_region {
                continue;
            }
            match farm.state {
                FarmState::Fallow => {
                    if raws.plants.get(farm.crop).grows_in(season) {
                        if let Some(seed) = self.nearest_seed(farm.crop, tile, my_region) {
                            consider(tile.manhattan(dwarf_pos), Cand::Plant { tile, seed }, &mut best);
                        }
                    }
                }
                FarmState::Grown => {
                    consider(tile.manhattan(dwarf_pos), Cand::Harvest { tile }, &mut best);
                }
                FarmState::Growing { .. } => {}
            }
        }

        // Crafting: keep the fort in drink and food.
        if want_drinks {
            if let Some((shop, input)) =
                self.craft_pair(BuildingKind::Still, true, dwarf_pos, my_region, raws)
            {
                let d = self.items[input].pos.manhattan(dwarf_pos);
                consider(d, Cand::Craft { shop, input, kind: CraftKind::Brew }, &mut best);
            }
        }
        if want_meals {
            if let Some((shop, input)) =
                self.craft_pair(BuildingKind::Kitchen, false, dwarf_pos, my_region, raws)
            {
                let d = self.items[input].pos.manhattan(dwarf_pos);
                consider(d, Cand::Craft { shop, input, kind: CraftKind::Cook }, &mut best);
            }
        }

        // Hauling loose items to stockpiles.
        if !self.stockpiles.is_empty() {
            for (idx, item) in self.items.iter().enumerate() {
                if !item.active()
                    || item.state != ItemState::OnGround
                    || item.reserved_by.is_some()
                {
                    continue;
                }
                if self.haul_retry.get(&idx).is_some_and(|&t| t > tick) {
                    continue;
                }
                if self.regions.id(item.pos) != my_region {
                    continue;
                }
                let Some(dest) = self.find_free_cell(item.pos, my_region) else { continue };
                consider(item.pos.manhattan(dwarf_pos), Cand::Haul { item: idx, dest }, &mut best);
            }
        }

        // --- Commit the winner.
        let Some((_, cand)) = best else { return };
        match cand {
            Cand::Mine { target, work } => {
                match path::astar(&self.map, dwarf_pos, work, MAX_ASTAR_NODES) {
                    Some(p) => {
                        self.designations.get_mut(&target).unwrap().assigned = true;
                        self.dwarves[i].task = Task::Mine { target, path: p, progress: 0 };
                    }
                    None => {
                        self.designations.get_mut(&target).unwrap().retry_at = tick + RETRY_DELAY;
                    }
                }
            }
            Cand::Plant { tile, seed } => {
                let seed_pos = self.items[seed].pos;
                if let Some(p) = path::astar(&self.map, dwarf_pos, seed_pos, MAX_ASTAR_NODES) {
                    self.items[seed].reserved_by = Some(i);
                    self.farms.get_mut(&tile).unwrap().reserved = true;
                    self.dwarves[i].task =
                        Task::Plant { tile, seed, path: p, stage: FetchStage::ToInput };
                }
            }
            Cand::Harvest { tile } => {
                if let Some(p) = path::astar(&self.map, dwarf_pos, tile, MAX_ASTAR_NODES) {
                    self.farms.get_mut(&tile).unwrap().reserved = true;
                    self.dwarves[i].task = Task::Harvest { tile, path: p, progress: 0 };
                }
            }
            Cand::Craft { shop, input, kind } => {
                let input_pos = self.items[input].pos;
                if let Some(p) = path::astar(&self.map, dwarf_pos, input_pos, MAX_ASTAR_NODES) {
                    self.items[input].reserved_by = Some(i);
                    self.dwarves[i].task = Task::Craft {
                        shop,
                        input,
                        kind,
                        path: p,
                        stage: FetchStage::ToInput,
                        progress: 0,
                    };
                }
            }
            Cand::Haul { item, dest } => {
                let item_pos = self.items[item].pos;
                match path::astar(&self.map, dwarf_pos, item_pos, MAX_ASTAR_NODES) {
                    Some(p) => {
                        self.items[item].reserved_by = Some(i);
                        self.dwarves[i].task =
                            Task::Haul { item, dest, path: p, carrying: false };
                    }
                    None => {
                        self.haul_retry.insert(item, tick + RETRY_DELAY);
                    }
                }
            }
        }
    }

    fn nearest_kind(&self, kind: ItemKind, near: Pos, region: u32) -> Option<usize> {
        self.items
            .iter()
            .enumerate()
            .filter(|(_, it)| {
                it.kind == kind && self.item_takeable(it) && self.regions.id(it.pos) == region
            })
            .min_by_key(|(_, it)| it.pos.manhattan(near))
            .map(|(i, _)| i)
    }

    /// Meals first; raw crops as a joyless fallback.
    fn nearest_food(&self, near: Pos, region: u32) -> Option<usize> {
        self.nearest_kind(ItemKind::Meal, near, region)
            .or_else(|| self.nearest_kind(ItemKind::Crop, near, region))
    }

    fn nearest_seed(&self, crop: u16, near: Pos, region: u32) -> Option<usize> {
        self.items
            .iter()
            .enumerate()
            .filter(|(_, it)| {
                it.kind == ItemKind::Seed
                    && it.stuff == crop
                    && self.item_takeable(it)
                    && self.regions.id(it.pos) == region
            })
            .min_by_key(|(_, it)| it.pos.manhattan(near))
            .map(|(i, _)| i)
    }

    /// Nearest (workshop, ingredient) pair for brewing/cooking.
    fn craft_pair(
        &self,
        shop_kind: BuildingKind,
        brewable: bool,
        near: Pos,
        region: u32,
        raws: &Raws,
    ) -> Option<(Pos, usize)> {
        let shop = self
            .buildings
            .iter()
            .filter(|b| b.kind == shop_kind && self.regions.id(b.pos) == region)
            .min_by_key(|b| b.pos.manhattan(near))?;
        let input = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, it)| {
                it.kind == ItemKind::Crop
                    && self.item_takeable(it)
                    && self.regions.id(it.pos) == region
                    && (!brewable || raws.plants.get(it.stuff).brewable)
            })
            .min_by_key(|(_, it)| it.pos.manhattan(near))
            .map(|(i, _)| i)?;
        Some((shop.pos, input))
    }

    /// Reserve `item` and path toward it; returns false if unreachable.
    fn start_goto_item(
        &mut self,
        i: usize,
        item: usize,
        make: impl Fn(usize, Vec<Pos>) -> Task,
    ) -> bool {
        let target = self.items[item].pos;
        match path::astar(&self.map, self.dwarves[i].pos, target, MAX_ASTAR_NODES) {
            Some(p) => {
                self.items[item].reserved_by = Some(i);
                self.dwarves[i].task = make(item, p);
                true
            }
            None => false,
        }
    }

    // -------------------------------------------------------------- update

    fn update_dwarf(&mut self, i: usize, raws: &Raws) {
        self.tick_needs(i);
        if !self.dwarves[i].alive {
            return; // needs may have killed them this very tick
        }

        let task = self.dwarves[i].task.clone();
        match task {
            Task::Idle { wander_cd } => {
                if self.dwarves[i].fatigue >= 100.0 {
                    self.dwarves[i].task = Task::Sleep { remaining: 1200 };
                    return;
                }
                if wander_cd > 0 {
                    self.dwarves[i].task = Task::Idle { wander_cd: wander_cd - 1 };
                } else {
                    let pos = self.dwarves[i].pos;
                    let mut opts = Vec::with_capacity(8);
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
                if !self.designations.contains_key(&target) {
                    self.dwarves[i].task = Task::Idle { wander_cd: 5 };
                    return;
                }
                if !path.is_empty() {
                    if self.step_along(i, &mut path) {
                        self.dwarves[i].task = Task::Mine { target, path, progress };
                    } else {
                        self.abandon_task(i);
                    }
                    return;
                }
                let speed = 1 + self.dwarves[i].skill_level(Skill::Mining) as u16 / 2;
                let progress = progress + speed;
                if progress < MINE_WORK {
                    self.dwarves[i].task = Task::Mine { target, path, progress };
                    return;
                }
                self.complete_mine(i, target, raws);
            }
            Task::Haul { item, dest, mut path, carrying } => {
                if !carrying {
                    if !path.is_empty() {
                        if self.step_along(i, &mut path) {
                            self.dwarves[i].task = Task::Haul { item, dest, path, carrying };
                        } else {
                            self.abandon_task(i);
                        }
                        return;
                    }
                    if !self.take_item(i, item) {
                        self.abandon_task(i);
                        return;
                    }
                    match path::astar(&self.map, self.dwarves[i].pos, dest, MAX_ASTAR_NODES) {
                        Some(p) => {
                            self.dwarves[i].task =
                                Task::Haul { item, dest, path: p, carrying: true };
                        }
                        None => self.abandon_task(i),
                    }
                } else {
                    if !path.is_empty() {
                        if self.step_along(i, &mut path) {
                            self.dwarves[i].task = Task::Haul { item, dest, path, carrying };
                        } else {
                            self.abandon_task(i);
                        }
                        return;
                    }
                    let here = self.dwarves[i].pos;
                    let taken = self.items.iter().enumerate().any(|(j, it)| {
                        j != item
                            && it.active()
                            && it.pos == here
                            && it.state != (ItemState::Carried { by: i })
                    });
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
            Task::Eat { item, mut path } | Task::Drink { item, mut path } => {
                let drinking = matches!(self.dwarves[i].task, Task::Drink { .. });
                if !path.is_empty() {
                    if self.step_along(i, &mut path) {
                        self.dwarves[i].task = if drinking {
                            Task::Drink { item, path }
                        } else {
                            Task::Eat { item, path }
                        };
                    } else {
                        self.abandon_task(i);
                    }
                    return;
                }
                // Consume on the spot.
                let it = &self.items[item];
                if !it.active() || it.pos != self.dwarves[i].pos || it.reserved_by != Some(i) {
                    self.abandon_task(i);
                    return;
                }
                let kind = it.kind;
                self.items[item].consumed = true;
                self.items[item].reserved_by = None;
                if drinking {
                    self.dwarves[i].thirst = 0.0;
                    self.dwarves[i].dehydrated_since = None;
                    self.push_thought(i, ThoughtKind::HadDrink);
                } else {
                    self.dwarves[i].hunger = 0.0;
                    self.dwarves[i].starving_since = None;
                    self.push_thought(
                        i,
                        if kind == ItemKind::Meal {
                            ThoughtKind::AteMeal
                        } else {
                            ThoughtKind::AteRawFood
                        },
                    );
                }
                self.dwarves[i].task = Task::Idle { wander_cd: 5 };
            }
            Task::Plant { tile, seed, mut path, stage } => {
                if !path.is_empty() {
                    if self.step_along(i, &mut path) {
                        self.dwarves[i].task = Task::Plant { tile, seed, path, stage };
                    } else {
                        self.abandon_task(i);
                    }
                    return;
                }
                match stage {
                    FetchStage::ToInput => {
                        if !self.take_item(i, seed) {
                            self.abandon_task(i);
                            return;
                        }
                        match path::astar(&self.map, self.dwarves[i].pos, tile, MAX_ASTAR_NODES) {
                            Some(p) => {
                                self.dwarves[i].task = Task::Plant {
                                    tile,
                                    seed,
                                    path: p,
                                    stage: FetchStage::ToStation,
                                };
                            }
                            None => self.abandon_task(i),
                        }
                    }
                    FetchStage::ToStation => {
                        let ok = matches!(
                            self.farms.get(&tile),
                            Some(FarmTile { state: FarmState::Fallow, .. })
                        );
                        if ok {
                            self.items[seed].consumed = true;
                            self.items[seed].reserved_by = None;
                            let farm = self.farms.get_mut(&tile).unwrap();
                            farm.state = FarmState::Growing { progress: 0 };
                            farm.reserved = false;
                            self.add_xp(i, Skill::Farming, 15);
                        } else {
                            self.drop_carried(i);
                            if let Some(f) = self.farms.get_mut(&tile) {
                                f.reserved = false;
                            }
                        }
                        self.dwarves[i].task = Task::Idle { wander_cd: 3 };
                    }
                }
            }
            Task::Harvest { tile, mut path, progress } => {
                if !path.is_empty() {
                    if self.step_along(i, &mut path) {
                        self.dwarves[i].task = Task::Harvest { tile, path, progress };
                    } else {
                        self.abandon_task(i);
                    }
                    return;
                }
                let ok = matches!(
                    self.farms.get(&tile),
                    Some(FarmTile { state: FarmState::Grown, .. })
                );
                if !ok {
                    self.abandon_task(i);
                    return;
                }
                let speed = 1 + self.dwarves[i].skill_level(Skill::Farming) as u16 / 2;
                let progress = progress + speed;
                if progress < HARVEST_WORK {
                    self.dwarves[i].task = Task::Harvest { tile, path, progress };
                    return;
                }
                let crop = self.farms.get(&tile).unwrap().crop;
                let farm = self.farms.get_mut(&tile).unwrap();
                farm.state = FarmState::Fallow;
                farm.reserved = false;
                self.spawn_item(ItemKind::Crop, crop, tile);
                self.spawn_item(ItemKind::Seed, crop, tile);
                self.spawn_item(ItemKind::Seed, crop, tile);
                self.stats.crops_harvested += 1;
                self.add_xp(i, Skill::Farming, 25);
                self.push_thought(i, ThoughtKind::HarvestedCrop);
                self.dwarves[i].task = Task::Idle { wander_cd: 3 };
            }
            Task::Craft { shop, input, kind, mut path, stage, progress } => {
                if !path.is_empty() {
                    if self.step_along(i, &mut path) {
                        self.dwarves[i].task =
                            Task::Craft { shop, input, kind, path, stage, progress };
                    } else {
                        self.abandon_task(i);
                    }
                    return;
                }
                match stage {
                    FetchStage::ToInput => {
                        if !self.take_item(i, input) {
                            self.abandon_task(i);
                            return;
                        }
                        match path::astar(&self.map, self.dwarves[i].pos, shop, MAX_ASTAR_NODES) {
                            Some(p) => {
                                self.dwarves[i].task = Task::Craft {
                                    shop,
                                    input,
                                    kind,
                                    path: p,
                                    stage: FetchStage::ToStation,
                                    progress,
                                };
                            }
                            None => self.abandon_task(i),
                        }
                    }
                    FetchStage::ToStation => {
                        let skill = match kind {
                            CraftKind::Brew => Skill::Brewing,
                            CraftKind::Cook => Skill::Cooking,
                        };
                        let speed = 1 + self.dwarves[i].skill_level(skill) as u16 / 2;
                        let progress = progress + speed;
                        if progress < CRAFT_WORK {
                            self.dwarves[i].task =
                                Task::Craft { shop, input, kind, path, stage, progress };
                            return;
                        }
                        let stuff = self.items[input].stuff;
                        self.items[input].consumed = true;
                        self.items[input].reserved_by = None;
                        let (out_kind, thought) = match kind {
                            CraftKind::Brew => {
                                self.stats.drinks_brewed += BATCH as u32;
                                (ItemKind::Drink, ThoughtKind::BrewedDrink)
                            }
                            CraftKind::Cook => {
                                self.stats.meals_cooked += BATCH as u32;
                                (ItemKind::Meal, ThoughtKind::CookedMeal)
                            }
                        };
                        for _ in 0..BATCH {
                            self.spawn_item(out_kind, stuff, shop);
                        }
                        self.add_xp(i, skill, 30);
                        self.push_thought(i, thought);
                        self.dwarves[i].task = Task::Idle { wander_cd: 3 };
                    }
                }
            }
        }
    }

    /// Needs tick + hunger/thirst thoughts + death countdowns.
    fn tick_needs(&mut self, i: usize) {
        let tick = self.clock.tick;
        let d = &mut self.dwarves[i];
        let old_hunger = d.hunger;
        let old_thirst = d.thirst;
        d.hunger = (d.hunger + HUNGER_RATE).min(100.0);
        d.thirst = (d.thirst + THIRST_RATE).min(100.0);
        d.fatigue = (d.fatigue + FATIGUE_RATE).min(100.0);
        // Happiness drifts back toward neutral.
        d.happiness += (50.0 - d.happiness).signum() * 0.0005;

        let crossed_hungry = old_hunger < NEED_AT && d.hunger >= NEED_AT;
        let crossed_thirsty = old_thirst < NEED_AT && d.thirst >= NEED_AT;
        let now_starving = d.hunger >= 100.0 && d.starving_since.is_none();
        let now_dehydrated = d.thirst >= 100.0 && d.dehydrated_since.is_none();
        if now_starving {
            d.starving_since = Some(tick);
        }
        if now_dehydrated {
            d.dehydrated_since = Some(tick);
        }
        let starved = d.starving_since.is_some_and(|t| tick - t > NEED_DEATH_TICKS);
        let died_of_thirst = d.dehydrated_since.is_some_and(|t| tick - t > NEED_DEATH_TICKS);

        if crossed_hungry {
            self.push_thought(i, ThoughtKind::Hungry);
        }
        if crossed_thirsty {
            self.push_thought(i, ThoughtKind::Thirsty);
        }
        if now_starving {
            self.push_thought(i, ThoughtKind::Starving);
        }
        if now_dehydrated {
            self.push_thought(i, ThoughtKind::Dehydrated);
        }
        if starved || died_of_thirst {
            self.kill_dwarf(i);
        }
    }

    fn kill_dwarf(&mut self, i: usize) {
        self.abandon_task(i);
        self.drop_carried(i);
        // Release anything still pointing at this dwarf.
        for it in &mut self.items {
            if it.reserved_by == Some(i) {
                it.reserved_by = None;
            }
        }
        self.dwarves[i].alive = false;
        self.stats.deaths += 1;
    }

    fn push_thought(&mut self, i: usize, kind: ThoughtKind) {
        let tick = self.clock.tick;
        let d = &mut self.dwarves[i];
        d.happiness = (d.happiness + kind.delta()).clamp(0.0, 100.0);
        d.thoughts.push((tick, kind));
        if d.thoughts.len() > 8 {
            d.thoughts.remove(0);
        }
    }

    fn add_xp(&mut self, i: usize, skill: Skill, xp: u32) {
        *self.dwarves[i].skills.entry(skill).or_insert(0) += xp;
    }

    fn spawn_item(&mut self, kind: ItemKind, stuff: u16, pos: Pos) {
        self.items.push(Item {
            kind,
            stuff,
            pos,
            state: ItemState::OnGround,
            reserved_by: None,
            consumed: false,
        });
    }

    /// Pick up an item the dwarf is standing on. Returns false if it's gone.
    fn take_item(&mut self, i: usize, item: usize) -> bool {
        let it = &self.items[item];
        if !it.active()
            || it.pos != self.dwarves[i].pos
            || it.reserved_by != Some(i)
            || !matches!(it.state, ItemState::OnGround | ItemState::Stored { .. })
        {
            return false;
        }
        self.items[item].state = ItemState::Carried { by: i };
        true
    }

    /// Move one step along `path` (respecting walk cooldown). Returns false
    /// if the next step stopped being a legal move (terrain changed).
    fn step_along(&mut self, i: usize, path: &mut Vec<Pos>) -> bool {
        if self.dwarves[i].move_cd > 0 {
            self.dwarves[i].move_cd -= 1;
            return true;
        }
        let next = path[0];
        let mut legal = Vec::with_capacity(8);
        path::neighbors(&self.map, self.dwarves[i].pos, &mut legal);
        if !legal.contains(&next) {
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

    fn complete_mine(&mut self, i: usize, target: Pos, raws: &Raws) {
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

        if tile.is_solid()
            && raws.materials.get(tile.material).category != MaterialCategory::Soil
        {
            self.spawn_item(ItemKind::Boulder, tile.material, target);
        }
        self.add_xp(i, Skill::Mining, 20);
        self.dwarves[i].task = Task::Idle { wander_cd: 2 };
    }

    /// Centralized cleanup: release every reservation the current task holds,
    /// drop anything carried, and go idle. Safe to call in any state.
    fn abandon_task(&mut self, i: usize) {
        match self.dwarves[i].task.clone() {
            Task::Mine { target, .. } => {
                if let Some(des) = self.designations.get_mut(&target) {
                    des.assigned = false;
                    des.retry_at = self.clock.tick + RETRY_DELAY;
                }
            }
            Task::Haul { item, .. } | Task::Eat { item, .. } | Task::Drink { item, .. } => {
                if self.items[item].reserved_by == Some(i) {
                    self.items[item].reserved_by = None;
                }
            }
            Task::Plant { tile, seed, .. } => {
                if self.items[seed].reserved_by == Some(i) {
                    self.items[seed].reserved_by = None;
                }
                if let Some(f) = self.farms.get_mut(&tile) {
                    f.reserved = false;
                }
            }
            Task::Harvest { tile, .. } => {
                if let Some(f) = self.farms.get_mut(&tile) {
                    f.reserved = false;
                }
            }
            Task::Craft { input, .. } => {
                if self.items[input].reserved_by == Some(i) {
                    self.items[input].reserved_by = None;
                }
            }
            Task::Idle { .. } | Task::Sleep { .. } => {}
        }
        self.drop_carried(i);
        self.dwarves[i].task = Task::Idle { wander_cd: 5 };
    }

    fn drop_carried(&mut self, i: usize) {
        let pos = self.dwarves[i].pos;
        for item in &mut self.items {
            if item.state == (ItemState::Carried { by: i }) {
                item.pos = pos;
                item.state = ItemState::OnGround;
            }
        }
    }
}

fn new_dwarf(rng: &mut ChaCha8Rng, pos: Pos) -> Dwarf {
    Dwarf {
        name: names::dwarf_name(rng),
        pos,
        alive: true,
        hunger: rng.gen_range(0.0..20.0),
        thirst: rng.gen_range(0.0..20.0),
        fatigue: rng.gen_range(0.0..30.0),
        happiness: 50.0,
        thoughts: Vec::new(),
        skills: BTreeMap::new(),
        task: Task::Idle { wander_cd: rng.gen_range(20..80) },
        move_cd: 0,
        starving_since: None,
        dehydrated_since: None,
    }
}

// ------------------------------------------------------------------- saves

const SAVE_MAGIC: u32 = 0x444B_5331; // "DKS1"
const SAVE_VERSION: u32 = 4;

#[derive(Serialize)]
struct SaveOut<'a> {
    magic: u32,
    version: u32,
    material_ids: Vec<String>,
    plant_ids: Vec<String>,
    sim: &'a Sim,
}

#[derive(Deserialize)]
struct SaveIn {
    magic: u32,
    version: u32,
    material_ids: Vec<String>,
    plant_ids: Vec<String>,
    sim: Sim,
}

pub fn save_sim(sim: &Sim, path: &FsPath, raws: &Raws) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let out = SaveOut {
        magic: SAVE_MAGIC,
        version: SAVE_VERSION,
        material_ids: raws.materials.id_manifest(),
        plant_ids: raws.plants.id_manifest(),
        sim,
    };
    let file = std::fs::File::create(path)
        .with_context(|| format!("creating {}", path.display()))?;
    bincode::serialize_into(std::io::BufWriter::new(file), &out)?;
    Ok(())
}

pub fn load_sim(path: &FsPath, raws: &Raws) -> Result<Sim> {
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
    sim.map.remap_materials(&save.material_ids, &raws.materials)?;

    let mat_remap = dk_world::build_remap(&save.material_ids, &raws.materials)?;
    let plant_remap: Vec<u16> = save
        .plant_ids
        .iter()
        .map(|id| {
            raws.plants
                .index_of(id)
                .with_context(|| format!("save uses plant '{id}' missing from current raws"))
        })
        .collect::<Result<_>>()?;
    let remap_one = |table: &[u16], v: u16, what: &str| -> Result<u16> {
        anyhow::ensure!((v as usize) < table.len(), "corrupt save: {what} index {v} out of range");
        Ok(table[v as usize])
    };
    for item in &mut sim.items {
        item.stuff = match item.kind {
            ItemKind::Boulder => remap_one(&mat_remap, item.stuff, "material")?,
            _ => remap_one(&plant_remap, item.stuff, "plant")?,
        };
    }
    for farm in sim.farms.values_mut() {
        farm.crop = remap_one(&plant_remap, farm.crop, "plant")?;
    }
    sim.rebuild_caches();
    Ok(sim)
}
