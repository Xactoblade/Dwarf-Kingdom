//! The living simulation: dwarves, needs, designations, jobs, farming,
//! workshops, hauling, happiness.
//!
//! Engine-agnostic and fully deterministic — `Sim::step()` advances one fixed
//! tick, so the whole game loop can run (and be tested) headlessly.

use anyhow::{Context, Result};
use dk_core::{Calendar, DAYS_PER_SEASON, TICKS_PER_DAY};
use dk_raws::{MaterialCategory, Raws};
use dk_sim::{FluidSim, WaterSim};
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
/// Ticks of work to complete a strange mood's artifact.
pub const MOOD_WORK: u16 = 300;
/// Ticks a tantrum or sulk episode lasts.
pub const EPISODE_TICKS: u16 = 800;
/// Relationship level at which two dwarves count as friends.
pub const FRIEND_AT: i32 = 30;
/// Days an unburied fort corpse waits before its ghost rises.
pub const GHOST_AFTER_DAYS: u64 = 8;
/// Ticks for an animal to reach adulthood (breeding & butchering age).
pub const ADULT_TICKS: u64 = 30 * TICKS_PER_DAY;
/// Gestation once bred.
pub const GESTATION_TICKS: u64 = 20 * TICKS_PER_DAY;
/// Cooldown after birth before an adult can breed again.
pub const BREED_COOLDOWN: u64 = 15 * TICKS_PER_DAY;
/// Ticks of work to butcher a marked animal.
pub const BUTCHER_WORK: u16 = 80;
/// A pasture won't overbreed past this many head.
pub const HERD_CAP: usize = 12;
/// Days between an adult sheep growing a shearable coat.
pub const WOOL_INTERVAL: u64 = 12 * TICKS_PER_DAY;
/// Stress at which a dwarf seeks the tavern to unwind.
pub const TAVERN_STRESS_AT: f32 = 40.0;
/// Ticks spent relaxing at the tavern.
pub const RELAX_TICKS: u16 = 300;
/// Ticks of work to land a catch while fishing.
pub const FISH_WORK: u16 = 220;

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
    /// A strange mood's masterwork. `stuff` = material index.
    Artifact,
    /// A dead citizen, awaiting burial. `stuff` unused; `name` names them.
    Corpse,
    /// A decorative stone craft — a trade good. `stuff` = material index.
    Craft,
    /// Raw wool sheared from sheep. `stuff` unused.
    Wool,
    /// Woven cloth — a trade good. `stuff` unused.
    Cloth,
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
    /// Material index for boulders/artifacts, plant index otherwise.
    pub stuff: u16,
    /// Artifacts bear generated names.
    pub name: Option<String>,
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
    /// Turns stone boulders into decorative trade goods.
    Craftsdwarf,
    /// Weaves raw wool into cloth.
    Loom,
    /// A resting place. Burying a corpse here lays its ghost to rest.
    Tomb,
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
            BuildingKind::Craftsdwarf => "Craftsdwarf's Workshop",
            BuildingKind::Loom => "Loom",
            BuildingKind::Tomb => "Tomb",
            BuildingKind::Floodgate => "Floodgate",
            BuildingKind::Lever { .. } => "Lever",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Building {
    pub kind: BuildingKind,
    pub pos: Pos,
    /// Tombs: whether someone rests here already.
    pub occupied: bool,
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

/// An axis-aligned rectangle of tiles on one z-level. Used for stockpiles
/// and pastures alike.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rect {
    pub z: i32,
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl Rect {
    pub fn contains(&self, p: Pos) -> bool {
        p.z == self.z && p.x >= self.x0 && p.x <= self.x1 && p.y >= self.y0 && p.y <= self.y1
    }

    pub fn cells(&self) -> impl Iterator<Item = Pos> + '_ {
        let (x0, x1, y0, y1, z) = (self.x0, self.x1, self.y0, self.y1, self.z);
        (y0..=y1).flat_map(move |y| (x0..=x1).map(move |x| Pos::new(x, y, z)))
    }

    /// A deterministic "center" cell for herding animals toward.
    pub fn center(&self) -> Pos {
        Pos::new((self.x0 + self.x1) / 2, (self.y0 + self.y1) / 2, self.z)
    }
}

/// Kept for API stability; stockpiles are `Rect`s.
pub type Stockpile = Rect;

// ----------------------------------------------------------------- animals

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AnimalKind {
    Cow,
    Sheep,
}

impl AnimalKind {
    pub fn name(self) -> &'static str {
        match self {
            AnimalKind::Cow => "cow",
            AnimalKind::Sheep => "sheep",
        }
    }

    /// Meals yielded when butchered as an adult.
    pub fn meat_yield(self) -> usize {
        match self {
            AnimalKind::Cow => 5,
            AnimalKind::Sheep => 3,
        }
    }
}

/// A grazing beast. Simpler than a dwarf: it wanders its pasture, matures,
/// breeds, and is eventually butchered for meat.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Animal {
    pub kind: AnimalKind,
    pub pos: Pos,
    pub alive: bool,
    /// Ticks lived; adulthood (and breeding/butchering eligibility) at
    /// `ADULT_TICKS`.
    pub age: u64,
    /// Set on the pregnant parent only; a calf is born when it reaches 0.
    pub gestation: Option<u64>,
    /// Ticks until this animal can breed again (no birth — just a rest).
    pub breed_cd: u64,
    /// Ticks until this animal (if a sheep) grows a shearable coat again.
    pub wool_cd: u64,
    /// A herder has been told to slaughter this animal.
    pub marked: bool,
    /// Dwarf index that has claimed this animal for butchering.
    pub reserved_by: Option<usize>,
    move_cd: u8,
}

impl Animal {
    pub fn is_adult(&self) -> bool {
        self.age >= ADULT_TICKS
    }
}

// ----------------------------------------------------------------- bodies

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Faction {
    Fort,
    Hostile,
    /// Caravan traders and other guests: protected, not commanded.
    Visitor,
}

impl Faction {
    /// Who fights whom: hostiles against everyone else, nobody else
    /// starts anything.
    pub fn hostile_to(self, other: Faction) -> bool {
        (self == Faction::Hostile) != (other == Faction::Hostile)
    }
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

// ------------------------------------------------------------- personality

/// A dwarf's disposition, rolled at creation. All facets are 0-100.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Personality {
    /// High: shrugs off bad news. Low: every misfortune cuts deep.
    pub cheer: f32,
    /// High: seeks work quickly, works faster. Low: dawdles.
    pub diligence: f32,
    /// High: chats often, makes friends fast.
    pub social: f32,
    /// High: steady in a fight. Low: frightened by sieges.
    pub bravery: f32,
}

impl Personality {
    fn roll(rng: &mut ChaCha8Rng) -> Self {
        let mut f = || rng.gen_range(5.0f32..95.0);
        Personality { cheer: f(), diligence: f(), social: f(), bravery: f() }
    }

    /// Words a player would use for this dwarf.
    pub fn descriptors(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.cheer > 70.0 { out.push("sunny") } else if self.cheer < 30.0 { out.push("gloomy") }
        if self.diligence > 70.0 { out.push("industrious") } else if self.diligence < 30.0 { out.push("idle-handed") }
        if self.social > 70.0 { out.push("gregarious") } else if self.social < 30.0 { out.push("solitary") }
        if self.bravery > 70.0 { out.push("fearless") } else if self.bravery < 30.0 { out.push("skittish") }
        if out.is_empty() { out.push("unremarkable") }
        out
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
    PleasantChat,
    FriendDied,
    DisturbedByTantrum,
    ThrewTantrum,
    FellIntoGloom,
    ScaredBySiege,
    MadeArtifact,
    SawArtifact,
    BecameBaron,
    MandateMet,
    Punished,
    SawPunishment,
    Haunted,
    LaidToRest,
    RelaxedAtTavern,
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
            ThoughtKind::PleasantChat => 3.0,
            ThoughtKind::FriendDied => -18.0,
            ThoughtKind::DisturbedByTantrum => -4.0,
            ThoughtKind::ThrewTantrum => -6.0,
            ThoughtKind::FellIntoGloom => -8.0,
            ThoughtKind::ScaredBySiege => -5.0,
            ThoughtKind::MadeArtifact => 30.0,
            ThoughtKind::SawArtifact => 5.0,
            ThoughtKind::BecameBaron => 15.0,
            ThoughtKind::MandateMet => 6.0,
            ThoughtKind::Punished => -15.0,
            ThoughtKind::SawPunishment => -6.0,
            ThoughtKind::Haunted => -8.0,
            ThoughtKind::LaidToRest => 8.0,
            ThoughtKind::RelaxedAtTavern => 6.0,
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
            ThoughtKind::PleasantChat => "had a pleasant chat",
            ThoughtKind::FriendDied => "lost a dear friend",
            ThoughtKind::DisturbedByTantrum => "was disturbed by a tantrum",
            ThoughtKind::ThrewTantrum => "threw a tantrum",
            ThoughtKind::FellIntoGloom => "fell into a dark gloom",
            ThoughtKind::ScaredBySiege => "was frightened by the siege",
            ThoughtKind::MadeArtifact => "created a legendary artifact!",
            ThoughtKind::SawArtifact => "admired a legendary artifact",
            ThoughtKind::BecameBaron => "was elevated to the barony",
            ThoughtKind::MandateMet => "saw their mandate fulfilled",
            ThoughtKind::Punished => "was beaten for a failed mandate",
            ThoughtKind::SawPunishment => "watched a comrade being punished",
            ThoughtKind::Haunted => "was tormented by a restless ghost",
            ThoughtKind::LaidToRest => "took comfort in a proper burial",
            ThoughtKind::RelaxedAtTavern => "unwound at the tavern",
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
    Crafting,
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
    /// Turn a stone boulder into a decorative trade good.
    Stonecraft,
    /// Weave raw wool into cloth.
    Weave,
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
    /// Stress broke loose: storming around, frightening witnesses.
    Tantrum { remaining: u16 },
    /// Stress turned inward: unresponsive, refusing work.
    Sulk { remaining: u16 },
    /// Possessed by inspiration: fetch a boulder, claim a workshop, create.
    StrangeMood { shop: Pos, input: usize, path: Vec<Pos>, stage: FetchStage, progress: u16 },
    /// Walk to a marked animal and slaughter it for meat.
    Butcher { animal: usize, path: Vec<Pos>, progress: u16 },
    /// Unwind at the tavern: walk there, drink, socialize, shed stress.
    Relax { spot: Pos, path: Vec<Pos>, remaining: u16, drank: bool },
    /// Fish at a bank tile beside water until something bites.
    Fish { spot: Pos, path: Vec<Pos>, progress: u16 },
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
    pub personality: Personality,
    /// 0-100+; negative thoughts feed it, calm drains it. Boiling over
    /// means a tantrum or a gloom.
    pub stress: f32,
    /// Affinity toward other dwarves by index; FRIEND_AT+ means friendship.
    pub relationships: BTreeMap<usize, i32>,
    /// Favorite material and crop (raws indices) — grist for biographies.
    pub favorite_material: u16,
    pub favorite_crop: u16,
    pub artifacts_made: u32,
    pub thoughts: Vec<(u64, ThoughtKind)>,
    pub skills: BTreeMap<Skill, u32>,
    pub task: Task,
    move_cd: u8,
    attack_cd: u8,
    chat_cd: u16,
    /// Tick a fully maxed need started, for death countdowns.
    starving_since: Option<u64>,
    dehydrated_since: Option<u64>,
    /// When they died, and whether their unquiet spirit walks.
    pub died_at: Option<u64>,
    pub ghost: bool,
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
            Task::Craft { kind: CraftKind::Stonecraft, .. } => "crafting",
            Task::Craft { kind: CraftKind::Weave, .. } => "weaving",
            Task::Fight { .. } => "attacking",
            Task::Tantrum { .. } => "throwing a tantrum",
            Task::Sulk { .. } => "sulking",
            Task::StrangeMood { .. } => "in a strange mood!",
            Task::Butcher { .. } => "butchering",
            Task::Relax { .. } => "relaxing at the tavern",
            Task::Fish { .. } => "fishing",
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
    pub caravans_arrived: u32,
    pub trades_completed: u32,
    pub boulders_mined: u32,
    pub mandates_met: u32,
    pub mandates_failed: u32,
    pub animals_butchered: u32,
    pub crafts_made: u32,
    pub cloth_woven: u32,
    pub fish_caught: u32,
}

// -------------------------------------------------------------- adventure

/// One turn's worth of player intent in adventure mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerAction {
    Move(i32, i32),
    /// +1 up / -1 down through stairs.
    Climb(i32),
    Wait,
}

