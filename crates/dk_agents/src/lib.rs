//! The living simulation: dwarves, needs, designations, jobs, farming,
//! workshops, hauling, happiness.
//!
//! Engine-agnostic and fully deterministic — `Sim::step()` advances one fixed
//! tick, so the whole game loop can run (and be tested) headlessly.

use anyhow::{Context, Result};
use dk_core::{Calendar, DAYS_PER_SEASON, TICKS_PER_DAY};
use dk_raws::{MaterialCategory, Raws};
use dk_sim::WaterSim;
use dk_world::path::{self, Pos, Regions};
use dk_world::{Map, Tile, TileShape, NO_MATERIAL};
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
/// Ticks between melee swings.
pub const ATTACK_COOLDOWN: u8 = 40;
/// Ticks fully submerged before drowning kills.
pub const BREATH_TICKS: f32 = 240.0;
/// Region rebuilds are throttled to once per this many ticks.
pub const REGION_REBUILD_INTERVAL: u64 = 20;

const HUNGER_RATE: f32 = 0.004;
const THIRST_RATE: f32 = 0.005;
const FATIGUE_RATE: f32 = 0.0015;

// ------------------------------------------------------------ designations

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DesignationKind {
    Mine,
    Stairs,
    /// Dig out the floor: this tile becomes open space, the tile below
    /// becomes a floor. Water pours into the resulting trench.
    Channel,
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
    /// Starts closed (tile becomes a Gate). Toggled by a linked lever.
    Floodgate,
    /// Pulling it toggles the floodgate at `target`.
    Lever { target: Pos },
}

impl BuildingKind {
    pub fn name(self) -> &'static str {
        match self {
            BuildingKind::Still => "Still",
            BuildingKind::Kitchen => "Kitchen",
            BuildingKind::Floodgate => "Floodgate",
            BuildingKind::Lever { .. } => "Lever",
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

// ----------------------------------------------------------------- bodies

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Faction {
    Fort,
    Hostile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PartKind {
    Head,
    Torso,
    LeftArm,
    RightArm,
    LeftLeg,
    RightLeg,
}

impl PartKind {
    pub fn name(self) -> &'static str {
        match self {
            PartKind::Head => "head",
            PartKind::Torso => "torso",
            PartKind::LeftArm => "left arm",
            PartKind::RightArm => "right arm",
            PartKind::LeftLeg => "left leg",
            PartKind::RightLeg => "right leg",
        }
    }

    fn vital(self) -> bool {
        matches!(self, PartKind::Head | PartKind::Torso)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct BodyPart {
    pub kind: PartKind,
    pub hp: i16,
    pub max_hp: i16,
    pub bleeding: u8,
}

fn default_body() -> Vec<BodyPart> {
    let part = |kind: PartKind, hp: i16| BodyPart { kind, hp, max_hp: hp, bleeding: 0 };
    vec![
        part(PartKind::Head, 20),
        part(PartKind::Torso, 40),
        part(PartKind::LeftArm, 25),
        part(PartKind::RightArm, 25),
        part(PartKind::LeftLeg, 25),
        part(PartKind::RightLeg, 25),
    ]
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
    /// Hostiles chasing a fort creature (index into dwarves).
    Fight { target: usize, path: Vec<Pos>, repath_cd: u16 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dwarf {
    pub name: String,
    pub pos: Pos,
    pub alive: bool,
    pub faction: Faction,
    pub hunger: f32,
    pub thirst: f32,
    pub fatigue: f32,
    pub happiness: f32,
    /// 0-100; bleeding drains it, running out is fatal.
    pub blood: f32,
    /// 0-100; drains while submerged in deep water.
    pub breath: f32,
    pub body: Vec<BodyPart>,
    pub thoughts: Vec<(u64, ThoughtKind)>,
    pub skills: BTreeMap<Skill, u32>,
    pub task: Task,
    move_cd: u8,
    attack_cd: u8,
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
            Task::Fight { .. } => "attacking",
        }
    }

    pub fn is_wounded(&self) -> bool {
        self.body.iter().any(|p| p.hp < p.max_hp || p.bleeding > 0)
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct SimStats {
    pub crops_harvested: u32,
    pub meals_cooked: u32,
    pub drinks_brewed: u32,
    pub migrants_arrived: u32,
    pub deaths: u32,
    pub raiders_arrived: u32,
    pub raiders_slain: u32,
    pub drownings: u32,
}

// ---------------------------------------------------------------- sieges

/// A named enemy supplied by world history: sieges are led by figures the
/// player can look up in Legends.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SiegeLeader {
    pub name: String,
    pub grudge: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SiegeRoster {
    pub civ_name: String,
    pub leaders: Vec<SiegeLeader>,
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
    pub water: WaterSim,
    /// Who attacks this fort and why — wired from world history at embark.
    pub siege_roster: Option<SiegeRoster>,
    /// Rolling event log shown in the UI (tick, message).
    pub log: Vec<(u64, String)>,
    /// World setting: do raiding parties attack this fort?
    pub invasions: bool,
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
                        dwarves.push(new_dwarf(&mut rng, pos, Faction::Fort));
                    }
                }
            }
        }
        assert!(!dwarves.is_empty(), "no walkable spawn tiles found");

        // A natural spring rises at the lowest point of the surface,
        // slowly forming a pond dwarves can channel water from.
        let mut water = WaterSim::default();
        let mut lowest: Option<(usize, i32, i32)> = None;
        for y in 0..map.height {
            for x in 0..map.width {
                if let Some(z) = map.walk_surface_z(x, y) {
                    if lowest.is_none_or(|(lz, _, _)| z < lz) {
                        lowest = Some((z, x as i32, y as i32));
                    }
                }
            }
        }
        if let Some((z, x, y)) = lowest {
            water.springs.insert(Pos::new(x, y, z as i32));
        }

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
            water,
            siege_roster: None,
            log: Vec::new(),
            invasions: true,
            rng,
            haul_retry: BTreeMap::new(),
            map_changed: true,
            regions,
        }
    }