// ---------------------------------------------------------------- nobility

/// Population at which the fort attracts a baron.
pub const BARONY_AT: usize = 8;
/// Days a mandate runs before it is judged.
pub const MANDATE_DAYS: u64 = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MandateKind {
    CookMeals,
    BrewDrinks,
    MineBoulders,
}

impl MandateKind {
    pub fn describe(self, amount: u32) -> String {
        match self {
            MandateKind::CookMeals => format!("{amount} meals be cooked"),
            MandateKind::BrewDrinks => format!("{amount} drinks be brewed"),
            MandateKind::MineBoulders => format!("{amount} boulders be mined"),
        }
    }
}

/// A baron's demand: produce `amount` of something before `deadline`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Mandate {
    pub kind: MandateKind,
    pub amount: u32,
    pub deadline: u64,
    /// Stat value when the mandate was issued (progress = current - baseline).
    pub baseline: u32,
}

// ----------------------------------------------------------------- trade

/// A caravan buys at a margin: your offer must beat the asked value by this
/// ratio (they came a long way).
pub const TRADE_MARGIN: f32 = 1.2;
/// Ticks a caravan stays before packing up.
pub const CARAVAN_STAY: u64 = 12 * TICKS_PER_DAY;

/// A visiting merchant company and its wagon of goods.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Caravan {
    pub civ_name: String,
    /// Goods in the wagon (not on the map until bought).
    pub goods: Vec<Item>,
    pub leaves_at: u64,
    /// Dwarf indices of the escorting traders.
    pub traders: Vec<usize>,
}