    pub fn log_event(&mut self, msg: String) {
        self.log.push((self.clock.tick, msg));
        if self.log.len() > 60 {
            self.log.remove(0);
        }
    }

    /// Rebuild caches after deserialization.
    pub fn rebuild_caches(&mut self) {
        self.regions = Regions::new(&self.map);
        let map = &self.map;
        self.water.wake_all(map);
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
                let below_solid = self
                    .map
                    .tile_at(Pos::new(x, y, a.z - 1))
                    .is_some_and(|t| t.is_solid());
                let workable = match kind {
                    DesignationKind::Mine => tile.is_solid(),
                    DesignationKind::Stairs => {
                        tile.is_solid()
                            || matches!(tile.shape, TileShape::Floor | TileShape::Ramp)
                    }
                    DesignationKind::Channel => {
                        tile.shape.is_walkable() && below_solid
                    }
                };
                // Never dig away a tile that carries a building (an open
                // floodgate is a plain Floor, but its Building persists).
                if self.building_at(p).is_some() {
                    continue;
                }
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
        if kind == BuildingKind::Floodgate {
            // Floodgates start closed: the tile becomes a barrier.
            let tile = self.map.tile_at(pos).unwrap();
            self.map
                .set_at(pos, Tile { material: tile.material, shape: TileShape::Gate, water: 0 });
            self.displace_water(pos, tile.water);
            self.regions.dirty = true;
            self.map_changed = true;
            self.water.wake(pos);
        }
        self.buildings.push(Building { kind, pos });
        true
    }

    /// Push water squeezed out of a closing gate into neighboring tiles
    /// with capacity (any that can't fit is crushed out of existence).
    fn displace_water(&mut self, from: Pos, mut units: u8) {
        if units == 0 {
            return;
        }
        let neighbors = [
            Pos::new(from.x + 1, from.y, from.z),
            Pos::new(from.x - 1, from.y, from.z),
            Pos::new(from.x, from.y + 1, from.z),
            Pos::new(from.x, from.y - 1, from.z),
            Pos::new(from.x, from.y, from.z + 1),
        ];
        for q in neighbors {
            if units == 0 {
                break;
            }
            let Some(t) = self.map.tile_at(q) else { continue };
            if !t.holds_water() || t.water >= dk_world::MAX_WATER {
                continue;
            }
            let space = dk_world::MAX_WATER - t.water;
            let moved = units.min(space);
            self.map.set_water(q, t.water + moved);
            units -= moved;
            self.water.wake(q);
        }
    }