/// Trade value of an item, in a common coin.
pub fn item_value(item: &Item, raws: &Raws) -> u32 {
    match item.kind {
        ItemKind::Boulder => raws.materials.get(item.stuff).value * 3,
        ItemKind::Seed => 3,
        ItemKind::Crop => 5,
        ItemKind::Meal => 8,
        ItemKind::Drink => 8,
        // Precious, but not a wagon-buying cheat: the material matters.
        ItemKind::Artifact => 50 + raws.materials.get(item.stuff).value * 5,
        // The dead are not for sale.
        ItemKind::Corpse => 0,
        // A worked craft is worth several times its raw stone.
        ItemKind::Craft => raws.materials.get(item.stuff).value * 12 + 4,
        ItemKind::Wool => 4,
        // Cloth is a fine, renewable trade good.
        ItemKind::Cloth => 18,
    }
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
    pub pastures: Vec<Rect>,
    pub taverns: Vec<Rect>,
    pub fisheries: Vec<Rect>,
    pub animals: Vec<Animal>,
    pub buildings: Vec<Building>,
    pub farms: BTreeMap<Pos, FarmTile>,
    pub designations: BTreeMap<Pos, Designation>,
    pub stats: SimStats,
    pub clock: Calendar,
    pub water: WaterSim,
    /// The second fluid: slower, hotter, considerably less forgiving.
    pub magma: FluidSim,
    /// Adventure mode: index of the player-controlled creature, if any.
    pub player: Option<usize>,
    /// Adventure quest: (target name, completed).
    pub quest: Option<(String, bool)>,
    /// Index of the spawned nemesis, so completion checks the right body.
    pub quest_target: Option<usize>,
    /// Notable feats accomplished by the player.
    pub deeds: Vec<String>,
    /// Who attacks this fort and why — wired from world history at embark.
    pub siege_roster: Option<SiegeRoster>,
    /// Friendly civ that sends caravans — wired from world history at embark.
    pub trade_partner: Option<String>,
    /// The caravan currently visiting, if any.
    pub caravan: Option<Caravan>,
    /// Killing traders has consequences: no caravans until this tick.
    pub trade_ban_until: u64,
    /// Set when a hostile (not the fort) kills a trader — the caravan
    /// scatters but the civ blames the raiders, not you.
    trader_lost_to_raiders: bool,
    /// The fort's baron (dwarf index), once population earns one.
    pub baron: Option<usize>,
    /// The baron's current demand.
    pub mandate: Option<Mandate>,
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
                        dwarves.push(new_dwarf(&mut rng, pos, Faction::Fort, raws));
                    }
                }
            }
        }
        assert!(!dwarves.is_empty(), "no walkable spawn tiles found");

        let mut magma = FluidSim::magma();
        magma.wake_all(&map);

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
            pastures: Vec::new(),
            taverns: Vec::new(),
            fisheries: Vec::new(),
            animals: Vec::new(),
            buildings: Vec::new(),
            farms: BTreeMap::new(),
            designations: BTreeMap::new(),
            stats: SimStats::default(),
            clock: Calendar::default(),
            water,
            magma,
            player: None,
            quest: None,
            quest_target: None,
            deeds: Vec::new(),
            siege_roster: None,
            trade_partner: None,
            caravan: None,
            trade_ban_until: 0,
            trader_lost_to_raiders: false,
            baron: None,
            mandate: None,
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
        self.magma.wake_all(map);
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

    /// Fence off a pasture where livestock graze and breed.
    pub fn add_pasture(&mut self, a: Pos, b: Pos) {
        assert_eq!(a.z, b.z);
        self.pastures.push(Rect {
            z: a.z,
            x0: a.x.min(b.x),
            y0: a.y.min(b.y),
            x1: a.x.max(b.x),
            y1: a.y.max(b.y),
        });
    }

    /// Designate a tavern where dwarves gather to drink and shed stress.
    pub fn add_tavern(&mut self, a: Pos, b: Pos) {
        assert_eq!(a.z, b.z);
        self.taverns.push(Rect {
            z: a.z,
            x0: a.x.min(b.x),
            y0: a.y.min(b.y),
            x1: a.x.max(b.x),
            y1: a.y.max(b.y),
        });
    }

    pub fn tavern_at(&self, p: Pos) -> bool {
        self.taverns.iter().any(|t| t.contains(p))
    }

    /// Designate a fishery over water (dwarves fish from its banks).
    pub fn add_fishery(&mut self, a: Pos, b: Pos) {
        assert_eq!(a.z, b.z);
        self.fisheries.push(Rect {
            z: a.z,
            x0: a.x.min(b.x),
            y0: a.y.min(b.y),
            x1: a.x.max(b.x),
            y1: a.y.max(b.y),
        });
    }

    pub fn fishery_at(&self, p: Pos) -> bool {
        self.fisheries.iter().any(|f| f.contains(p))
    }

    /// A walkable tile bordering water inside a fishery — where a dwarf can
    /// stand to fish.
    fn fishing_bank(&self, near: Pos, region: u32) -> Option<Pos> {
        let mut best: Option<(u32, Pos)> = None;
        for f in &self.fisheries {
            for cell in f.cells() {
                // The bank must be walkable and in our region; some adjacent
                // tile within the fishery must hold water.
                if !self.map.walkable(cell) || self.regions.id(cell) != region {
                    continue;
                }
                let has_water = [(1, 0), (-1, 0), (0, 1), (0, -1)].iter().any(|&(dx, dy)| {
                    let w = Pos::new(cell.x + dx, cell.y + dy, cell.z);
                    f.contains(w) && self.map.water_at(w) > 0
                });
                if has_water {
                    let d = cell.manhattan(near);
                    if best.is_none_or(|(bd, _)| d < bd) {
                        best = Some((d, cell));
                    }
                }
            }
        }
        best.map(|(_, p)| p)
    }

    /// Place an animal (embark stock, a caravan purchase, or a test).
    pub fn add_animal(&mut self, kind: AnimalKind, pos: Pos, adult: bool) -> usize {
        self.animals.push(Animal {
            kind,
            pos,
            alive: true,
            age: if adult { ADULT_TICKS } else { 0 },
            gestation: None,
            breed_cd: 0,
            wool_cd: 0,
            marked: false,
            reserved_by: None,
            move_cd: 0,
        });
        self.animals.len() - 1
    }

    /// Mark the nearest living adult animal to `p` for slaughter. Returns
    /// its index if one was marked.
    pub fn mark_nearest_animal(&mut self, p: Pos) -> Option<usize> {
        let idx = self
            .animals
            .iter()
            .enumerate()
            .filter(|(_, a)| a.alive && !a.marked)
            .min_by_key(|(_, a)| a.pos.manhattan(p))
            .map(|(i, _)| i)?;
        self.animals[idx].marked = true;
        let kind = self.animals[idx].kind.name();
        self.log_event(format!("A {kind} is marked for slaughter."));
        Some(idx)
    }

    pub fn animal_at(&self, p: Pos) -> Option<&Animal> {
        self.animals.iter().find(|a| a.alive && a.pos == p)
    }

    pub fn alive_animals(&self) -> usize {
        self.animals.iter().filter(|a| a.alive).count()
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
                .set_at(pos, Tile { material: tile.material, shape: TileShape::Gate, water: 0, magma: 0 });
            self.displace_water(pos, tile.water);
            self.regions.dirty = true;
            self.map_changed = true;
            self.water.wake(pos);
            self.magma.wake(pos);
        }
        self.buildings.push(Building { kind, pos, occupied: false });
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
            .set_at(pos, Tile { material: tile.material, shape: new_shape, water: kept_water, magma: 0 });
        if new_shape == TileShape::Gate {
            self.displace_water(pos, tile.water);
        }
        self.regions.dirty = true;
        self.map_changed = true;
        self.water.wake(pos);
        self.magma.wake(pos);
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

        // A breeding pair of each beast, dropped on nearby walkable ground.
        for (n, kind) in [AnimalKind::Cow, AnimalKind::Sheep].into_iter().enumerate() {
            for pair in 0..2 {
                let ox = cx - 3 + n as i32 * 2 + pair;
                if let Some(z) = self.map.walk_surface_z(ox.max(0) as usize, (cy + 4).max(0) as usize)
                {
                    self.add_animal(kind, Pos::new(ox, cy + 4, z as i32), true);
                }
            }
        }

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
                            name: None,
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

    /// Enter adventure mode: the first living fort dwarf becomes the
    /// player, and (if history supplied enemies) their nemesis spawns
    /// somewhere on the map as a quest target.
    pub fn begin_adventure(&mut self, raws: &Raws) -> Option<usize> {
        let hero = self
            .dwarves
            .iter()
            .position(|d| d.alive && d.faction == Faction::Fort)?;
        self.player = Some(hero);
        self.invasions = false; // the world stands still for a duel
        let name = self.dwarves[hero].name.clone();
        self.log_event(format!("{name} sets out on an adventure."));

        // Find a lair for the quarry BEFORE touching the roster, so an
        // unsuitable map never costs the fort a historical enemy.
        let hero_pos = self.dwarves[hero].pos;
        let mut spot = None;
        'search: for y in (1..self.map.height - 1).rev() {
            for x in (1..self.map.width - 1).rev() {
                if let Some(z) = self.map.walk_surface_z(x, y) {
                    let p = Pos::new(x as i32, y as i32, z as i32);
                    if self.regions.same_region(hero_pos, p) && p.manhattan(hero_pos) > 20 {
                        spot = Some(p);
                        break 'search;
                    }
                }
            }
        }
        if let Some(p) = spot {
            // The slain stay dead: only living leaders can be the nemesis.
            let dead: Vec<String> = self
                .dwarves
                .iter()
                .filter(|d| !d.alive && d.faction == Faction::Hostile)
                .map(|d| d.name.clone())
                .collect();
            let leader = self.siege_roster.as_mut().and_then(|r| {
                let idx = r.leaders.iter().position(|l| !dead.contains(&l.name))?;
                // Rotate, like sieges do — the figure remains in history.
                let leader = r.leaders.remove(idx);
                r.leaders.push(leader.clone());
                Some(leader)
            });
            if let Some(leader) = leader {
                self.spawn_raider_at(p, raws);
                let idx = self.dwarves.len() - 1;
                self.dwarves[idx].name = leader.name.clone();
                self.quest = Some((leader.name.clone(), false));
                self.quest_target = Some(idx);
                self.log_event(format!(
                    "Your quarry {} is near — they {}.",
                    leader.name, leader.grudge
                ));
            }
        }
        Some(hero)
    }

    /// One player turn: act, then let the world advance a few ticks.
    /// Moving into an enemy attacks it. Returns false if the player is
    /// dead or absent.
    pub fn player_step(&mut self, action: PlayerAction, raws: &Raws) -> bool {
        let Some(hero) = self.player else { return false };
        if !self.dwarves[hero].alive {
            return false;
        }
        match action {
            PlayerAction::Wait => {}
            PlayerAction::Move(dx, dy) => {
                let me = self.dwarves[hero].pos;
                // Creatures can share tiles; an enemy standing on ours gets
                // dealt with before anything else.
                let here_enemy = self.dwarves.iter().position(|d| {
                    d.alive && d.faction == Faction::Hostile && d.pos == me
                });
                if let Some(enemy) = here_enemy {
                    self.dwarves[hero].attack_cd = 0; // one swing per turn
                    self.melee(hero, enemy);
                } else {
                    // Sloped terrain: a step can land level, up a ramp, or
                    // down one — resolve like any other walker would. Both
                    // moving AND attacking are limited to tiles the hero
                    // could legally step to (no swinging through floors).
                    let candidates = [
                        Pos::new(me.x + dx, me.y + dy, me.z),
                        Pos::new(me.x + dx, me.y + dy, me.z + 1),
                        Pos::new(me.x + dx, me.y + dy, me.z - 1),
                    ];
                    let mut legal = Vec::with_capacity(8);
                    path::neighbors(&self.map, me, &mut legal);
                    let reachable: Vec<Pos> = candidates
                        .iter()
                        .copied()
                        .filter(|c| legal.contains(c))
                        .collect();
                    let enemy = self.dwarves.iter().position(|d| {
                        d.alive
                            && d.faction == Faction::Hostile
                            && reachable.contains(&d.pos)
                    });
                    if let Some(enemy) = enemy {
                        self.dwarves[hero].attack_cd = 0;
                        self.melee(hero, enemy);
                    } else if let Some(&t) = reachable.first() {
                        self.dwarves[hero].pos = t;
                        self.carry_item_along(hero);
                    }
                }
            }
            PlayerAction::Climb(dz) => {
                let me = self.dwarves[hero].pos;
                let target = Pos::new(me.x, me.y, me.z + dz);
                let mut legal = Vec::with_capacity(8);
                path::neighbors(&self.map, me, &mut legal);
                if legal.contains(&target) {
                    self.dwarves[hero].pos = target;
                    self.carry_item_along(hero);
                }
            }
        }
        // Sustenance: standing on provisions, help yourself — but only to
        // what the active need actually calls for.
        let me = self.dwarves[hero].pos;
        if self.dwarves[hero].hunger >= 50.0 {
            let snack = self.items.iter().position(|it| {
                it.pos == me
                    && self.item_takeable(it)
                    && matches!(it.kind, ItemKind::Meal | ItemKind::Crop)
            });
            if let Some(idx) = snack {
                self.items[idx].consumed = true;
                self.dwarves[hero].hunger = 0.0;
            }
        }
        if self.dwarves[hero].thirst >= 50.0 {
            let sip = self.items.iter().position(|it| {
                it.pos == me && self.item_takeable(it) && it.kind == ItemKind::Drink
            });
            if let Some(idx) = sip {
                self.items[idx].consumed = true;
                self.dwarves[hero].thirst = 0.0;
            }
        }
        // The world takes its turn.
        for _ in 0..3 {
            self.step(raws);
        }
        // Duels carry real risk: enemies engaged with the hero shake off
        // their cooldown far faster than the ambient tick rate.
        let hero_pos = self.dwarves[hero].pos;
        for d in &mut self.dwarves {
            if d.alive
                && d.faction == Faction::Hostile
                && d.pos.z == hero_pos.z
                && d.pos.x.abs_diff(hero_pos.x) + d.pos.y.abs_diff(hero_pos.y) <= 1
            {
                d.attack_cd = d.attack_cd.saturating_sub(10);
            }
        }
        // Quest bookkeeping: completion means THIS nemesis fell, not any
        // same-named corpse from an earlier siege.
        if let Some((target, done)) = self.quest.clone() {
            if !done {
                let slain = self
                    .quest_target
                    .and_then(|t| self.dwarves.get(t))
                    .is_some_and(|d| !d.alive);
                if slain {
                    self.quest = Some((target.clone(), true));
                    let hero_name = self.dwarves[hero].name.clone();
                    let deed = format!("{hero_name} slew {target} in single combat");
                    self.deeds.push(deed.clone());
                    self.log_event(format!("{deed}!"));
                }
            }
        }
        self.dwarves[hero].alive
    }

    /// Kill a creature outright (scenarios/tests — a rockfall, a cursed
    /// verse, an act of the gods).
    pub fn slay(&mut self, i: usize) {
        if self.dwarves[i].alive {
            let name = self.dwarves[i].name.clone();
            self.log_event(format!("{name} has died."));
            self.kill_dwarf(i);
        }
    }

    /// Drop a boulder on the ground (scenarios/tests).
    pub fn debug_spawn_boulder(&mut self, material: u16, pos: Pos) {
        self.spawn_item(ItemKind::Boulder, material, pos);
    }

    /// Drop a mug of drink on the ground (scenarios/tests).
    pub fn debug_spawn_drink(&mut self, pos: Pos) {
        self.spawn_item(ItemKind::Drink, 0, pos);
    }

    /// A short life story assembled from everything the sim knows about a
    /// dwarf — the "tell me about them" answer the blueprint asks for.
    pub fn biography(&self, i: usize, raws: &Raws) -> String {
        let d = &self.dwarves[i];
        let mut out = format!("{} is a {} dwarf", d.name, d.personality.descriptors().join(", "));
        out.push_str(&format!(
            ", fond of {} and of {}",
            raws.materials.get(d.favorite_material).name,
            raws.plants.get(d.favorite_crop).name
        ));
        if let Some((&friend, level)) = d
            .relationships
            .iter()
            .filter(|(_, &v)| v >= FRIEND_AT)
            .max_by_key(|(_, &v)| v)
        {
            let _ = level;
            if let Some(f) = self.dwarves.get(friend) {
                out.push_str(&format!(", and a close friend of {}", f.name));
            }
        }
        out.push('.');
        if let Some(best) = d.skills.iter().max_by_key(|(_, &xp)| xp) {
            let (skill, _) = best;
            out.push_str(&format!(" Their craft is {:?} (level {}).", skill, d.skill_level(*skill)));
        }
        if d.artifacts_made > 0 {
            out.push_str(&format!(" They created {} legendary work(s).", d.artifacts_made));
        }
        if let Some((_, worst)) = d
            .thoughts
            .iter()
            .filter(|(_, t)| t.delta() < -5.0)
            .last()
        {
            out.push_str(&format!(" Lately they {}.", worst.text()));
        }
        if !d.alive {
            out.push_str(" They are gone now, and missed.");
        }
        out
    }

    // ------------------------------------------------------------- stepping

    pub fn step(&mut self, raws: &Raws) {
        self.clock.advance();

        // Fluids first: they change what is walkable this tick. Magma is
        // slow and heavy — it moves at a quarter of water's pace.
        {
            let map = &mut self.map;
            if self.water.step(map) {
                self.regions.dirty = true;
                self.map_changed = true;
            }
            if self.clock.tick % 4 == 0 && self.magma.step(map) {
                self.regions.dirty = true;
                self.map_changed = true;
            }
        }
        self.form_obsidian(raws);
        // Region rebuilds are throttled; A* remains the authority in between.
        if self.regions.dirty && self.clock.tick % REGION_REBUILD_INTERVAL == 0 {
            self.regions.rebuild(&self.map);
        }

        self.grow_farms(raws);
        if self.clock.tick % ASSIGN_INTERVAL == 0 {
            self.assign_jobs(raws);
        }
        for i in 0..self.dwarves.len() {
            if !self.dwarves[i].alive {
                continue;
            }
            if self.player == Some(i) {
                // The player acts only on command; their body still lives.
                self.tick_needs(i);
                continue;
            }
            match self.dwarves[i].faction {
                Faction::Fort => self.update_dwarf(i, raws),
                Faction::Hostile => self.update_hostile(i),
                Faction::Visitor => self.update_visitor(i),
            }
        }
        // Caravans arrive mid-season (offset from raids and migrants).
        let season_ticks = TICKS_PER_DAY * DAYS_PER_SEASON;
        if self.clock.tick % season_ticks == season_ticks / 2 {
            self.maybe_caravan(raws);
        }
        self.tick_caravan();

        // The barony: appointments, demands, and judgments (daily check).
        if self.clock.tick % TICKS_PER_DAY == 0 && self.clock.tick > 0 {
            self.tick_nobility();
        }
        // The unquiet dead stir every other day.
        if self.clock.tick % (2 * TICKS_PER_DAY) == 0 && self.clock.tick > 0 {
            self.tick_ghosts();
        }
        // Livestock wander a little each tick; herd bookkeeping is daily.
        self.tick_animals_movement();
        if self.clock.tick % TICKS_PER_DAY == 0 && self.clock.tick > 0 {
            self.tick_animals_husbandry();
        }

        // Season boundary: migrants, moods, and (later years) raiders.
        if self.clock.tick % season_ticks == 0 && self.clock.tick > 0 {
            self.maybe_migrants(raws);
            self.maybe_strange_mood();
            let seasons_elapsed = self.clock.tick / season_ticks;
            // Cap active hostiles so stuck raiders don't accumulate season
            // over season into an unbounded horde.
            if self.invasions && seasons_elapsed >= 2 && self.alive_hostiles() < 8 {
                let wealth = self.items.iter().filter(|i| i.active()).count();
                let n = (1 + wealth / 150).min(5);
                self.spawn_raiders(n, raws);
                // Timid dwarves take the news badly.
                for i in 0..self.dwarves.len() {
                    let d = &self.dwarves[i];
                    if d.alive && d.faction == Faction::Fort && d.personality.bravery < 40.0 {
                        self.push_thought(i, ThoughtKind::ScaredBySiege);
                    }
                }
            }
        }
    }

    /// Spawn a single raider at an exact position (tests/scenarios).
    pub fn spawn_raider_at(&mut self, pos: Pos, raws: &Raws) {
        let mut r = new_dwarf(&mut self.rng, pos, Faction::Hostile, raws);
        r.name = format!("raider {}", names::dwarf_name(&mut self.rng));
        self.dwarves.push(r);
        self.stats.raiders_arrived += 1;
    }

    /// Spawn a raiding party at the map edge. Public for tests/scenarios.
    pub fn spawn_raiders(&mut self, count: usize, raws: &Raws) {
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
                    let mut r = new_dwarf(&mut self.rng, pos, Faction::Hostile, raws);
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

    /// Lay a corpse to rest in a tomb: the ghost (if risen) departs, and
    /// those who loved them find some peace.
    fn bury(&mut self, corpse: usize, tomb: usize, hauler: usize) {
        let name = self.items[corpse]
            .name
            .clone()
            .unwrap_or_else(|| "the departed".to_string());
        // The corpse's `stuff` is the dead dwarf's index (see kill_dwarf).
        let dead_idx = self.items[corpse].stuff as usize;
        self.items[corpse].consumed = true;
        self.items[corpse].reserved_by = None;
        self.buildings[tomb].occupied = true;
        // Quiet exactly that dwarf's ghost.
        let dead: Option<usize> = self
            .dwarves
            .get(dead_idx)
            .filter(|d| !d.alive && d.faction == Faction::Fort)
            .map(|_| dead_idx);
        if let Some(dd) = dead {
            if self.dwarves[dd].ghost {
                self.dwarves[dd].ghost = false;
                let gname = self.dwarves[dd].name.clone();
                self.log_event(format!("The ghost of {gname} is finally at peace."));
            }
            // Friends of the dead take comfort.
            let mourners: Vec<usize> = self
                .dwarves
                .iter()
                .enumerate()
                .filter(|(j, d)| {
                    *j != dd
                        && d.alive
                        && d.faction == Faction::Fort
                        && d.relationships.get(&dd).copied().unwrap_or(0) >= FRIEND_AT
                })
                .map(|(j, _)| j)
                .collect();
            for m in mourners {
                self.push_thought(m, ThoughtKind::LaidToRest);
            }
        }
        self.log_event(format!("{name} laid to rest in the tomb."));
        self.dwarves[hauler].task = Task::Idle { wander_cd: 5 };
    }

    /// The unquiet dead: unburied citizens rise as ghosts and torment the
    /// Livestock drift within (or toward) their pasture and age a tick.
    fn tick_animals_movement(&mut self) {
        for idx in 0..self.animals.len() {
            if !self.animals[idx].alive {
                continue;
            }
            self.animals[idx].age = self.animals[idx].age.saturating_add(1);
            if self.animals[idx].move_cd > 0 {
                self.animals[idx].move_cd -= 1;
                continue;
            }
            if !self.rng.gen_ratio(1, 30) {
                continue;
            }
            let pos = self.animals[idx].pos;
            // Which pasture, if any, is this beast assigned to (the one it
            // stands in)? If it has strayed, herd it back toward a center.
            let goal = self
                .pastures
                .iter()
                .find(|p| p.contains(pos))
                .or_else(|| self.pastures.first())
                .map(|p| p.center());
            let mut opts = Vec::with_capacity(8);
            path::neighbors(&self.map, pos, &mut opts);
            if opts.is_empty() {
                continue;
            }
            let next = match goal {
                Some(g) if !self.pastures.iter().any(|p| p.contains(pos)) => {
                    // Stray: step toward the pasture center.
                    *opts.iter().min_by_key(|q| q.manhattan(g)).unwrap()
                }
                _ => opts[self.rng.gen_range(0..opts.len())],
            };
            self.animals[idx].pos = next;
            self.animals[idx].move_cd = WALK_COOLDOWN * 2;
        }
    }

    /// Daily husbandry: gestation, birth, and breeding among pastured adults.
    fn tick_animals_husbandry(&mut self) {
        // Births first — ONLY the pregnant parent (gestation set) gives
        // birth; the mate merely carries a breed cooldown. Sheep also grow
        // wool that a shepherd can shear (a Wool item dropped at their feet).
        let mut newborns: Vec<(AnimalKind, Pos)> = Vec::new();
        let mut sheared: Vec<Pos> = Vec::new();
        for a in &mut self.animals {
            if !a.alive {
                continue;
            }
            a.breed_cd = a.breed_cd.saturating_sub(TICKS_PER_DAY);
            if a.kind == AnimalKind::Sheep && a.is_adult() {
                a.wool_cd = a.wool_cd.saturating_sub(TICKS_PER_DAY);
                if a.wool_cd == 0 {
                    sheared.push(a.pos);
                    a.wool_cd = WOOL_INTERVAL;
                }
            }
            if let Some(g) = a.gestation {
                let g = g.saturating_sub(TICKS_PER_DAY);
                if g == 0 {
                    a.gestation = None;
                    newborns.push((a.kind, a.pos));
                } else {
                    a.gestation = Some(g);
                }
            }
        }
        for (kind, pos) in newborns {
            self.add_animal(kind, pos, false);
            self.log_event(format!("A {} is born in the pasture.", kind.name()));
        }
        for pos in sheared {
            self.spawn_item(ItemKind::Wool, 0, pos);
        }

        if self.alive_animals() >= HERD_CAP {
            return; // a full pasture stops breeding
        }
        // Breeding: two adults of the same kind sharing a pasture, both free
        // to breed (not pregnant, not on cooldown). Exactly ONE conception.
        let ready = |a: &Animal| {
            a.alive && a.is_adult() && a.gestation.is_none() && a.breed_cd == 0
        };
        let n = self.animals.len();
        for i in 0..n {
            if !ready(&self.animals[i]) {
                continue;
            }
            let (kind_i, pos_i) = (self.animals[i].kind, self.animals[i].pos);
            if !self.pastures.iter().any(|p| p.contains(pos_i)) {
                continue;
            }
            let mate = (0..n).find(|&j| {
                j != i
                    && ready(&self.animals[j])
                    && self.animals[j].kind == kind_i
                    && self.animals[j].pos.manhattan(pos_i) <= 6
                    && self.pastures.iter().any(|p| p.contains(self.animals[j].pos))
            });
            if let Some(j) = mate {
                // Only the dam becomes pregnant; both rest before breeding
                // again so the herd grows one calf at a time.
                self.animals[i].gestation = Some(GESTATION_TICKS);
                self.animals[i].breed_cd = GESTATION_TICKS + BREED_COOLDOWN;
                self.animals[j].breed_cd = GESTATION_TICKS + BREED_COOLDOWN;
                break; // one conception per day keeps growth gentle
            }
        }
    }

    /// living until someone builds them a tomb.
    fn tick_ghosts(&mut self) {
        let tick = self.clock.tick;
        // Rise: any fort corpse still above ground past the grace period.
        // Corpses carry their dwarf index in `stuff`, so this is exact.
        let unburied: std::collections::BTreeSet<usize> = self
            .items
            .iter()
            .filter(|it| it.active() && it.kind == ItemKind::Corpse)
            .map(|it| it.stuff as usize)
            .collect();
        for i in 0..self.dwarves.len() {
            let d = &self.dwarves[i];
            if d.alive || d.faction != Faction::Fort || d.ghost {
                continue;
            }
            let Some(died) = d.died_at else { continue };
            if tick - died < GHOST_AFTER_DAYS * TICKS_PER_DAY {
                continue;
            }
            if !unburied.contains(&i) {
                continue; // buried (or no corpse ever) — they rest
            }
            self.dwarves[i].ghost = true;
            let name = self.dwarves[i].name.clone();
            self.log_event(format!(
                "The restless ghost of {name} rises! Bury their remains to grant them peace."
            ));
        }
        // Torment: each ghost unsettles a random living citizen every couple
        // of days.
        let ghosts: Vec<usize> = self
            .dwarves
            .iter()
            .enumerate()
            .filter(|(_, d)| d.ghost)
            .map(|(i, _)| i)
            .collect();
        for _g in ghosts {
            if !self.rng.gen_ratio(1, 2) {
                continue;
            }
            let living: Vec<usize> = self
                .dwarves
                .iter()
                .enumerate()
                .filter(|(_, d)| d.alive && d.faction == Faction::Fort)
                .map(|(i, _)| i)
                .collect();
            if living.is_empty() {
                continue;
            }
            let victim = living[self.rng.gen_range(0..living.len())];
            self.push_thought(victim, ThoughtKind::Haunted);
        }
    }

    /// Barons arrive with population, demand things, and punish failure.
    fn tick_nobility(&mut self) {
        // Appointment: the fort's happiest citizen takes the title.
        if self.baron.is_none() && self.alive_dwarves() >= BARONY_AT {
            let chosen = self
                .dwarves
                .iter()
                .enumerate()
                .filter(|(i, d)| {
                    d.alive && d.faction == Faction::Fort && self.player != Some(*i)
                })
                .max_by(|(_, a), (_, b)| {
                    a.happiness.partial_cmp(&b.happiness).unwrap_or(std::cmp::Ordering::Equal)
                })
                .map(|(i, _)| i);
            if let Some(i) = chosen {
                self.baron = Some(i);
                let name = self.dwarves[i].name.clone();
                self.push_thought(i, ThoughtKind::BecameBaron);
                self.log_event(format!(
                    "{name} has been elevated to baron of the fortress!"
                ));
            }
            return;
        }

        // A dead baron holds no court.
        if let Some(b) = self.baron {
            if !self.dwarves[b].alive {
                self.baron = None;
                self.mandate = None;
                return;
            }
        }
        let Some(baron) = self.baron else { return };

        match self.mandate {
            None => {
                // A new demand, colored by the baron's tastes.
                let kind = match self.rng.gen_range(0..3) {
                    0 => MandateKind::CookMeals,
                    1 => MandateKind::BrewDrinks,
                    _ => MandateKind::MineBoulders,
                };
                let amount = self.rng.gen_range(3..8u32);
                let baseline = match kind {
                    MandateKind::CookMeals => self.stats.meals_cooked,
                    MandateKind::BrewDrinks => self.stats.drinks_brewed,
                    MandateKind::MineBoulders => self.stats.boulders_mined,
                };
                let deadline = self.clock.tick + MANDATE_DAYS * TICKS_PER_DAY;
                self.mandate = Some(Mandate { kind, amount, deadline, baseline });
                let name = self.dwarves[baron].name.clone();
                self.log_event(format!(
                    "Baron {name} demands that {} within {MANDATE_DAYS} days!",
                    kind.describe(amount)
                ));
            }
            Some(m) => {
                let progress = match m.kind {
                    MandateKind::CookMeals => self.stats.meals_cooked - m.baseline,
                    MandateKind::BrewDrinks => self.stats.drinks_brewed - m.baseline,
                    MandateKind::MineBoulders => self.stats.boulders_mined - m.baseline,
                };
                if progress >= m.amount {
                    self.mandate = None;
                    self.stats.mandates_met += 1;
                    self.push_thought(baron, ThoughtKind::MandateMet);
                    let name = self.dwarves[baron].name.clone();
                    self.log_event(format!("Baron {name}'s mandate has been fulfilled."));
                } else if self.clock.tick >= m.deadline {
                    self.mandate = None;
                    self.stats.mandates_failed += 1;
                    self.punish_for_mandate(baron, m);
                }
            }
        }
    }

    /// Justice, of a sort: some poor soul answers for the shortfall.
    fn punish_for_mandate(&mut self, baron: usize, m: Mandate) {
        let candidates: Vec<usize> = self
            .dwarves
            .iter()
            .enumerate()
            .filter(|(i, d)| {
                *i != baron
                    && d.alive
                    && d.faction == Faction::Fort
                    && self.player != Some(*i)
            })
            .map(|(i, _)| i)
            .collect();
        let baron_name = self.dwarves[baron].name.clone();
        let Some(&culprit) = candidates
            .get(self.rng.gen_range(0..candidates.len().max(1)))
            .or(candidates.first())
        else {
            self.log_event(format!(
                "Baron {baron_name}'s mandate ({}) went unmet, but there was no one to blame.",
                m.kind.describe(m.amount)
            ));
            return;
        };
        // A beating: bruised, shamed, and stressed — but never maimed.
        let name = self.dwarves[culprit].name.clone();
        if let Some(part) = self.dwarves[culprit]
            .body
            .iter_mut()
            .find(|p| !p.kind.vital() && p.hp > 6)
        {
            part.hp -= 5;
        }
        self.push_thought(culprit, ThoughtKind::Punished);
        self.log_event(format!(
            "The mandate went unmet: {name} is beaten on baron {baron_name}'s order."
        ));
        let witnesses: Vec<usize> = self
            .dwarves
            .iter()
            .enumerate()
            .filter(|(i, d)| {
                *i != culprit && *i != baron && d.alive && d.faction == Faction::Fort
            })
            .map(|(i, _)| i)
            .collect();
        for w in witnesses {
            self.push_thought(w, ThoughtKind::SawPunishment);
        }
    }

    /// Where water met magma this tick, stone is born.
    fn form_obsidian(&mut self, raws: &Raws) {
        if self.water.contacts.is_empty() && self.magma.contacts.is_empty() {
            return;
        }
        let contacts: Vec<Pos> = self
            .water
            .contacts
            .drain(..)
            .chain(self.magma.contacts.drain(..))
            .collect();
        let obsidian = raws
            .materials
            .index_of("obsidian")
            .or_else(|| {
                raws.materials
                    .indices_in_category(MaterialCategory::Igneous)
                    .first()
                    .copied()
            })
            .unwrap_or(0);
        let mut formed = 0;
        for p in contacts {
            let Some(t) = self.map.tile_at(p) else { continue };
            // Only a genuine meeting point hardens (both fluids interacted
            // there; the contact tile holds the defender's fluid).
            if !t.holds_water() || (t.water == 0 && t.magma == 0) {
                continue;
            }
            self.map.set_at(p, Tile::solid(obsidian));
            self.water.wake(p);
            self.magma.wake(p);
            formed += 1;
        }
        if formed > 0 {
            self.regions.dirty = true;
            self.map_changed = true;
            self.log_event(format!(
                "Water meets magma with a roar of steam — {formed} tile(s) of obsidian form."
            ));
        }
    }

    /// A merchant caravan from the fort's trade partner arrives, wagons
    /// loaded with what their homeland produces.
    fn maybe_caravan(&mut self, raws: &Raws) {
        if self.caravan.is_some()
            || self.trade_partner.is_none()
            || self.clock.tick < self.trade_ban_until
            || self.alive_dwarves() == 0
        {
            return;
        }
        let civ_name = self.trade_partner.clone().unwrap();

        // Traders enter at the map edge, like everyone else.
        let mut traders = Vec::new();
        'outer: for y in 1..self.map.height - 1 {
            for x in [1usize, self.map.width - 2] {
                if traders.len() >= 2 {
                    break 'outer;
                }
                if let Some(z) = self.map.walk_surface_z(x, y) {
                    let pos = Pos::new(x as i32, y as i32, z as i32);
                    if self.dwarves.iter().any(|d| d.alive && d.pos == pos) {
                        continue;
                    }
                    let mut t = new_dwarf(&mut self.rng, pos, Faction::Visitor, raws);
                    t.name = format!("trader {}", t.name);
                    self.dwarves.push(t);
                    traders.push(self.dwarves.len() - 1);
                }
            }
        }
        if traders.is_empty() {
            return;
        }

        // The wagon: ores, seeds, and provisions from home.
        let mut goods = Vec::new();
        let ores = raws.materials.indices_in_category(MaterialCategory::Ore);
        for _ in 0..self.rng.gen_range(2..5usize) {
            if !ores.is_empty() {
                let ore = ores[self.rng.gen_range(0..ores.len())];
                goods.push(Item {
                    kind: ItemKind::Boulder,
                    stuff: ore,
                    name: None,
                    pos: Pos::new(0, 0, 0),
                    state: ItemState::OnGround,
                    reserved_by: None,
                    consumed: false,
                });
            }
        }
        for _ in 0..self.rng.gen_range(3..7usize) {
            let plant = self.rng.gen_range(0..raws.plants.len()) as u16;
            let kind = match self.rng.gen_range(0..3) {
                0 => ItemKind::Seed,
                1 => ItemKind::Meal,
                _ => ItemKind::Drink,
            };
            goods.push(Item {
                kind,
                stuff: plant,
                name: None,
                pos: Pos::new(0, 0, 0),
                state: ItemState::OnGround,
                reserved_by: None,
                consumed: false,
            });
        }

        self.stats.caravans_arrived += 1;
        self.log_event(format!(
            "A caravan from {civ_name} has arrived! ({} goods — press r to trade)",
            goods.len()
        ));
        self.caravan = Some(Caravan {
            civ_name,
            goods,
            leaves_at: self.clock.tick + CARAVAN_STAY,
            traders,
        });
    }

    /// Departure and the consequences of dead traders.
    fn tick_caravan(&mut self) {
        let Some(caravan) = &self.caravan else { return };
        let trader_dead = caravan
            .traders
            .iter()
            .any(|&t| self.dwarves.get(t).is_some_and(|d| !d.alive));
        if trader_dead {
            let civ = caravan.civ_name.clone();
            let traders = caravan.traders.clone();
            for &t in &traders {
                if let Some(d) = self.dwarves.get_mut(t) {
                    if d.alive {
                        d.alive = false; // fled the map
                    }
                }
            }
            self.caravan = None;
            if self.trader_lost_to_raiders {
                // The civ blames the raiders; trade resumes next season.
                self.trader_lost_to_raiders = false;
                self.log_event(format!(
                    "Raiders slew a trader from {civ}! The caravan scatters."
                ));
            } else {
                // Died to your levers, your floods, or your dwarves' blades.
                self.trade_ban_until =
                    self.clock.tick + TICKS_PER_DAY * dk_core::DAYS_PER_YEAR;
                self.log_event(format!(
                    "A trader from {civ} died in your care! No caravans will come this year."
                ));
            }
            return;
        }
        if self.clock.tick >= caravan.leaves_at {
            let civ = caravan.civ_name.clone();
            let traders = caravan.traders.clone();
            for &t in &traders {
                if let Some(d) = self.dwarves.get_mut(t) {
                    d.alive = false; // departed the map
                }
            }
            self.caravan = None;
            self.log_event(format!("The caravan from {civ} has departed."));
        }
    }

    /// Execute a trade: `offer` are indices into sim items (must be stored,
    /// unreserved goods), `request` are indices into the caravan's wagon.
    /// The caravan accepts when the offer beats the ask by TRADE_MARGIN.
    pub fn execute_trade(
        &mut self,
        offer: &[usize],
        request: &[usize],
        raws: &Raws,
    ) -> Result<(), String> {
        let Some(caravan) = &self.caravan else {
            return Err("no caravan is visiting".to_string());
        };
        if request.is_empty() {
            return Err("select something to buy".to_string());
        }
        let mut seen = std::collections::BTreeSet::new();
        for &i in offer.iter().chain(request.iter()) {
            let _ = i;
        }
        for &i in offer {
            if !seen.insert(("o", i)) {
                return Err("duplicate offer item".to_string());
            }
            let Some(it) = self.items.get(i) else {
                return Err("no such item".to_string());
            };
            if !it.active()
                || it.reserved_by.is_some()
                || !matches!(it.state, ItemState::Stored { .. } | ItemState::OnGround)
            {
                return Err(format!("{:?} is not available to trade", it.kind));
            }
        }
        for &g in request {
            if !seen.insert(("r", g)) {
                return Err("duplicate requested item".to_string());
            }
            if g >= caravan.goods.len() {
                return Err("no such caravan good".to_string());
            }
        }
        let offered: u32 = offer.iter().map(|&i| item_value(&self.items[i], raws)).sum();
        let asked: u32 = request
            .iter()
            .map(|&g| item_value(&caravan.goods[g], raws))
            .sum();
        if (offered as f32) < asked as f32 * TRADE_MARGIN {
            return Err(format!(
                "the merchants scoff: they ask {} in goods for that (you offered {})",
                (asked as f32 * TRADE_MARGIN).ceil() as u32,
                offered
            ));
        }

        // Deal. Your goods leave with the wagon; theirs land at a trader's feet.
        let drop_at = self
            .caravan
            .as_ref()
            .and_then(|c| c.traders.first().copied())
            .and_then(|t| self.dwarves.get(t))
            .map(|d| d.pos)
            .unwrap_or_else(|| self.dwarves[0].pos);
        for &i in offer {
            self.items[i].consumed = true;
        }
        // Remove bought goods from the wagon (descending order keeps indices valid).
        let mut bought: Vec<usize> = request.to_vec();
        bought.sort_unstable_by(|a, b| b.cmp(a));
        let caravan = self.caravan.as_mut().unwrap();
        let mut received = Vec::new();
        for g in bought {
            received.push(caravan.goods.remove(g));
        }
        for mut it in received {
            it.pos = drop_at;
            it.state = ItemState::OnGround;
            self.items.push(it);
        }
        self.stats.trades_completed += 1;
        self.log_event(format!(
            "Trade completed: {} in goods for {} received.",
            offered, asked
        ));
        Ok(())
    }

    /// Visitors mill about near where they stand; no jobs, no needs (they
    /// carry their own provisions), but they will defend themselves.
    fn update_visitor(&mut self, i: usize) {
        self.tick_vitals(i);
        if !self.dwarves[i].alive {
            return;
        }
        if let Some(enemy) = self.adjacent_enemy(i) {
            self.melee(i, enemy);
            return;
        }
        if self.dwarves[i].move_cd > 0 {
            self.dwarves[i].move_cd -= 1;
            return;
        }
        if self.rng.gen_ratio(1, 60) {
            let pos = self.dwarves[i].pos;
            let mut opts = Vec::with_capacity(8);
            path::neighbors(&self.map, pos, &mut opts);
            if !opts.is_empty() {
                let n = opts[self.rng.gen_range(0..opts.len())];
                self.dwarves[i].pos = n;
                self.dwarves[i].move_cd = WALK_COOLDOWN;
            }
        }
    }

    /// Once in a while, inspiration seizes a dwarf: they claim a workshop
    /// and a boulder and will not rest until a masterwork exists.
    fn maybe_strange_mood(&mut self) {
        if !self.rng.gen_ratio(1, 2) {
            return;
        }
        // One mood at a time.
        if self
            .dwarves
            .iter()
            .any(|d| d.alive && matches!(d.task, Task::StrangeMood { .. }))
        {
            return;
        }
        let Some(shop) = self
            .buildings
            .iter()
            .find(|b| matches!(b.kind, BuildingKind::Still | BuildingKind::Kitchen))
            .map(|b| b.pos)
        else {
            return;
        };
        let candidates: Vec<usize> = self
            .dwarves
            .iter()
            .enumerate()
            .filter(|(i, d)| {
                // The player follows no muse: their task loop never runs,
                // so a mood would freeze them (and its boulder) forever.
                self.player != Some(*i)
                    && d.alive
                    && d.faction == Faction::Fort
                    && d.is_idle()
            })
            .map(|(i, _)| i)
            .collect();
        if candidates.is_empty() {
            return;
        }
        let chosen = candidates[self.rng.gen_range(0..candidates.len())];
        let my_region = self.regions.id(self.dwarves[chosen].pos);
        let Some(input) = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, it)| {
                it.kind == ItemKind::Boulder
                    && self.item_takeable(it)
                    && self.regions.id(it.pos) == my_region
            })
            .min_by_key(|(_, it)| it.pos.manhattan(self.dwarves[chosen].pos))
            .map(|(idx, _)| idx)
        else {
            return;
        };
        let start = self.dwarves[chosen].pos;
        let item_pos = self.items[input].pos;
        let Some(p) = path::astar(&self.map, start, item_pos, MAX_ASTAR_NODES) else {
            return;
        };
        self.abandon_task(chosen);
        self.items[input].reserved_by = Some(chosen);
        self.dwarves[chosen].task = Task::StrangeMood {
            shop,
            input,
            path: p,
            stage: FetchStage::ToInput,
            progress: 0,
        };
        let name = self.dwarves[chosen].name.clone();
        self.log_event(format!("{name} is taken by a strange mood!"));
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
                        let mut d = new_dwarf(&mut self.rng, pos, Faction::Fort, raws);
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
        let mut pending_crafts = 0usize;
        for d in &self.dwarves {
            if d.alive {
                match d.task {
                    Task::Craft { kind: CraftKind::Brew, .. } => pending_brews += 1,
                    Task::Craft { kind: CraftKind::Cook, .. } => pending_cooks += 1,
                    Task::Craft { kind: CraftKind::Stonecraft, .. } => pending_crafts += 1,
                    _ => {}
                }
            }
        }
        let has_craftsdwarf = self
            .buildings
            .iter()
            .any(|b| b.kind == BuildingKind::Craftsdwarf);
        for i in 0..self.dwarves.len() {
            if self.player == Some(i) {
                continue; // the player follows no job board
            }
            let d = &self.dwarves[i];
            if d.alive && d.faction == Faction::Fort && d.is_idle() {
                let want_drinks =
                    self.count_kind(ItemKind::Drink) + pending_brews * BATCH < alive * 3;
                let want_meals =
                    self.count_kind(ItemKind::Meal) + pending_cooks * BATCH < alive * 3;
                // Craft trade goods when boulders are surplus (keep a
                // reserve for building) and the craft stock isn't huge.
                let boulders = self.count_kind(ItemKind::Boulder);
                let crafts = self.count_kind(ItemKind::Craft);
                let want_crafts = has_craftsdwarf
                    && boulders > 4 + pending_crafts
                    && crafts + pending_crafts < 20;
                match self.assign_one(i, raws, want_drinks, want_meals, want_crafts) {
                    Some(CraftKind::Brew) => pending_brews += 1,
                    Some(CraftKind::Cook) => pending_cooks += 1,
                    Some(CraftKind::Stonecraft) => pending_crafts += 1,
                    Some(CraftKind::Weave) | None => {}
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
        want_crafts: bool,
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
        // The weary of heart seek the tavern before returning to labor.
        if self.dwarves[i].stress >= TAVERN_STRESS_AT && !self.taverns.is_empty() {
            if let Some(spot) = self
                .taverns
                .iter()
                .flat_map(|t| t.cells())
                .filter(|&c| self.regions.id(c) == my_region && self.map.walkable(c))
                .min_by_key(|&c| c.manhattan(dwarf_pos))
            {
                if let Some(p) = path::astar(&self.map, dwarf_pos, spot, MAX_ASTAR_NODES) {
                    self.dwarves[i].task = Task::Relax {
                        spot,
                        path: p,
                        remaining: RELAX_TICKS,
                        drank: false,
                    };
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
            Butcher { animal: usize },
            Fish { spot: Pos },
        }
        let mut best: Option<(u32, Cand)> = None;
        let consider = |dist: u32, c: Cand, best: &mut Option<(u32, Cand)>| {
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
        // Stone crafts for trade — only when boulders are plentiful, so the
        // fort keeps stone for building.
        if want_crafts {
            if let Some((shop, input)) = self.craft_stone_pair(dwarf_pos, my_region) {
                let d = self.items[input].pos.manhattan(dwarf_pos);
                consider(d, Cand::Craft { shop, input, kind: CraftKind::Stonecraft }, &mut best);
            }
        }
        // Weaving: any wool on hand becomes cloth at the loom.
        if let Some((shop, input)) = self.craft_wool_pair(dwarf_pos, my_region) {
            let d = self.items[input].pos.manhattan(dwarf_pos);
            consider(d, Cand::Craft { shop, input, kind: CraftKind::Weave }, &mut best);
        }
        // Fishing: when the larder runs low, cast a line from a fishery bank.
        if want_meals && !self.fisheries.is_empty() {
            if let Some(spot) = self.fishing_bank(dwarf_pos, my_region) {
                consider(spot.manhattan(dwarf_pos), Cand::Fish { spot }, &mut best);
            }
        }

        // Butchering: marked animals reachable from here.
        for (idx, animal) in self.animals.iter().enumerate() {
            if !animal.alive || !animal.marked || animal.reserved_by.is_some() {
                continue;
            }
            if self.regions.id(animal.pos) != my_region {
                continue;
            }
            consider(animal.pos.manhattan(dwarf_pos), Cand::Butcher { animal: idx }, &mut best);
        }

        // Hauling loose items: corpses go to open tombs, goods to stockpiles.
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
            let dest = if item.kind == ItemKind::Corpse {
                self.buildings
                    .iter()
                    .filter(|b| {
                        b.kind == BuildingKind::Tomb
                            && !b.occupied
                            && self.regions.id(b.pos) == my_region
                    })
                    .min_by_key(|b| b.pos.manhattan(item.pos))
                    .map(|b| b.pos)
            } else if !self.stockpiles.is_empty() {
                self.find_free_cell(item.pos, my_region)
            } else {
                None
            };
            let Some(dest) = dest else { continue };
            consider(item.pos.manhattan(dwarf_pos), Cand::Haul { item: idx, dest }, &mut best);
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
            Cand::Butcher { animal } => {
                let animal_pos = self.animals[animal].pos;
                if let Some(p) = path::astar(&self.map, dwarf_pos, animal_pos, MAX_ASTAR_NODES) {
                    self.animals[animal].reserved_by = Some(i);
                    self.dwarves[i].task = Task::Butcher { animal, path: p, progress: 0 };
                }
            }
            Cand::Fish { spot } => {
                if let Some(p) = path::astar(&self.map, dwarf_pos, spot, MAX_ASTAR_NODES) {
                    self.dwarves[i].task = Task::Fish { spot, path: p, progress: 0 };
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

    /// Nearest (craftsdwarf workshop, boulder) pair for making trade goods.
    fn craft_stone_pair(&self, near: Pos, region: u32) -> Option<(Pos, usize)> {
        let shop = self
            .buildings
            .iter()
            .filter(|b| b.kind == BuildingKind::Craftsdwarf && self.regions.id(b.pos) == region)
            .min_by_key(|b| b.pos.manhattan(near))?;
        let input = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, it)| {
                it.kind == ItemKind::Boulder
                    && self.item_takeable(it)
                    && self.regions.id(it.pos) == region
            })
            .min_by_key(|(_, it)| it.pos.manhattan(near))
            .map(|(i, _)| i)?;
        Some((shop.pos, input))
    }

    /// Nearest (loom, wool) pair for weaving cloth.
    fn craft_wool_pair(&self, near: Pos, region: u32) -> Option<(Pos, usize)> {
        let shop = self
            .buildings
            .iter()
            .filter(|b| b.kind == BuildingKind::Loom && self.regions.id(b.pos) == region)
            .min_by_key(|b| b.pos.manhattan(near))?;
        let input = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, it)| {
                it.kind == ItemKind::Wool
                    && self.item_takeable(it)
                    && self.regions.id(it.pos) == region
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

        // Stress boils over into an episode (never interrupts a mood).
        if self.dwarves[i].stress >= 100.0
            && !matches!(
                self.dwarves[i].task,
                Task::Tantrum { .. } | Task::Sulk { .. } | Task::StrangeMood { .. }
            )
        {
            self.abandon_task(i);
            let name = self.dwarves[i].name.clone();
            if self.dwarves[i].personality.cheer < 50.0 {
                self.dwarves[i].task = Task::Sulk { remaining: EPISODE_TICKS };
                self.push_thought(i, ThoughtKind::FellIntoGloom);
                self.log_event(format!("{name} has withdrawn into a dark gloom."));
            } else {
                self.dwarves[i].task = Task::Tantrum { remaining: EPISODE_TICKS };
                self.push_thought(i, ThoughtKind::ThrewTantrum);
                self.log_event(format!("{name} is throwing a tantrum!"));
            }
            return;
        }

        let task = self.dwarves[i].task.clone();
        match task {
            Task::Idle { wander_cd } => {
                if self.dwarves[i].fatigue >= 100.0 {
                    self.dwarves[i].task = Task::Sleep { remaining: 1200 };
                    return;
                }
                // Company: idle dwarves next to each other strike up a chat.
                if self.dwarves[i].chat_cd == 0 {
                    let me = self.dwarves[i].pos;
                    let partner = self.dwarves.iter().enumerate().find(|(j, d)| {
                        *j != i
                            && d.alive
                            && d.faction == Faction::Fort
                            && d.is_idle()
                            && d.chat_cd == 0
                            && d.pos.z == me.z
                            && d.pos.x.abs_diff(me.x) + d.pos.y.abs_diff(me.y) <= 1
                    });
                    if let Some((j, _)) = partner {
                        // Warm-up scales with how social the chattier one is.
                        let bond = 4 + (self.dwarves[i].personality.social.max(
                            self.dwarves[j].personality.social,
                        ) / 20.0) as i32;
                        *self.dwarves[i].relationships.entry(j).or_insert(0) += bond;
                        *self.dwarves[j].relationships.entry(i).or_insert(0) += bond;
                        let cd_i = 600 + (100.0 - self.dwarves[i].personality.social) as u16 * 6;
                        let cd_j = 600 + (100.0 - self.dwarves[j].personality.social) as u16 * 6;
                        self.dwarves[i].chat_cd = cd_i;
                        self.dwarves[j].chat_cd = cd_j;
                        self.push_thought(i, ThoughtKind::PleasantChat);
                        self.push_thought(j, ThoughtKind::PleasantChat);
                    }
                } else {
                    self.dwarves[i].chat_cd -= 1;
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
                    let lazy = (100.0 - self.dwarves[i].personality.diligence) as u16;
                    let cd = self.rng.gen_range(40..160) + lazy;
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
                let diligent = self.dwarves[i].personality.diligence > 70.0;
                let speed = 1
                    + self.dwarves[i].skill_level(Skill::Mining) as u16 / 2
                    + diligent as u16;
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
                    // Corpse delivered to an open tomb: a burial.
                    if self.items[item].kind == ItemKind::Corpse {
                        let tomb = self
                            .buildings
                            .iter()
                            .position(|b| {
                                b.kind == BuildingKind::Tomb && b.pos == here && !b.occupied
                            });
                        if let Some(t) = tomb {
                            self.bury(item, t, i);
                            return;
                        }
                    }
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
                            CraftKind::Stonecraft | CraftKind::Weave => Skill::Crafting,
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
                        match kind {
                            CraftKind::Brew => {
                                self.stats.drinks_brewed += BATCH as u32;
                                for _ in 0..BATCH {
                                    self.spawn_item(ItemKind::Drink, stuff, shop);
                                }
                                self.push_thought(i, ThoughtKind::BrewedDrink);
                            }
                            CraftKind::Cook => {
                                self.stats.meals_cooked += BATCH as u32;
                                for _ in 0..BATCH {
                                    self.spawn_item(ItemKind::Meal, stuff, shop);
                                }
                                self.push_thought(i, ThoughtKind::CookedMeal);
                            }
                            CraftKind::Stonecraft => {
                                // One boulder yields a couple of trade goods.
                                self.stats.crafts_made += 2;
                                for _ in 0..2 {
                                    self.spawn_item(ItemKind::Craft, stuff, shop);
                                }
                                self.push_thought(i, ThoughtKind::CookedMeal); // a job well done
                            }
                            CraftKind::Weave => {
                                self.stats.cloth_woven += 1;
                                self.spawn_item(ItemKind::Cloth, 0, shop);
                                self.push_thought(i, ThoughtKind::CookedMeal);
                            }
                        }
                        self.add_xp(i, skill, 30);
                        self.dwarves[i].task = Task::Idle { wander_cd: 3 };
                    }
                }
            }
            // Fort citizens never chase (hostiles use update_hostile);
            // clear it if it somehow appears.
            Task::Fight { .. } => {
                self.dwarves[i].task = Task::Idle { wander_cd: 5 };
            }
            Task::Butcher { animal, mut path, progress } => {
                // The quarry may have died, been un-marked, or wandered off.
                let valid = self
                    .animals
                    .get(animal)
                    .is_some_and(|a| a.alive && a.marked && a.reserved_by == Some(i));
                if !valid {
                    self.abandon_task(i);
                    return;
                }
                if !path.is_empty() {
                    if self.step_along(i, &mut path) {
                        self.dwarves[i].task = Task::Butcher { animal, path, progress };
                    } else {
                        self.abandon_task(i);
                    }
                    return;
                }
                // Arrived where the animal was. If it drifted, chase it.
                let here = self.dwarves[i].pos;
                let apos = self.animals[animal].pos;
                if here.manhattan(apos) > 1 {
                    match path::astar(&self.map, here, apos, MAX_ASTAR_NODES) {
                        Some(p) if !p.is_empty() => {
                            self.dwarves[i].task = Task::Butcher { animal, path: p, progress };
                        }
                        _ => self.abandon_task(i),
                    }
                    return;
                }
                let progress = progress + 1;
                if progress < BUTCHER_WORK {
                    self.dwarves[i].task = Task::Butcher { animal, path, progress };
                    return;
                }
                // Slaughter: meat for the larder, hides tanned another day.
                let kind = self.animals[animal].kind;
                let adult = self.animals[animal].is_adult();
                self.animals[animal].alive = false;
                self.animals[animal].reserved_by = None;
                let meat = if adult { kind.meat_yield() } else { kind.meat_yield() / 2 + 1 };
                for _ in 0..meat {
                    self.spawn_item(ItemKind::Meal, 0, apos);
                }
                self.stats.animals_butchered += 1;
                self.add_xp(i, Skill::Cooking, 15);
                self.log_event(format!(
                    "A {} is butchered — {meat} servings of meat.",
                    kind.name()
                ));
                self.dwarves[i].task = Task::Idle { wander_cd: 3 };
            }
            Task::Relax { spot, mut path, remaining, drank } => {
                if !path.is_empty() {
                    if self.step_along(i, &mut path) {
                        self.dwarves[i].task = Task::Relax { spot, path, remaining, drank };
                    } else {
                        // Path broke; if we're in a tavern anyway, stay.
                        if self.tavern_at(self.dwarves[i].pos) {
                            self.dwarves[i].task =
                                Task::Relax { spot, path: Vec::new(), remaining, drank };
                        } else {
                            self.dwarves[i].task = Task::Idle { wander_cd: 5 };
                        }
                    }
                    return;
                }
                // At the tavern: have a drink if one's to hand, unwind, chat.
                let here = self.dwarves[i].pos;
                let mut drank = drank;
                if !drank {
                    let drink = self.items.iter().position(|it| {
                        it.kind == ItemKind::Drink
                            && self.item_takeable(it)
                            && self.tavern_at(it.pos)
                    });
                    if let Some(idx) = drink {
                        self.items[idx].consumed = true;
                        self.dwarves[i].thirst = 0.0;
                        drank = true;
                    }
                }
                // Socialize with anyone else unwinding nearby.
                if remaining % 100 == 0 {
                    let companions: Vec<usize> = self
                        .dwarves
                        .iter()
                        .enumerate()
                        .filter(|(j, d)| {
                            *j != i
                                && d.alive
                                && d.faction == Faction::Fort
                                && matches!(d.task, Task::Relax { .. })
                                && d.pos.z == here.z
                                && d.pos.manhattan(here) <= 3
                        })
                        .map(|(j, _)| j)
                        .collect();
                    for j in companions {
                        *self.dwarves[i].relationships.entry(j).or_insert(0) += 2;
                    }
                }
                // Unwinding steadily sheds stress.
                self.dwarves[i].stress = (self.dwarves[i].stress - 0.1).max(0.0);
                if remaining == 0 {
                    self.push_thought(i, ThoughtKind::RelaxedAtTavern);
                    self.dwarves[i].task = Task::Idle { wander_cd: 10 };
                } else {
                    self.dwarves[i].task =
                        Task::Relax { spot, path, remaining: remaining - 1, drank };
                }
            }
            Task::Fish { spot, mut path, progress } => {
                if !path.is_empty() {
                    if self.step_along(i, &mut path) {
                        self.dwarves[i].task = Task::Fish { spot, path, progress };
                    } else {
                        self.abandon_task(i);
                    }
                    return;
                }
                // Standing on the bank: cast until something bites.
                let progress = progress + 1;
                if progress < FISH_WORK {
                    self.dwarves[i].task = Task::Fish { spot, path, progress };
                    return;
                }
                // A catch: prepared fish, ready to eat.
                self.spawn_item(ItemKind::Meal, 0, self.dwarves[i].pos);
                self.stats.fish_caught += 1;
                self.add_xp(i, Skill::Farming, 10);
                self.dwarves[i].task = Task::Idle { wander_cd: 5 };
            }
            Task::Tantrum { remaining } => {
                if remaining == 0 {
                    self.dwarves[i].stress = 50.0;
                    self.dwarves[i].task = Task::Idle { wander_cd: 20 };
                    return;
                }
                // Storm around; unsettle anyone nearby every so often.
                if self.dwarves[i].move_cd == 0 {
                    let pos = self.dwarves[i].pos;
                    let mut opts = Vec::with_capacity(8);
                    path::neighbors(&self.map, pos, &mut opts);
                    if !opts.is_empty() {
                        let n = opts[self.rng.gen_range(0..opts.len())];
                        self.dwarves[i].pos = n;
                        self.carry_item_along(i);
                    }
                    self.dwarves[i].move_cd = WALK_COOLDOWN;
                } else {
                    self.dwarves[i].move_cd -= 1;
                }
                if remaining % 200 == 0 {
                    let me = self.dwarves[i].pos;
                    let witnesses: Vec<usize> = self
                        .dwarves
                        .iter()
                        .enumerate()
                        .filter(|(j, d)| {
                            *j != i
                                && d.alive
                                && d.faction == Faction::Fort
                                && d.pos.z == me.z
                                && d.pos.x.abs_diff(me.x) + d.pos.y.abs_diff(me.y) <= 4
                        })
                        .map(|(j, _)| j)
                        .collect();
                    for j in witnesses {
                        self.push_thought(j, ThoughtKind::DisturbedByTantrum);
                    }
                }
                self.dwarves[i].task = Task::Tantrum { remaining: remaining - 1 };
            }
            Task::Sulk { remaining } => {
                if remaining == 0 {
                    self.dwarves[i].stress = 60.0;
                    self.dwarves[i].task = Task::Idle { wander_cd: 30 };
                } else {
                    self.dwarves[i].task = Task::Sulk { remaining: remaining - 1 };
                }
            }
            Task::StrangeMood { shop, input, mut path, stage, progress } => {
                if !path.is_empty() {
                    if self.step_along(i, &mut path) {
                        self.dwarves[i].task =
                            Task::StrangeMood { shop, input, path, stage, progress };
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
                                self.dwarves[i].task = Task::StrangeMood {
                                    shop,
                                    input,
                                    path: p,
                                    stage: FetchStage::ToStation,
                                    progress,
                                };
                            }
                            None => self.abandon_task(i),
                        }
                    }
                    FetchStage::ToStation => {
                        let progress = progress + 1;
                        if progress < MOOD_WORK {
                            self.dwarves[i].task =
                                Task::StrangeMood { shop, input, path, stage, progress };
                            return;
                        }
                        let material = self.items[input].stuff;
                        self.items[input].consumed = true;
                        self.items[input].reserved_by = None;
                        let artifact_name = names::artifact_name(&mut self.rng);
                        let mat_name = raws.materials.get(material).name.clone();
                        let full = format!("{artifact_name}, a {mat_name} masterwork");
                        self.spawn_named_item(
                            ItemKind::Artifact,
                            material,
                            shop,
                            Some(full.clone()),
                        );
                        self.dwarves[i].artifacts_made += 1;
                        self.dwarves[i].stress = 0.0;
                        let name = self.dwarves[i].name.clone();
                        self.push_thought(i, ThoughtKind::MadeArtifact);
                        self.log_event(format!("{name} has created {full}!"));
                        // The whole fort takes pride in it.
                        let admirers: Vec<usize> = self
                            .dwarves
                            .iter()
                            .enumerate()
                            .filter(|(j, d)| *j != i && d.alive && d.faction == Faction::Fort)
                            .map(|(j, _)| j)
                            .collect();
                        for j in admirers {
                            self.push_thought(j, ThoughtKind::SawArtifact);
                        }
                        self.dwarves[i].task = Task::Idle { wander_cd: 10 };
                    }
                }
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
            .filter(|(_, d)| d.alive && matches!(d.faction, Faction::Fort | Faction::Visitor))
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
            .filter(|(_, d)| {
                d.alive && me.faction.hostile_to(d.faction) && d.pos.z == me.pos.z
            })
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
            // Merchants killed by raiders are a tragedy, not your crime.
            if self.dwarves[defender].faction == Faction::Visitor
                && self.dwarves[attacker].faction == Faction::Hostile
            {
                self.trader_lost_to_raiders = true;
            }
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
        let incinerated = self.map.magma_at(pos) > 0;

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

        if incinerated {
            if self.dwarves[i].faction == Faction::Hostile {
                self.stats.raiders_slain += 1;
            }
            self.log_event(format!("{name} is incinerated by magma!"));
            self.kill_dwarf(i);
        } else if drowned {
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

        // Stress slowly drains in calm times; boiling over breaks the mind.
        d.stress = (d.stress - 0.001).max(0.0);
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
        self.dwarves[i].died_at = Some(self.clock.tick);
        // `deaths` means fort citizens lost; raider kills have their own
        // counters at the call sites.
        if self.dwarves[i].faction == Faction::Fort {
            // The body remains, and it wants burying. `stuff` holds the dead
            // dwarf's index so burial/haunting never confuse two dwarves who
            // happen to share a generated name. (The dwarves vec is never
            // pruned; a fort would need 65k+ lifetime spawns to overflow u16,
            // which no real game reaches — but assert it in debug builds.)
            debug_assert!(i <= u16::MAX as usize, "dwarf index overflows corpse id");
            let name = self.dwarves[i].name.clone();
            let pos = self.dwarves[i].pos;
            self.spawn_named_item(
                ItemKind::Corpse,
                i as u16,
                pos,
                Some(format!("remains of {name}")),
            );
            self.stats.deaths += 1;
            // Friends grieve.
            let mourners: Vec<usize> = self
                .dwarves
                .iter()
                .enumerate()
                .filter(|(j, d)| {
                    *j != i
                        && d.alive
                        && d.faction == Faction::Fort
                        && d.relationships.get(&i).copied().unwrap_or(0) >= FRIEND_AT
                })
                .map(|(j, _)| j)
                .collect();
            for j in mourners {
                self.push_thought(j, ThoughtKind::FriendDied);
            }
        }
    }

    fn push_thought(&mut self, i: usize, kind: ThoughtKind) {
        let tick = self.clock.tick;
        let d = &mut self.dwarves[i];
        d.happiness = (d.happiness + kind.delta()).clamp(0.0, 100.0);
        // Bad thoughts pile onto stress; a sunny disposition sheds most of
        // it, a gloomy one magnifies it. Good thoughts bleed stress off.
        let delta = kind.delta();
        if delta < 0.0 {
            let amplifier = 1.5 - d.personality.cheer / 100.0; // 0.5..1.5
            d.stress = (d.stress - delta * amplifier).min(150.0);
        } else {
            d.stress = (d.stress - delta * 0.5).max(0.0);
        }
        d.thoughts.push((tick, kind));
        if d.thoughts.len() > 12 {
            d.thoughts.remove(0);
        }
    }

    fn add_xp(&mut self, i: usize, skill: Skill, xp: u32) {
        *self.dwarves[i].skills.entry(skill).or_insert(0) += xp;
    }

    fn spawn_item(&mut self, kind: ItemKind, stuff: u16, pos: Pos) {
        self.spawn_named_item(kind, stuff, pos, None);
    }

    fn spawn_named_item(&mut self, kind: ItemKind, stuff: u16, pos: Pos, name: Option<String>) {
        self.items.push(Item {
            kind,
            stuff,
            name,
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
                    Tile { material: tile.material, shape: TileShape::Floor, water: tile.water, magma: 0 },
                );
            }
            DesignationKind::Stairs => {
                self.map.set_at(
                    target,
                    Tile { material: tile.material, shape: TileShape::Stairs, water: tile.water, magma: 0 },
                );
            }
            DesignationKind::Channel => {
                // The floor is dug away: open space here, floor below.
                self.map.set_at(
                    target,
                    Tile { material: NO_MATERIAL, shape: TileShape::Empty, water: tile.water, magma: 0 },
                );
                let below = Pos::new(target.x, target.y, target.z - 1);
                if let Some(bt) = self.map.tile_at(below) {
                    if bt.is_solid() {
                        boulder_from = bt;
                        self.map.set_at(
                            below,
                            Tile { material: bt.material, shape: TileShape::Floor, water: bt.water, magma: 0 },
                        );
                    }
                }
                self.water.wake(below);
                self.magma.wake(below);
            }
        }
        self.regions.dirty = true;
        self.map_changed = true;
        self.water.wake(target);
        self.magma.wake(target);

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
            self.stats.boulders_mined += 1;
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
            Task::StrangeMood { input, .. } => {
                if self.items[input].reserved_by == Some(i) {
                    self.items[input].reserved_by = None;
                }
            }
            Task::Butcher { animal, .. } => {
                if let Some(a) = self.animals.get_mut(animal) {
                    if a.reserved_by == Some(i) {
                        a.reserved_by = None;
                    }
                }
            }
            Task::Idle { .. }
            | Task::Sleep { .. }
            | Task::Fight { .. }
            | Task::Tantrum { .. }
            | Task::Sulk { .. }
            | Task::Relax { .. }
            | Task::Fish { .. } => {}
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

fn new_dwarf(rng: &mut ChaCha8Rng, pos: Pos, faction: Faction, raws: &Raws) -> Dwarf {
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
        personality: Personality::roll(rng),
        stress: 0.0,
        relationships: BTreeMap::new(),
        favorite_material: rng.gen_range(0..raws.materials.len()) as u16,
        favorite_crop: rng.gen_range(0..raws.plants.len()) as u16,
        artifacts_made: 0,
        thoughts: Vec::new(),
        skills: BTreeMap::new(),
        task: Task::Idle { wander_cd: rng.gen_range(20..80) },
        move_cd: 0,
        attack_cd: 0,
        chat_cd: 0,
        starving_since: None,
        dehydrated_since: None,
        died_at: None,
        ghost: false,
    }
}

// ------------------------------------------------------------------- saves

const SAVE_MAGIC: u32 = 0x444B_5331; // "DKS1"
const SAVE_VERSION: u32 = 19;

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
    // Boulders AND artifacts carry material indices; everything else is
    // plant-based. Caravan wagon goods are items too and must be remapped.
    let remap_item = |item: &mut Item| -> Result<()> {
        item.stuff = match item.kind {
            ItemKind::Boulder | ItemKind::Artifact | ItemKind::Craft => {
                remap_one(&mat_remap, item.stuff, "material")?
            }
            // These carry no raws index (dwarf index, or nothing).
            ItemKind::Corpse | ItemKind::Wool | ItemKind::Cloth => item.stuff,
            _ => remap_one(&plant_remap, item.stuff, "plant")?,
        };
        Ok(())
    };
    for item in &mut sim.items {
        remap_item(item)?;
    }
    if let Some(caravan) = &mut sim.caravan {
        for item in &mut caravan.goods {
            remap_item(item)?;
        }
    }
    for farm in sim.farms.values_mut() {
        farm.crop = remap_one(&plant_remap, farm.crop, "plant")?;
    }
    sim.rebuild_caches();
    Ok(sim)
}