    /// Place a lever linked to the nearest floodgate. Returns the linked
    /// gate position if any.
    pub fn add_lever(&mut self, pos: Pos) -> Option<Pos> {
        let target = self
            .buildings
            .iter()
            .filter(|b| b.kind == BuildingKind::Floodgate)
            .min_by_key(|b| b.pos.manhattan(pos))?
            .pos;
        if self.add_building(BuildingKind::Lever { target }, pos) {
            Some(target)
        } else {
            None
        }
    }

    /// Pull the lever at `pos`: toggles its linked floodgate open/closed.
    pub fn pull_lever(&mut self, pos: Pos) -> bool {
        let Some(target) = self.buildings.iter().find_map(|b| match b.kind {
            BuildingKind::Lever { target } if b.pos == pos => Some(target),
            _ => None,
        }) else {
            return false;
        };
        self.toggle_floodgate(target)
    }

    pub fn toggle_floodgate(&mut self, pos: Pos) -> bool {
        let Some(tile) = self.map.tile_at(pos) else { return false };
        let new_shape = match tile.shape {
            TileShape::Gate => TileShape::Floor,
            TileShape::Floor => TileShape::Gate,
            _ => return false,
        };
        // Closing squeezes standing water out into the neighbors; a Gate
        // tile is skipped by the water CA, so it must never hold any.
        let kept_water = if new_shape == TileShape::Gate { 0 } else { tile.water };
        self.map
            .set_at(pos, Tile { material: tile.material, shape: new_shape, water: kept_water });
        if new_shape == TileShape::Gate {
            self.displace_water(pos, tile.water);
        }
        self.regions.dirty = true;
        self.map_changed = true;
        self.water.wake(pos);
        let state = if new_shape == TileShape::Gate { "closed" } else { "opened" };
        self.log_event(format!("The floodgate at ({}, {}) {state}.", pos.x, pos.y));
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
            let blocked = (y..=y1).any(|yy| {
                (x..=x1).any(|xx| {
                    let p = Pos::new(xx, yy, z);
                    self.farms.contains_key(&p) || self.building_at(p).is_some()
                })
            });
            if !overlaps && !blocked {
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

    /// Living fort citizens (hostiles excluded).
    pub fn alive_dwarves(&self) -> usize {
        self.dwarves
            .iter()
            .filter(|d| d.alive && d.faction == Faction::Fort)
            .count()
    }

    pub fn alive_hostiles(&self) -> usize {
        self.dwarves
            .iter()
            .filter(|d| d.alive && d.faction == Faction::Hostile)
            .count()
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

        // Water first: it changes what is walkable this tick.
        {
            let map = &mut self.map;
            if self.water.step(map) {
                self.regions.dirty = true;
                self.map_changed = true;
            }
        }
        // Region rebuilds are throttled; A* remains the authority in between.
        if self.regions.dirty && self.clock.tick % REGION_REBUILD_INTERVAL == 0 {
            self.regions.rebuild(&self.map);
        }

        self.grow_farms(raws);
        if self.clock.tick % ASSIGN_INTERVAL == 0 {
            self.assign_jobs(raws);
        }
        for i in 0..self.dwarves.len() {
            if self.dwarves[i].alive {
                match self.dwarves[i].faction {
                    Faction::Fort => self.update_dwarf(i, raws),
                    Faction::Hostile => self.update_hostile(i),
                }
            }
        }
        // Season boundary: migrants and (later years) raiders.
        let season_ticks = TICKS_PER_DAY * DAYS_PER_SEASON;
        if self.clock.tick % season_ticks == 0 && self.clock.tick > 0 {
            self.maybe_migrants(raws);
            let seasons_elapsed = self.clock.tick / season_ticks;
            // Cap active hostiles so stuck raiders don't accumulate season
            // over season into an unbounded horde.
            if self.invasions && seasons_elapsed >= 2 && self.alive_hostiles() < 8 {
                let wealth = self.items.iter().filter(|i| i.active()).count();
                let n = (1 + wealth / 150).min(5);
                self.spawn_raiders(n);
            }
        }
    }

    /// Spawn a single raider at an exact position (tests/scenarios).
    pub fn spawn_raider_at(&mut self, pos: Pos) {
        let mut r = new_dwarf(&mut self.rng, pos, Faction::Hostile);
        r.name = format!("raider {}", names::dwarf_name(&mut self.rng));
        self.dwarves.push(r);
        self.stats.raiders_arrived += 1;
    }

    /// Spawn a raiding party at the map edge. Public for tests/scenarios.
    pub fn spawn_raiders(&mut self, count: usize) {
        let mut spawned = 0;
        'outer: for y in 1..self.map.height - 1 {
            for x in [1usize, self.map.width - 2] {
                if spawned >= count {
                    break 'outer;
                }
                if let Some(z) = self.map.walk_surface_z(x, y) {
                    let pos = Pos::new(x as i32, y as i32, z as i32);
                    if self.dwarves.iter().any(|d| d.alive && d.pos == pos) {
                        continue;
                    }
                    let mut r = new_dwarf(&mut self.rng, pos, Faction::Hostile);
                    r.name = format!("raider {}", names::dwarf_name(&mut self.rng));
                    self.dwarves.push(r);
                    self.stats.raiders_arrived += 1;
                    spawned += 1;
                }
            }
        }
        if spawned > 0 {
            // The party is led by a figure from world history when we have
            // one — their grudge is the reason this is happening. Leaders
            // rotate so named sieges continue for the fort's whole life,
            // but the slain stay dead.
            let dead: Vec<String> = self
                .dwarves
                .iter()
                .filter(|d| !d.alive && d.faction == Faction::Hostile)
                .map(|d| d.name.clone())
                .collect();
            let led = self.siege_roster.as_mut().and_then(|r| {
                let idx = r.leaders.iter().position(|l| !dead.contains(&l.name))?;
                let leader = r.leaders.remove(idx);
                r.leaders.push(leader.clone());
                Some((r.civ_name.clone(), leader))
            });
            match led {
                Some((civ, leader)) => {
                    // The last-spawned raider bears the historical name.
                    if let Some(d) = self
                        .dwarves
                        .iter_mut()
                        .rev()
                        .find(|d| d.alive && d.faction == Faction::Hostile)
                    {
                        d.name = leader.name.clone();
                    }
                    self.log_event(format!(
                        "{} of {} leads a raiding party of {spawned} — they {}!",
                        leader.name, civ, leader.grudge
                    ));
                }
                None => {
                    self.log_event(format!("A raiding party of {spawned} has arrived!"));
                }
            }
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
        let drink = self.count_kind(ItemKind::Drink);
        if food < alive || drink < alive {
            return; // word gets out that the fort is starving (or dry)
        }
        let Some(anchor) = self
            .dwarves
            .iter()
            .find(|d| d.alive && d.faction == Faction::Fort)
            .map(|d| d.pos)
        else {
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
                        let mut d = new_dwarf(&mut self.rng, pos, Faction::Fort);
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
        // Count batches already in flight so a 1-item deficit doesn't send
        // every idle dwarf to the workshops at once.
        let mut pending_brews = 0usize;
        let mut pending_cooks = 0usize;
        for d in &self.dwarves {
            if d.alive {
                match d.task {
                    Task::Craft { kind: CraftKind::Brew, .. } => pending_brews += 1,
                    Task::Craft { kind: CraftKind::Cook, .. } => pending_cooks += 1,
                    _ => {}
                }
            }
        }
        for i in 0..self.dwarves.len() {
            let d = &self.dwarves[i];
            if d.alive && d.faction == Faction::Fort && d.is_idle() {
                let want_drinks =
                    self.count_kind(ItemKind::Drink) + pending_brews * BATCH < alive * 3;
                let want_meals =
                    self.count_kind(ItemKind::Meal) + pending_cooks * BATCH < alive * 3;
                match self.assign_one(i, raws, want_drinks, want_meals) {
                    Some(CraftKind::Brew) => pending_brews += 1,
                    Some(CraftKind::Cook) => pending_cooks += 1,
                    None => {}
                }
            }
        }
    }

    fn assign_one(
        &mut self,
        i: usize,
        raws: &Raws,
        want_drinks: bool,
        want_meals: bool,
    ) -> Option<CraftKind> {
        let dwarf_pos = self.dwarves[i].pos;
        let my_region = self.regions.id(dwarf_pos);
        if my_region == 0 {
            return None;
        }
        let tick = self.clock.tick;

        // --- Needs come first.
        if self.dwarves[i].hunger >= NEED_AT {
            let hunger = self.dwarves[i].hunger;
            if let Some(item) = self.nearest_food(dwarf_pos, my_region, hunger) {
                if self.start_goto_item(i, item, |it, p| Task::Eat { item: it, path: p }) {
                    return None;
                }
            }
        }
        if self.dwarves[i].thirst >= NEED_AT {
            if let Some(item) = self.nearest_kind(ItemKind::Drink, dwarf_pos, my_region) {
                if self.start_goto_item(i, item, |it, p| Task::Drink { item: it, path: p }) {
                    return None;
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
                // Channeling from the doomed tile itself would drop the digger
                // into the trench.
                .filter(|&&w| !(des.kind == DesignationKind::Channel && w == target))
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
        let Some((_, cand)) = best else { return None };
        let mut started_craft = None;
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
                    started_craft = Some(kind);
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
        started_craft
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

    /// Meals first. Raw crops are a joyless fallback — and if a kitchen
    /// exists, only a desperate dwarf raids the larder (crops turn into
    /// three meals each when cooked).
    fn nearest_food(&self, near: Pos, region: u32, hunger: f32) -> Option<usize> {
        if let Some(meal) = self.nearest_kind(ItemKind::Meal, near, region) {
            return Some(meal);
        }
        let has_kitchen = self
            .buildings
            .iter()
            .any(|b| b.kind == BuildingKind::Kitchen);
        if !has_kitchen || hunger >= 85.0 {
            self.nearest_kind(ItemKind::Crop, near, region)
        } else {
            None
        }
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
        // Self-defense: fight any adjacent hostile before doing anything else.
        if let Some(enemy) = self.adjacent_enemy(i) {
            self.melee(i, enemy);
            return;
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
                    // Only resting items block a cell — creatures carrying
                    // things through the stockpile don't occupy it.
                    let taken = self.items.iter().enumerate().any(|(j, it)| {
                        j != item
                            && it.active()
                            && it.pos == here
                            && matches!(it.state, ItemState::OnGround | ItemState::Stored { .. })
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
            // Fort citizens never chase (hostiles use update_hostile);
            // clear it if it somehow appears.
            Task::Fight { .. } => {
                self.dwarves[i].task = Task::Idle { wander_cd: 5 };
            }
        }
    }

    /// Raider AI: chase the nearest fort creature; swing when adjacent;
    /// approach greedily when no path exists (e.g. walls or moats).
    fn update_hostile(&mut self, i: usize) {
        self.tick_vitals(i);
        if !self.dwarves[i].alive {
            return;
        }
        if let Some(enemy) = self.adjacent_enemy(i) {
            self.melee(i, enemy);
            return;
        }
        let my_pos = self.dwarves[i].pos;
        let target = self
            .dwarves
            .iter()
            .enumerate()
            .filter(|(_, d)| d.alive && d.faction == Faction::Fort)
            .min_by_key(|(_, d)| d.pos.manhattan(my_pos))
            .map(|(j, _)| j);
        let Some(target) = target else {
            // Nobody left to fight; mill about.
            self.dwarves[i].task = Task::Idle { wander_cd: 50 };
            return;
        };

        let (mut path, mut repath_cd) = match self.dwarves[i].task.clone() {
            Task::Fight { target: t, path, repath_cd } if t == target => (path, repath_cd),
            _ => (Vec::new(), 0),
        };
        if repath_cd == 0 && path.is_empty() {
            path = path::astar(&self.map, my_pos, self.dwarves[target].pos, 20_000)
                .unwrap_or_default();
            repath_cd = 120;
        }
        repath_cd = repath_cd.saturating_sub(1);

        if !path.is_empty() {
            if !self.step_along(i, &mut path) {
                path.clear();
            }
        } else if self.dwarves[i].move_cd == 0 {
            // No path (walls, moats): press greedily toward the target, and
            // when pinned against a wall face, prowl to a random neighbor so
            // the raider keeps probing instead of freezing forever.
            let goal = self.dwarves[target].pos;
            let mut opts = Vec::with_capacity(8);
            path::neighbors(&self.map, my_pos, &mut opts);
            let step = match opts.iter().min_by_key(|q| q.manhattan(goal)) {
                Some(&next) if next.manhattan(goal) < my_pos.manhattan(goal) => Some(next),
                _ if !opts.is_empty() && self.rng.gen_ratio(1, 8) => {
                    Some(opts[self.rng.gen_range(0..opts.len())])
                }
                _ => None,
            };
            if let Some(next) = step {
                self.dwarves[i].pos = next;
                self.dwarves[i].move_cd = WALK_COOLDOWN;
            }
        } else {
            self.dwarves[i].move_cd -= 1;
        }
        self.dwarves[i].task = Task::Fight { target, path, repath_cd };
    }

    fn adjacent_enemy(&self, i: usize) -> Option<usize> {
        let me = &self.dwarves[i];
        // Same z-level only — a full-3D distance would let creatures brawl
        // through solid floors (movement between z-levels is always an
        // explicit stair/ramp edge).
        self.dwarves
            .iter()
            .enumerate()
            .filter(|(_, d)| d.alive && d.faction != me.faction && d.pos.z == me.pos.z)
            .find(|(_, d)| d.pos.x.abs_diff(me.pos.x) + d.pos.y.abs_diff(me.pos.y) <= 1)
            .map(|(j, _)| j)
    }

    /// One melee swing, if off cooldown: pick a body part, deal damage,
    /// start bleeding, log it, and kill on vital destruction.
    fn melee(&mut self, attacker: usize, defender: usize) {
        if self.dwarves[attacker].attack_cd > 0 {
            self.dwarves[attacker].attack_cd -= 1;
            return;
        }
        self.dwarves[attacker].attack_cd = ATTACK_COOLDOWN;

        // Torso is the biggest target; head the deadliest.
        let roll = self.rng.gen_range(0..8usize);
        let part_kind = match roll {
            0 => PartKind::Head,
            1 | 2 | 3 => PartKind::Torso,
            4 => PartKind::LeftArm,
            5 => PartKind::RightArm,
            6 => PartKind::LeftLeg,
            _ => PartKind::RightLeg,
        };
        let dmg = self.rng.gen_range(8..=20) as i16;
        let bleed = self.rng.gen_range(1..=3) as u8;

        let att_name = self.dwarves[attacker].name.clone();
        let def_name = self.dwarves[defender].name.clone();
        let d = &mut self.dwarves[defender];
        let Some(part) = d.body.iter_mut().find(|pt| pt.kind == part_kind) else { return };
        part.hp -= dmg;
        part.bleeding = part.bleeding.saturating_add(bleed);
        let destroyed = part.hp <= 0;
        let vital = part.kind.vital();
        self.log_event(format!(
            "{att_name} strikes {def_name} in the {}!",
            part_kind.name()
        ));
        if destroyed && vital {
            self.log_event(format!("{def_name} falls dead!"));
            self.kill_dwarf(defender);
            if self.dwarves[defender].faction == Faction::Hostile {
                self.stats.raiders_slain += 1;
            }
        }
    }

    /// Blood, breath, bleeding, and rest-healing — applies to every faction.
    fn tick_vitals(&mut self, i: usize) {
        let tick = self.clock.tick;
        let pos = self.dwarves[i].pos;
        let submerged = self.map.water_at(pos) >= 5;
        let name = self.dwarves[i].name.clone();
        let d = &mut self.dwarves[i];

        if submerged {
            d.breath -= 100.0 / BREATH_TICKS;
        } else {
            d.breath = (d.breath + 2.0).min(100.0);
        }
        let drowned = d.breath <= 0.0;

        let bleeding: u32 = d.body.iter().map(|p| p.bleeding as u32).sum();
        if bleeding > 0 {
            d.blood -= bleeding as f32 * 0.01;
        } else {
            d.blood = (d.blood + 0.002).min(100.0);
        }
        let resting = matches!(d.task, Task::Sleep { .. });
        let decay_every = if resting { 100 } else { 400 };
        if tick % decay_every == 0 {
            for p in &mut d.body {
                p.bleeding = p.bleeding.saturating_sub(1);
            }
        }
        if resting && tick % 200 == 0 {
            for p in &mut d.body {
                if p.hp < p.max_hp {
                    p.hp += 1;
                }
            }
        }
        let bled_out = d.blood <= 0.0;

        if drowned {
            // HUD semantics: drownings counts raiders killed by floods.
            if self.dwarves[i].faction == Faction::Hostile {
                self.stats.drownings += 1;
            }
            self.log_event(format!("{name} has drowned."));
            self.kill_dwarf(i);
        } else if bled_out {
            // Bleed-out is still a kill for the scoreboard.
            if self.dwarves[i].faction == Faction::Hostile {
                self.stats.raiders_slain += 1;
            }
            self.log_event(format!("{name} has bled out."));
            self.kill_dwarf(i);
        }
    }

    /// Needs tick + hunger/thirst thoughts + death countdowns.
    fn tick_needs(&mut self, i: usize) {
        self.tick_vitals(i);
        if !self.dwarves[i].alive {
            return;
        }
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
        // `deaths` means fort citizens lost; raider kills have their own
        // counters at the call sites.
        if self.dwarves[i].faction == Faction::Fort {
            self.stats.deaths += 1;
        }
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
        let mut boulder_from = tile;
        match des.kind {
            DesignationKind::Mine => {
                self.map.set_at(
                    target,
                    Tile { material: tile.material, shape: TileShape::Floor, water: tile.water },
                );
            }
            DesignationKind::Stairs => {
                self.map.set_at(
                    target,
                    Tile { material: tile.material, shape: TileShape::Stairs, water: tile.water },
                );
            }
            DesignationKind::Channel => {
                // The floor is dug away: open space here, floor below.
                self.map.set_at(
                    target,
                    Tile { material: NO_MATERIAL, shape: TileShape::Empty, water: tile.water },
                );
                let below = Pos::new(target.x, target.y, target.z - 1);
                if let Some(bt) = self.map.tile_at(below) {
                    if bt.is_solid() {
                        boulder_from = bt;
                        self.map.set_at(
                            below,
                            Tile { material: bt.material, shape: TileShape::Floor, water: bt.water },
                        );
                    }
                }
                self.water.wake(below);
            }
        }
        self.regions.dirty = true;
        self.map_changed = true;
        self.water.wake(target);

        if boulder_from.is_solid()
            && boulder_from.material != NO_MATERIAL
            && raws.materials.get(boulder_from.material).category != MaterialCategory::Soil
        {
            // Channeled boulders land in the trench below.
            let drop_at = if des.kind == DesignationKind::Channel {
                Pos::new(target.x, target.y, target.z - 1)
            } else {
                target
            };
            self.spawn_item(ItemKind::Boulder, boulder_from.material, drop_at);
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
            Task::Idle { .. } | Task::Sleep { .. } | Task::Fight { .. } => {}
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

fn new_dwarf(rng: &mut ChaCha8Rng, pos: Pos, faction: Faction) -> Dwarf {
    Dwarf {
        name: names::dwarf_name(rng),
        pos,
        alive: true,
        faction,
        hunger: rng.gen_range(0.0..20.0),
        thirst: rng.gen_range(0.0..20.0),
        fatigue: rng.gen_range(0.0..30.0),
        happiness: 50.0,
        blood: 100.0,
        breath: 100.0,
        body: default_body(),
        thoughts: Vec::new(),
        skills: BTreeMap::new(),
        task: Task::Idle { wander_cd: rng.gen_range(20..80) },
        move_cd: 0,
        attack_cd: 0,
        starving_since: None,
        dehydrated_since: None,
    }
}

// ------------------------------------------------------------------- saves

const SAVE_MAGIC: u32 = 0x444B_5331; // "DKS1"
const SAVE_VERSION: u32 = 6;

#[derive(Serialize)]
struct SaveOut<'a> {
    magic: u32,
    version: u32,
    material_ids: Vec<String>,
    plant_ids: Vec<String>,
    sim: &'a Sim,
}

// Read as header-then-body so version mismatches produce a clear error
// instead of a bincode failure mid-struct. bincode serializes struct fields
// sequentially, so this matches SaveOut's layout exactly.
type SaveBody = (Vec<String>, Vec<String>, Sim);

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
    let mut reader = std::io::BufReader::new(file);
    let (magic, version): (u32, u32) = bincode::deserialize_from(&mut reader)
        .with_context(|| format!("reading save header of {}", path.display()))?;
    anyhow::ensure!(magic == SAVE_MAGIC, "not a Dwarf Kingdom save file");
    anyhow::ensure!(
        version == SAVE_VERSION,
        "save version {version} unsupported (expected {SAVE_VERSION}) — this save \
         is from another game version"
    );
    let (material_ids, plant_ids, sim): SaveBody = bincode::deserialize_from(&mut reader)
        .with_context(|| format!("deserializing {}", path.display()))?;
    let mut sim = sim;
    sim.map.validate()?;
    sim.map.remap_materials(&material_ids, &raws.materials)?;

    let mat_remap = dk_world::build_remap(&material_ids, &raws.materials)?;
    let plant_remap: Vec<u16> = plant_ids
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
