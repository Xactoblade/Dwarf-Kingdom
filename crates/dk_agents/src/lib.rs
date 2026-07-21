//! The living simulation: dwarves, needs, designations, jobs, farming,
//! workshops, hauling, happiness.
//!
//! Engine-agnostic and fully deterministic — `Sim::step()` advances one fixed
//! tick, so the whole game loop can run (and be tested) headlessly.

use anyhow::{Context, Result};
use dk_core::{Calendar, Season, DAYS_PER_SEASON, SEASONS_PER_YEAR, TICKS_PER_DAY};
use dk_raws::{MaterialCategory, Raws};
pub use dk_raws::{CombatStats, DamageType};
use dk_sim::{FluidSim, WaterSim};
use dk_world::path::{self, Pos, Regions};
use dk_world::{Map, Tile, TileShape, NO_MATERIAL};
use rand::Rng;
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path as FsPath;

pub mod names;

/// Ticks of digging to mine out one tile.
pub const MINE_WORK: u16 = 40;
/// Ticks of work to harvest a grown crop.
pub const HARVEST_WORK: u16 = 60;
/// Ticks of work (at speed 1) to fell a tree.
pub const CHOP_WORK: u16 = 120;
/// Ticks of work (at speed 1) to forage a wild shrub for berries. Quicker than
/// felling a tree — you only stoop and pick.
pub const GATHER_WORK: u16 = 50;
/// A foraged shrub regrows on this cadence: once a day, a living shrub may cast
/// a seed to an open neighbour, so a tended berry patch is a renewable food.
pub const SHRUB_REGROW_INTERVAL: u64 = TICKS_PER_DAY;
/// Forests regrow far more slowly than berries: a sapling takes root near a
/// standing tree only every few days, so a felled woodland recovers over time.
pub const TREE_REGROW_INTERVAL: u64 = 3 * TICKS_PER_DAY;
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
/// How long food left out of the fort's stores lasts before it turns.
///
/// A month, which is the only figure Dwarf Fortress has ever published for it
/// ("meat and prepared meals will rot if not placed on a stockpile within a
/// month or so") — and that from a version two behind, on a page since proven
/// wrong about barrels. Treat it as ours to tune, not as a ported fact.
pub const SHELF_LIFE_DAYS: u64 = 30;
/// How far the stench of a rotting meal carries.
pub const MIASMA_RANGE: u32 = 6;
/// How close a cat must be to catch a vermin.
pub const CAT_REACH: u32 = 3;
/// The most vermin a fort's country will support at once.
pub const VERMIN_CAP: usize = 4;
/// How often another one creeps in. Ours, not Dwarf Fortress's — the wiki
/// documents no rate anywhere.
pub const VERMIN_SPAWN_INTERVAL: u64 = TICKS_PER_DAY / 2;
/// Vermin are quick, but not that quick.
pub const VERMIN_WALK_COOLDOWN: u8 = 6;
/// How often one of them actually gets a mouthful.
///
/// Ours, not Dwarf Fortress's — the wiki gives no rate anywhere, so this is a
/// number I picked and then measured. Twice a day per rat out-ate the fort's
/// whole kitchen: an uncatted fort lost every meal it cooked and starved two
/// dwarves in a year. A vermin should be a bleed you notice, not a second
/// famine. At one bite every two days a catless fort still loses its stores to
/// them, but slowly enough to see it coming and do something — get a cat, or
/// pack the larder into casks.
pub const VERMIN_EAT_INTERVAL: u64 = TICKS_PER_DAY * 2;
/// Ticks between melee swings.
pub const ATTACK_COOLDOWN: u8 = 40;
/// How close a raider must be before a war dog charges it.
pub const WAR_DOG_ENGAGE: u32 = 18;
/// Damage a suit of armor turns aside from each blow that lands on its wearer
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
/// Ticks of work to war-train a dog.
pub const TRAIN_WORK: u16 = 160;
/// Ticks of work to raise a constructed wall.
pub const BUILD_WORK: u16 = 60;
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
/// Ticks between a dwarf's visits to the temple to worship.
pub const PRAYER_INTERVAL: u64 = 6 * TICKS_PER_DAY;
/// Ticks a vampire waits between feedings — it hunts roughly nightly, so a
/// single victim can recover between visits unless the vampire returns to it.
pub const VAMPIRE_FEED_INTERVAL: u64 = 3 * TICKS_PER_DAY / 2;
/// Blood a vampire drains in one feeding (of 100). Rarely lethal in a single
/// bite, but repeated feeding on the same sleeper eventually bleeds them white.
pub const VAMPIRE_DRAIN: f32 = 34.0;
/// Blood spatter: intensity per tile (a fresh pool), and how it dries away.
pub const BLOOD_MAX: u16 = 200;
/// Ticks between drying passes, and how much intensity each pass removes — a
/// fresh pool fades over roughly a day, a light drip within hours.
pub const BLOOD_DRY_INTERVAL: u64 = TICKS_PER_DAY / 20;
pub const BLOOD_DRY: u16 = 8;
/// Gore decay: a severed part rots through stages, its `stuff` field holding
/// the stage (0 fresh, 1 rotting, 2 skeletal). Flesh sours within a day and is
/// picked clean to bone over a few — after which the bones lie there for good.
pub const GORE_ROT: u64 = TICKS_PER_DAY;
pub const GORE_SKELETONIZE: u64 = 3 * TICKS_PER_DAY;
/// Days in the lunar cycle; a werebeast transforms during the first few nights.
pub const WERE_MOON_CYCLE: u64 = 28;
/// Nights of each cycle the moon is full (a cursed dwarf is a beast).
pub const WERE_MOON_NIGHTS: u64 = 2;
/// Ticks between a necromancer's attempts to raise a nearby corpse.
pub const NECRO_INTERVAL: u64 = 40;
/// How near (Manhattan) a corpse must be for a necromancer to raise it.
pub const NECRO_RANGE: u32 = 6;
/// Ticks spent in prayer.
pub const PRAY_TICKS: u16 = 200;
/// Ticks a wounded dwarf lingers in the hospital before checking out.
pub const REST_TICKS: u16 = 500;
/// Ticks a soldier drills at the barracks per session.
pub const SPAR_TICKS: u16 = 300;

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
    /// Smooth and engrave a wall: the wall stays, but its face is carved with
    /// a scene from the fortress's history.
    Smooth,
    /// Fell a tree standing on this tile, yielding a log.
    Chop,
    /// Forage a wild shrub standing on this tile, yielding edible berries.
    Gather,
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
    /// A limb, head, or other part struck clean off a body in battle. `stuff`
    /// unused; `name` says which part ("severed left arm"). Refuse, like a
    /// corpse — gore for the ground, worth nothing.
    BodyPart,
    /// A decorative stone craft — a trade good. `stuff` = material index.
    Craft,
    /// A trinket carved from a skeletonized body part — the fort's use for the
    /// bones a battle leaves behind. `stuff` unused; a modest, renewable trade
    /// good, worth rather less than a stone craft.
    BoneCraft,
    /// Raw wool sheared from sheep. `stuff` unused.
    Wool,
    /// Woven cloth — a trade good. `stuff` unused.
    Cloth,
    /// A rough gem struck while mining. `stuff` = gem-type index.
    RoughGem,
    /// A cut gem — a premium trade good. `stuff` = gem-type index.
    CutGem,
    /// A forged weapon. `stuff` = material index. Arms a soldier for battle
    /// and is a valuable trade good in its own right.
    Weapon,
    /// Blown glass — the fort's finest trade good. `stuff` = material index it
    /// was fluxed from.
    Glass,
    /// A smelted metal bar — the refined stock a forge works into weapons.
    /// `stuff` = material index it was smelted from. A trade good in its own
    /// right, and the first step of the fort's metal industry.
    Bar,
    /// Forged plate — worn by a soldier, it turns aside blows that would maim
    /// an unarmored dwarf. `stuff` = material index it was forged from.
    Armor,
    /// A shield — held, not worn. It blocks blows outright rather than
    /// softening them, and a skilled shield-arm turns aside far more than a
    /// green one. `stuff` = material index.
    Shield,
    /// A bed built at the mason's workshop. `stuff` = material index. A bulky
    /// trade good, and its owner sleeps better than a dwarf on the bare stone.
    Bed,
    /// Sewn clothes — the fort's woven cloth made into something to wear.
    /// `stuff` unused. A fine trade good, and a well-dressed dwarf is content.
    Clothes,
    /// A felled log. `stuff` unused. The carpenter's raw stock, and a modest
    /// trade good.
    Log,
    /// A barrel worked from a log at the carpenter's shop. `stuff` unused.
    /// A CONTAINER: standing in a stockpile it swallows the fort's food and
    /// drink, so one tile holds a larder instead of a single crop. Also a
    /// fine wooden trade good — sold with whatever is inside it.
    Barrel,
    /// A bin worked from a log at the carpenter's shop. `stuff` unused. The
    /// barrel's counterpart for goods: bars, cloth, leather, gems, crafts and
    /// arms. Never food — a bin is no place for a meal.
    Bin,
    /// A carved statue. `stuff` = material index. A precious work of art that
    /// beautifies the fort — and a rich trade good.
    Statue,
    /// A wooden musical instrument worked from a log. `stuff` unused. Played at
    /// the tavern, it lets the fort compose songs; also a fine trade good.
    Instrument,
    /// A raw animal hide, saved from butchering once a tanner stands. `stuff`
    /// unused. The tanner's stock.
    Hide,
    /// Tanned leather. `stuff` unused. A fine trade good worked from a hide.
    Leather,
    /// Foraged berries and wild roots, gathered from a shrub on the surface.
    /// `stuff` unused. Directly edible with no cooking — the fort's simplest
    /// food — and a modest trade good.
    Berry,
}

impl ItemKind {
    /// Every kind there is. The compiler cannot hand us this, so adding a kind
    /// means adding it here too — `every_kind_is_listed_and_filed` fails loudly
    /// if you forget.
    pub const ALL: [ItemKind; 29] = [
        ItemKind::Boulder,
        ItemKind::Seed,
        ItemKind::Crop,
        ItemKind::Meal,
        ItemKind::Drink,
        ItemKind::Artifact,
        ItemKind::Corpse,
        ItemKind::BodyPart,
        ItemKind::Craft,
        ItemKind::BoneCraft,
        ItemKind::Wool,
        ItemKind::Cloth,
        ItemKind::RoughGem,
        ItemKind::CutGem,
        ItemKind::Weapon,
        ItemKind::Glass,
        ItemKind::Bar,
        ItemKind::Armor,
        ItemKind::Shield,
        ItemKind::Bed,
        ItemKind::Clothes,
        ItemKind::Log,
        ItemKind::Barrel,
        ItemKind::Bin,
        ItemKind::Statue,
        ItemKind::Instrument,
        ItemKind::Hide,
        ItemKind::Leather,
        ItemKind::Berry,
    ];

    /// The stable name this kind is priced under in `data/economy/prices.ron`
    /// (see `EconomyConfig`). Must match the variant name; a mismatch is caught
    /// by `validate_economy`.
    pub fn key(self) -> &'static str {
        match self {
            ItemKind::Boulder => "Boulder",
            ItemKind::Seed => "Seed",
            ItemKind::Crop => "Crop",
            ItemKind::Meal => "Meal",
            ItemKind::Drink => "Drink",
            ItemKind::Artifact => "Artifact",
            ItemKind::Corpse => "Corpse",
            ItemKind::BodyPart => "BodyPart",
            ItemKind::Craft => "Craft",
            ItemKind::BoneCraft => "BoneCraft",
            ItemKind::Wool => "Wool",
            ItemKind::Cloth => "Cloth",
            ItemKind::RoughGem => "RoughGem",
            ItemKind::CutGem => "CutGem",
            ItemKind::Weapon => "Weapon",
            ItemKind::Glass => "Glass",
            ItemKind::Bar => "Bar",
            ItemKind::Armor => "Armor",
            ItemKind::Shield => "Shield",
            ItemKind::Bed => "Bed",
            ItemKind::Clothes => "Clothes",
            ItemKind::Log => "Log",
            ItemKind::Barrel => "Barrel",
            ItemKind::Bin => "Bin",
            ItemKind::Statue => "Statue",
            ItemKind::Instrument => "Instrument",
            ItemKind::Hide => "Hide",
            ItemKind::Leather => "Leather",
            ItemKind::Berry => "Berry",
        }
    }
}

/// The sky's mood, cycling with the seasons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Weather {
    #[default]
    Clear,
    Rain,
    Snow,
}

impl Weather {
    pub fn name(self) -> &'static str {
        match self {
            Weather::Clear => "clear",
            Weather::Rain => "rain",
            Weather::Snow => "snow",
        }
    }
}

// Gems are data now — see `dk_raws::GemRegistry` (`data/gems/*.ron`). Access is
// `raws.gems.name/color/value_tier(idx)`; a `RoughGem`/`CutGem` still stores its
// gem by index in `stuff`, remapped on load like any other raws index.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ItemState {
    OnGround,
    Carried { by: usize },
    Stored { stockpile: usize },
    /// Packed inside a barrel or bin (an item index). The item's `pos` mirrors
    /// its container's tile, so a dwarf who wants it simply walks to the
    /// container and takes it out — every "nearest X" query keeps working
    /// unchanged. Containment is recorded HERE and nowhere else: a container
    /// keeps no list of what it holds, so contents can never be orphaned by a
    /// stale index. `contents_of` derives the list when it's needed.
    Inside { container: usize },
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
    /// Craftsdwarfship: 0 = ordinary, up to 5 = a masterwork. Set from the
    /// maker's skill; raises the item's worth.
    pub quality: u8,
    /// The tick this item came into the world. Food reckons its age from here
    /// (see `tick_spoilage`); everything else ignores it.
    pub made_at: u64,
    /// A subtype the `kind` alone doesn't carry. For a Weapon it is the
    /// `WeaponKind` (a sword is not a hammer); unused, and 0, for everything
    /// else.
    #[serde(default)]
    pub variant: u8,
}

impl Item {
    /// This weapon's kind as an index into `raws.weapons`, or `None` if it is not
    /// a weapon. Combat/UI resolve the index against the weapon registry.
    pub fn weapon_variant(&self) -> Option<u16> {
        if self.kind != ItemKind::Weapon {
            return None;
        }
        Some(self.variant as u16)
    }
}

/// How many of `holding` fit in one `container`, or 0 if that container will
/// not take that kind at all. This is the whole rulebook for what goes where:
/// barrels take food and drink, bins take goods, and neither takes the other.
///
/// The numbers are Dwarf Fortress's own (wiki: Barrel, Using bins and
/// barrels): "Barrels can hold up to 60 prepared meals, plants, or cheeses,
/// 30 pieces of meat or fish, any number of units of brewed alcohol (but only
/// a single stack)"; "Each bin can store up to 12 bars or blocks, while 30 or
/// more small crafts may fit into a single bin."
///
/// Drink follows DF's "any number of units of brewed alcohol (but only a
/// single stack)". A brew yields one stack, and that stack is `BATCH` units of
/// our unit-sized `Drink` — so a barrel of wine holds exactly what one brewing
/// put in it and takes no more, which is why DF's embark barrels arrive
/// holding only a few units each.
pub fn container_capacity(container: ItemKind, holding: ItemKind) -> usize {
    match (container, holding) {
        // A barrel is for food and drink.
        (ItemKind::Barrel, ItemKind::Drink) => BATCH,
        (ItemKind::Barrel, ItemKind::Meal | ItemKind::Crop | ItemKind::Berry) => 60,
        // DF keeps seeds in bags and the bags in barrels. We have no bags, so
        // seeds ride in the barrel directly rather than inventing an item to
        // stand between them.
        (ItemKind::Barrel, ItemKind::Seed) => 60,
        // A bin is for goods. Bars are the wiki's own 12; leather 45 and gems
        // 305 come from its goods-storage table; the rest take the "30 or more
        // small crafts" line as the house rule for a worked good.
        (ItemKind::Bin, ItemKind::Bar) => 12,
        (ItemKind::Bin, ItemKind::Leather | ItemKind::Hide) => 45,
        (ItemKind::Bin, ItemKind::RoughGem | ItemKind::CutGem) => 305,
        (ItemKind::Bin, ItemKind::Cloth | ItemKind::Wool) => 30,
        (
            ItemKind::Bin,
            ItemKind::Craft
            | ItemKind::Clothes
            | ItemKind::Weapon
            | ItemKind::Armor
            | ItemKind::Glass,
        ) => 30,
        // Everything else — furniture, stone, logs, corpses, artifacts, and
        // containers themselves — is stored loose, as in DF.
        _ => 0,
    }
}

/// Is this item a container others can be packed into?
pub fn is_container(kind: ItemKind) -> bool {
    matches!(kind, ItemKind::Barrel | ItemKind::Bin)
}

/// The adjective for a quality tier (0..=5), for describing crafted goods.
pub fn quality_name(q: u8) -> &'static str {
    match q {
        0 => "ordinary",
        1 => "well-crafted",
        2 => "fine",
        3 => "superior",
        4 => "exceptional",
        _ => "masterful",
    }
}

/// Spell a small ordinal — for naming squads "the First Company", "the Second
/// Company", and so on. Falls back to the numeral past what a fort will field.
pub fn ordinal(n: usize) -> &'static str {
    match n {
        1 => "First",
        2 => "Second",
        3 => "Third",
        4 => "Fourth",
        5 => "Fifth",
        6 => "Sixth",
        7 => "Seventh",
        8 => "Eighth",
        9 => "Ninth",
        _ => "Tenth",
    }
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
    /// Cuts rough gems into brilliant, valuable cut gems.
    Jeweler,
    /// Forges metal bars into weapons that arm the fort's soldiers.
    Forge,
    /// Smelts stone/ore boulders down into refined metal bars.
    Smelter,
    /// Cuts stone into furniture — beds for the fort's dwarves.
    Mason,
    /// Sews woven cloth into clothes for the fort to wear.
    Clothier,
    /// Works logs into wooden goods — barrels for the fort.
    Carpenter,
    /// Tans raw hides into leather.
    Tanner,
    /// A well: a thirsty dwarf can draw clean water from it when the fort has
    /// run dry of brewed drink.
    Well,
    /// Melts stone into blown glass — the fort's finest trade goods.
    GlassFurnace,
    /// A weapon trap: a raider that steps onto it is struck by hidden blades.
    Trap,
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
            BuildingKind::Jeweler => "Jeweler's Workshop",
            BuildingKind::Forge => "Forge",
            BuildingKind::Smelter => "Smelter",
            BuildingKind::Mason => "Mason's Workshop",
            BuildingKind::Clothier => "Clothier's Shop",
            BuildingKind::Carpenter => "Carpenter's Workshop",
            BuildingKind::Tanner => "Tanner's Shop",
            BuildingKind::Well => "Well",
            BuildingKind::GlassFurnace => "Glass Furnace",
            BuildingKind::Trap => "Weapon Trap",
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

/// The classes of goods a stockpile can be told to hold. Every item kind
/// belongs to exactly one (see `stock_category`), so a pile's filter is a set
/// of these — Dwarf Fortress's stockpile categories, pared to the goods our
/// fort actually produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum StockCategory {
    /// Meals, drink, crops, berries, seeds — everything a barrel takes.
    Food,
    /// Boulders: the mason's and mechanic's stock.
    Stone,
    /// Logs, before the carpenter has his way with them.
    Wood,
    /// Smelted bars.
    Bars,
    /// Worked goods: crafts, cloth, leather, gems, glass, clothes, instruments.
    Goods,
    /// Weapons and armor.
    Military,
    /// Beds, statues, and the fort's empty containers.
    Furniture,
    /// The dead, until they are buried.
    Refuse,
}

impl StockCategory {
    pub const ALL: [StockCategory; 8] = [
        StockCategory::Food,
        StockCategory::Stone,
        StockCategory::Wood,
        StockCategory::Bars,
        StockCategory::Goods,
        StockCategory::Military,
        StockCategory::Furniture,
        StockCategory::Refuse,
    ];

    pub fn name(self) -> &'static str {
        match self {
            StockCategory::Food => "food",
            StockCategory::Stone => "stone",
            StockCategory::Wood => "wood",
            StockCategory::Bars => "bars",
            StockCategory::Goods => "goods",
            StockCategory::Military => "arms",
            StockCategory::Furniture => "furniture",
            StockCategory::Refuse => "refuse",
        }
    }

    fn bit(self) -> u16 {
        1 << (self as u16)
    }
}

/// Which pile a kind of item belongs in. One category per kind — a stockpile
/// filter is then just a set of these.
pub fn stock_category(kind: ItemKind) -> StockCategory {
    match kind {
        ItemKind::Meal | ItemKind::Drink | ItemKind::Crop | ItemKind::Berry | ItemKind::Seed => {
            StockCategory::Food
        }
        ItemKind::Boulder => StockCategory::Stone,
        ItemKind::Log => StockCategory::Wood,
        ItemKind::Bar => StockCategory::Bars,
        ItemKind::Craft
        | ItemKind::BoneCraft
        | ItemKind::Cloth
        | ItemKind::Wool
        | ItemKind::Hide
        | ItemKind::Leather
        | ItemKind::RoughGem
        | ItemKind::CutGem
        | ItemKind::Glass
        | ItemKind::Clothes
        | ItemKind::Instrument
        | ItemKind::Artifact => StockCategory::Goods,
        ItemKind::Weapon | ItemKind::Armor | ItemKind::Shield => StockCategory::Military,
        ItemKind::Bed | ItemKind::Statue | ItemKind::Barrel | ItemKind::Bin => {
            StockCategory::Furniture
        }
        ItemKind::Corpse | ItemKind::BodyPart => StockCategory::Refuse,
    }
}

/// The set of categories a stockpile will take.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StockFilter(u16);

impl StockFilter {
    /// A pile that takes anything — what an undirected stockpile has always
    /// been, and still is unless the player says otherwise.
    pub fn any() -> Self {
        StockFilter(u16::MAX)
    }

    pub fn only(cats: &[StockCategory]) -> Self {
        StockFilter(cats.iter().fold(0, |m, c| m | c.bit()))
    }

    pub fn allows(self, cat: StockCategory) -> bool {
        self.0 & cat.bit() != 0
    }

    /// The categories this pile takes, for describing it to the player.
    pub fn categories(self) -> Vec<StockCategory> {
        StockCategory::ALL.into_iter().filter(|&c| self.allows(c)).collect()
    }

    pub fn takes_everything(self) -> bool {
        StockCategory::ALL.iter().all(|&c| self.allows(c))
    }
}

/// A stockpile: a rectangle of floor, and the classes of goods it will take.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stockpile {
    pub rect: Rect,
    pub accepts: StockFilter,
}

impl Stockpile {
    pub fn contains(&self, p: Pos) -> bool {
        self.rect.contains(p)
    }

    pub fn cells(&self) -> impl Iterator<Item = Pos> + '_ {
        self.rect.cells()
    }

    /// Would this pile take that item? A container belongs wherever the goods
    /// it carries belong — a barrel stands in the food pile it serves, not off
    /// in the furniture pile with the beds — so it is judged by its cargo.
    pub fn takes(&self, kind: ItemKind) -> bool {
        if is_container(kind) {
            // Strictly by cargo. A furniture pile must NOT take casks: a cask
            // hauled there is a cask `find_container_for` will never use,
            // because that asks whether the cask's pile wants the food. The
            // barrel would sit among the beds forever while the larder went
            // back to one meal per tile.
            return ItemKind::ALL
                .iter()
                .any(|&k| container_capacity(kind, k) > 0 && self.accepts.allows(stock_category(k)));
        }
        self.accepts.allows(stock_category(kind))
    }
}

// ----------------------------------------------------------------- animals

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AnimalKind {
    Cow,
    Sheep,
    /// A working dog: not raised for meat, but can be trained to guard the
    /// fort and fight off raiders.
    Dog,
    /// A cat: not raised for anything. It kills the vermin that eat the fort's
    /// food, which is the only reason a dwarf tolerates one.
    Cat,
}

/// The vermin that eat a fort's food.
///
/// Dwarf Fortress has 131 kinds of vermin and exactly SEVEN of them carry
/// `[VERMIN_EATER]` — "the vermin creature will attempt to eat exposed food".
/// The wiki's own prose says "many types feed on stockpiles"; its table says
/// seven. These are the seven, and which country each haunts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VerminKind {
    /// Evil country only. The worst of them, and it knows how to get into
    /// things.
    DemonRat,
    Rat,
    Hamster,
    LargeRoach,
    /// Savage country only.
    RhinoLizard,
    /// Hot country only.
    Lizard,
    /// Good country only. It is exactly as harmless as it sounds, and it still
    /// eats your food.
    FluffyWambler,
}

impl VerminKind {
    pub fn name(self) -> &'static str {
        match self {
            VerminKind::DemonRat => "demon rat",
            VerminKind::Rat => "rat",
            VerminKind::Hamster => "hamster",
            VerminKind::LargeRoach => "large roach",
            VerminKind::RhinoLizard => "two-legged rhino lizard",
            VerminKind::Lizard => "lizard",
            VerminKind::FluffyWambler => "fluffy wambler",
        }
    }

}

impl AnimalKind {
    pub fn name(self) -> &'static str {
        match self {
            AnimalKind::Cow => "cow",
            AnimalKind::Sheep => "sheep",
            AnimalKind::Dog => "dog",
            AnimalKind::Cat => "cat",
        }
    }

    /// Meals yielded when butchered as an adult.
    pub fn meat_yield(self) -> usize {
        match self {
            AnimalKind::Cow => 5,
            AnimalKind::Sheep => 3,
            AnimalKind::Dog => 1,
            // A dwarf would have to be very hungry indeed.
            AnimalKind::Cat => 1,
        }
    }

    /// Only dogs can be trained to war — livestock cannot.
    pub fn trainable(self) -> bool {
        matches!(self, AnimalKind::Dog)
    }

    /// Combat health when this animal fights (a war dog's hardiness).
    pub fn war_hp(self) -> i16 {
        match self {
            AnimalKind::Dog => 45,
            _ => 30,
        }
    }
}

/// A single vermin, skulking about the fort.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Vermin {
    pub kind: VerminKind,
    pub pos: Pos,
    pub alive: bool,
    move_cd: u8,
    /// Ticks until this one gets another mouthful. Its OWN clock, not the
    /// world's: a vermin only reaches the eat check on its action ticks (one
    /// in seven, thanks to `move_cd`), so gating on `tick % INTERVAL == 0`
    /// sampled a coincidence that almost never happened — the real rate came
    /// out seven times slower than the constant said.
    eat_cd: u64,
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
    /// A trainer has been told to war-train this animal (dogs only).
    pub war_marked: bool,
    /// A trained war animal: it guards the fort and fights raiders.
    pub war: bool,
    /// Combat health while fighting; when it hits 0 the animal falls.
    pub hp: i16,
    /// Attack cadence while fighting.
    atk_cd: u8,
    /// Dwarf index that has claimed this animal for butchering or training.
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

// Weapons are data now — see `dk_raws::WeaponRegistry` (`data/weapons/*.ron`).
// A `Weapon` item stores its kind as an index into that registry in `variant`;
// combat reads `raws.weapons.damage_type/heft/verb/is_ranged(variant)`, the
// forge draws from `raws.weapons.melee_indices()`, and `DamageType` is re-
// exported from dk_raws above.

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

/// A monstrous body: many times the toughness of a mortal frame.
fn beast_body() -> Vec<BodyPart> {
    let part = |kind: PartKind, hp: i16| BodyPart { kind, hp, max_hp: hp, bleeding: 0 };
    vec![
        part(PartKind::Head, 90),
        part(PartKind::Torso, 180),
        part(PartKind::LeftArm, 110),
        part(PartKind::RightArm, 110),
        part(PartKind::LeftLeg, 110),
        part(PartKind::RightLeg, 110),
    ]
}

/// A lesser cavern dweller — hardier than a dwarf, but far short of a forgotten
/// beast. The wildlife that makes the caverns a place to fear.
fn cave_beast_body() -> Vec<BodyPart> {
    let part = |kind: PartKind, hp: i16| BodyPart { kind, hp, max_hp: hp, bleeding: 0 };
    vec![
        part(PartKind::Head, 45),
        part(PartKind::Torso, 95),
        part(PartKind::LeftArm, 55),
        part(PartKind::RightArm, 55),
        part(PartKind::LeftLeg, 55),
        part(PartKind::RightLeg, 55),
    ]
}

/// The kinds of creature that lurk in the caverns.
const CAVE_CREATURES: [&str; 8] = [
    "giant cave spider",
    "cave crawler",
    "blind cave ogre",
    "troglodyte",
    "giant bat",
    "cave fisher",
    "rutherer",
    "crundle",
];

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
    PrayedAtTemple,
    /// Woke in a bed of one's own, in a room of one's own.
    SleptInOwnRoom,
    /// Woke in a bed, but out in the open where anyone might tread.
    SleptInBed,
    /// Woke on the bare stone, as no dwarf should have to.
    SleptOnFloor,
    /// Ate in the hall, in company, like a dwarf and not a dog.
    DinedInHall,
    /// Walked past food someone left to rot.
    SmelledRot,
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
            ThoughtKind::PrayedAtTemple => 5.0,
            ThoughtKind::SleptInOwnRoom => 5.0,
            ThoughtKind::SleptInBed => 2.0,
            ThoughtKind::SleptOnFloor => -4.0,
            ThoughtKind::DinedInHall => 3.0,
            ThoughtKind::SmelledRot => -5.0,
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
            ThoughtKind::PrayedAtTemple => "found peace in prayer",
            ThoughtKind::SleptInOwnRoom => "slept in a fine bedroom of their own",
            ThoughtKind::SleptInBed => "slept in a bed",
            ThoughtKind::SleptOnFloor => "slept on the cold hard stone",
            ThoughtKind::DinedInHall => "dined in the great hall",
            ThoughtKind::SmelledRot => "gagged on the stench of rotting food",
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
    /// Prowess in melee, honed by drawing blood. Raises the force of a blow.
    Fighting,
}

/// The extra damage a fighter of the given skill level (0..=6) adds to each
/// blow. A seasoned veteran hits markedly harder than a green recruit.
pub fn fighting_bonus(level: u32) -> i16 {
    level as i16 * 2
}

/// The outcome of one landed blow: how much the struck part loses, and how
/// hard it bleeds.
pub struct BlowResult {
    pub damage: i16,
    pub bleed: u8,
}

/// Resolve a single landed blow — the heart of combat, and where Dwarf
/// Fortress's three damage types earn their keep.
///
/// `force` is the raw power behind the swing (skill + a beast's monstrous
/// strength). The weapon supplies a damage type, a heft, and its material; the
/// armour, if any, supplies its own material. Everything else falls out of how
/// those meet:
///
/// - **Edge** cuts with the weapon's sharpness. A better metal defeats a
///   lesser armour, and an equal one is largely turned — which is why a sword
///   glances off plate. It draws the most blood.
/// - **Pierce** is a narrow edge: it concentrates, defeating armour better than
///   a slash and punching a deep wound, but spilling less blood.
/// - **Blunt** ignores the edge entirely and drives its mass through the
///   armour into the flesh. Armour blunts it but never stops it — a war hammer
///   hurts a plated dwarf where a sword would ring off — and it breaks bone
///   rather than opening veins.
///
/// `weapon = None` is a bare fist: a feeble blunt tap.
pub fn resolve_blow(
    force: f32,
    weapon: Option<(DamageType, f32, CombatStats)>,
    armor: Option<CombatStats>,
) -> BlowResult {
    let (dtype, heft, wmat) = weapon.unwrap_or((
        DamageType::Blunt,
        0.5,
        CombatStats { sharpness: 0.1, density: 1.0, hardness: 10.0 },
    ));
    let armor_hard = armor.map_or(0.0, |a| a.hardness);
    let armor_dens = armor.map_or(0.0, |a| a.density);

    // A soft blade holds no edge: below iron it dulls and folds, so a copper
    // sword bites worse than an iron one however keen its geometry. Past iron
    // it stops mattering — sharpness carries the harder metals — so the temper
    // only ever penalises, never rewards.
    let temper = (wmat.hardness / 100.0).min(1.0);
    let (raw, bleed) = match dtype {
        DamageType::Edge => {
            // Cut deepens with the blade's keenness; armour hardness turns it.
            let cut = force * wmat.sharpness * temper;
            let turned = armor_hard * 0.12;
            (cut - turned, 3u8)
        }
        DamageType::Pierce => {
            // Narrower and more concentrated: defeats armour better, bleeds
            // less, but the point drives deep.
            let punch = force * wmat.sharpness * temper * 1.15;
            let turned = armor_hard * 0.08;
            (punch - turned, 2u8)
        }
        DamageType::Blunt => {
            // Mass behind the head, transmitted through the armour. Iron is
            // density ~7.8, so a mid-weight weapon of it hits near its raw
            // force; armour only softens the blow, never negates it.
            let mass = heft * (wmat.density / 7.8).max(0.3);
            let hit = force * mass;
            let softened = armor_hard * 0.06 + armor_dens * 0.4;
            // At least 40% of a crush always reaches the bone.
            ((hit - softened).max(hit * 0.4), 1u8)
        }
    };
    // A landed blow always does something and always draws at least a little
    // blood — no hit is truly harmless.
    let damage = raw.round().max(1.0) as i16;
    BlowResult { damage, bleed: bleed.max(1) }
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
    /// Carve a skeletonized body part into a bone trinket at the craftsdwarf's.
    BoneCraft,
    /// Weave raw wool into cloth.
    Weave,
    /// Cut a rough gem into a brilliant one.
    CutGem,
    /// Forge a metal bar into a weapon.
    ForgeWeapon,
    /// Smelt a stone/ore boulder down into a refined metal bar.
    Smelt,
    /// Forge a metal bar into a suit of armor.
    ForgeArmor,
    /// Work a bar into a shield at the forge.
    ForgeShield,
    /// Forge a metal bar into a crossbow — the marksdwarf's arm.
    ForgeCrossbow,
    /// Forge a metal bar into a quiver of crossbow bolts.
    ForgeBolts,
    /// Work a stone boulder into a piece of furniture (a bed).
    MakeFurniture,
    /// Sew a bolt of cloth into clothes.
    SewClothes,
    /// Work a log into a barrel at the carpenter's shop.
    MakeBarrel,
    /// Work a log into a bin at the carpenter's shop.
    MakeBin,
    /// Carve a stone boulder into a statue at the mason's workshop.
    CarveStatue,
    /// Work a log into a musical instrument at the carpenter's shop.
    MakeInstrument,
    /// Tan a raw hide into leather at the tanner's shop.
    TanHide,
    /// Melt a boulder into a piece of blown glass.
    MakeGlass,
    /// Work a log into a wooden bed at the carpenter's shop — furniture in the
    /// log's own wood.
    MakeWoodBed,
    /// Carve a log into a wooden statue at the carpenter's shop — art in the
    /// log's own wood.
    CarveWoodStatue,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Task {
    Idle { wander_cd: u16 },
    Sleep { remaining: u16 },
    /// Trudging to one's own bed to sleep in it.
    GoToBed { bed: usize, path: Vec<Pos> },
    /// Carrying a meal to the dining hall to eat it in company.
    DineAt { item: usize, path: Vec<Pos> },
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
    /// Walk to a marked dog and train it for war over time.
    Train { animal: usize, path: Vec<Pos>, progress: u16 },
    /// Fetch a boulder and raise a wall on a planned construction tile.
    Build { site: Pos, input: usize, path: Vec<Pos>, stage: FetchStage, progress: u16 },
    /// Unwind at the tavern: walk there, drink, socialize, shed stress.
    Relax { spot: Pos, path: Vec<Pos>, remaining: u16, drank: bool },
    /// Fish at a bank tile beside water until something bites.
    Fish { spot: Pos, path: Vec<Pos>, progress: u16 },
    /// Worship at the temple: walk there and pray a while for solace.
    Pray { spot: Pos, path: Vec<Pos>, remaining: u16 },
    /// Rest in a hospital until wounds mend.
    Recover { spot: Pos, path: Vec<Pos>, remaining: u16 },
    /// Drill at the barracks, honing the fighting skill.
    Spar { spot: Pos, path: Vec<Pos>, remaining: u16 },
    /// Hold a post: a stationed soldier marches to its point and stands guard,
    /// striking only what strays near (the hunt loop handles engagement).
    Station { spot: Pos, path: Vec<Pos> },
    /// Walk a beat: march between two points, flipping ends on arrival. The
    /// hunt loop breaks off to strike what strays near the route.
    Patrol { a: Pos, b: Pos, toward_b: bool, path: Vec<Pos> },
    /// Shelter in a burrow while the alarm sounds.
    Shelter { spot: Pos, path: Vec<Pos> },
    /// Walk to a marked tree and fell it for a log.
    Chop { tree: Pos, path: Vec<Pos>, progress: u16 },
    /// Walk to a marked shrub and forage it for berries.
    Gather { shrub: Pos, path: Vec<Pos>, progress: u16 },
    /// Walk to a well and drink clean water (a fallback when brewed drink is
    /// gone).
    DrinkWell { spot: Pos, path: Vec<Pos> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dwarf {
    pub name: String,
    pub pos: Pos,
    pub alive: bool,
    pub faction: Faction,
    /// The bed this dwarf calls their own (an item index), once they have
    /// claimed one. A dwarf sleeps in their own bed and nobody else's.
    pub bed: Option<usize>,
    pub hunger: f32,
    pub thirst: f32,
    pub fatigue: f32,
    pub happiness: f32,
    /// 0-100; bleeding drains it, running out is fatal.
    pub blood: f32,
    /// Blood on this creature's feet, tracked from a pool and left as fading
    /// footprints as it walks. 0 when clean.
    #[serde(default)]
    pub blood_tracked: u16,
    /// Where it stood last step, to tell whether it moved (for footprints).
    #[serde(default)]
    pub last_pos: Pos,
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
    /// A forgotten beast from the deep — vastly tougher, hits far harder.
    pub beast: bool,
    /// Enlisted: proactively hunts hostiles instead of only defending when
    /// one walks adjacent.
    pub soldier: bool,
    /// Tick of this dwarf's last prayer, for the worship cadence.
    pub last_prayer: u64,
    /// Adventure mode: a recruited companion who shadows the hero, fights at
    /// their side, and journeys with them from land to land.
    pub follower: bool,
    /// A secret night-creature. Looks and works like any other dwarf, but
    /// never hungers or thirsts — it sustains itself on the blood of sleeping
    /// fort-mates, and does not age or die of its needs.
    pub vampire: bool,
    /// Tick a vampire last fed, pacing its hunt for blood.
    pub last_fed: u64,
    /// A werebeast: an ordinary dwarf by day, but under the full moon it twists
    /// into a snarling beast and turns on the fort. The curse spreads by its bite.
    pub werebeast: bool,
    /// Currently transformed into the beast (only true during a full moon).
    pub were_form: bool,
    /// A necromancer (a hostile): raises the fort's fallen dead as undead.
    pub necromancer: bool,
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
            Task::GoToBed { .. } => "going to bed",
            Task::DineAt { .. } => "carrying food to the hall",
            Task::Mine { .. } => "mining",
            Task::Haul { .. } => "hauling",
            Task::Eat { .. } => "getting food",
            Task::Drink { .. } => "getting a drink",
            Task::Plant { .. } => "planting",
            Task::Harvest { .. } => "harvesting",
            Task::Craft { kind: CraftKind::Brew, .. } => "brewing",
            Task::Craft { kind: CraftKind::Cook, .. } => "cooking",
            Task::Craft { kind: CraftKind::Stonecraft, .. } => "crafting",
            Task::Craft { kind: CraftKind::BoneCraft, .. } => "carving bone",
            Task::Craft { kind: CraftKind::Weave, .. } => "weaving",
            Task::Craft { kind: CraftKind::CutGem, .. } => "cutting gems",
            Task::Craft { kind: CraftKind::ForgeWeapon, .. } => "forging a weapon",
            Task::Craft { kind: CraftKind::Smelt, .. } => "smelting",
            Task::Craft { kind: CraftKind::ForgeArmor, .. } => "forging armor",
            Task::Craft { kind: CraftKind::ForgeShield, .. } => "forging a shield",
            Task::Craft { kind: CraftKind::ForgeCrossbow, .. } => "forging a crossbow",
            Task::Craft { kind: CraftKind::ForgeBolts, .. } => "forging bolts",
            Task::Craft { kind: CraftKind::MakeFurniture, .. } => "building furniture",
            Task::Craft { kind: CraftKind::SewClothes, .. } => "sewing clothes",
            Task::Craft { kind: CraftKind::MakeBarrel, .. } => "making a barrel",
            Task::Craft { kind: CraftKind::MakeBin, .. } => "making a bin",
            Task::Craft { kind: CraftKind::CarveStatue, .. } => "carving a statue",
            Task::Craft { kind: CraftKind::MakeInstrument, .. } => "making an instrument",
            Task::Craft { kind: CraftKind::TanHide, .. } => "tanning leather",
            Task::Chop { .. } => "chopping wood",
            Task::Gather { .. } => "gathering plants",
            Task::DrinkWell { .. } => "drawing water",
            Task::Craft { kind: CraftKind::MakeGlass, .. } => "blowing glass",
            Task::Craft { kind: CraftKind::MakeWoodBed, .. } => "building a wooden bed",
            Task::Craft { kind: CraftKind::CarveWoodStatue, .. } => "carving a wooden statue",
            Task::Fight { .. } => "attacking",
            Task::Tantrum { .. } => "throwing a tantrum",
            Task::Sulk { .. } => "sulking",
            Task::StrangeMood { .. } => "in a strange mood!",
            Task::Butcher { .. } => "butchering",
            Task::Train { .. } => "training a war dog",
            Task::Build { .. } => "building a wall",
            Task::Relax { .. } => "relaxing at the tavern",
            Task::Fish { .. } => "fishing",
            Task::Pray { .. } => "praying at the temple",
            Task::Recover { .. } => "resting in the hospital",
            Task::Spar { .. } => "drilling at the barracks",
            Task::Station { .. } => "holding a post",
            Task::Patrol { .. } => "walking a patrol",
            Task::Shelter { .. } => "sheltering from the raid",
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
    pub beasts_slain: u32,
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
    pub gems_found: u32,
    pub gems_cut: u32,
    pub weapons_forged: u32,
    pub glass_blown: u32,
    /// Fort-mates found drained of blood — the mark of a vampire.
    pub drained: u32,
    /// Metal bars smelted from ore at the smelter.
    pub bars_smelted: u32,
    /// Suits of armor forged for the fort's soldiers.
    pub armor_forged: u32,
    pub shields_forged: u32,
    /// Crossbows forged for the marksdwarves.
    pub crossbows_forged: u32,
    /// Bolts forged (in quivers) for the fort's ammo stock.
    pub bolts_forged: u32,
    /// Bolts loosed at the enemy by marksdwarves.
    pub bolts_fired: u32,
    /// Pieces of furniture (beds) built at the mason's workshop.
    pub furniture_made: u32,
    /// Sets of clothes sewn at the clothier's shop.
    pub clothes_sewn: u32,
    /// Trees felled by woodcutters.
    pub trees_felled: u32,
    /// Wild shrubs foraged for berries.
    pub foraged: u32,
    /// Demons loosed from the underworld by digging too deep.
    pub demons_loosed: u32,
    /// Barrels worked from logs at the carpenter's shop.
    pub barrels_made: u32,
    /// Bins worked from logs at the carpenter's shop.
    pub bins_made: u32,
    /// Food that turned for want of a stockpile to keep it in.
    pub food_spoiled: u32,
    /// Food carried off by vermin that got at it.
    pub food_gnawed: u32,
    /// Vermin the fort's cats have killed.
    pub vermin_slain: u32,
    /// Statues carved at the mason's workshop.
    pub statues_carved: u32,
    /// Instruments crafted at the carpenter's shop.
    pub instruments_made: u32,
    /// Hides tanned into leather at the tanner's shop.
    pub leather_tanned: u32,
    /// Corpses raised as undead by a necromancer.
    pub raised: u32,
    /// Total trade value of goods bought from caravans over the fort's life.
    pub value_imported: u64,
    /// Total trade value of goods sold to caravans over the fort's life.
    pub value_exported: u64,
}

/// Which discretionary crafts the fort wants more of this assignment pass,
/// computed once and handed to `assign_one`. A named struct rather than a row
/// of positional bools so the flags can never be silently transposed at the
/// call site.
#[derive(Clone, Copy, Default)]
struct Wants {
    drinks: bool,
    meals: bool,
    crafts: bool,
    bone_crafts: bool,
    weapons: bool,
    crossbows: bool,
    bolts: bool,
    glass: bool,
    bars: bool,
    armor: bool,
    shields: bool,
    furniture: bool,
    clothes: bool,
    barrels: bool,
    bins: bool,
    statues: bool,
    instruments: bool,
    leather: bool,
    wood_beds: bool,
    wood_statues: bool,
}

// -------------------------------------------------------------- adventure

/// One turn's worth of player intent in adventure mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerAction {
    Move(i32, i32),
    /// +1 up / -1 down through stairs.
    Climb(i32),
    Wait,
    /// Pick up a loose item under the hero's feet (e.g. a fallen foe's blade).
    Grab,
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
    /// A DF-faithful noble whim: forbid the sale of a material to caravans for
    /// the mandate's span. The banned material is the mandate's `target`; selling
    /// a good of it defies the baron and is punished when the edict lapses.
    ExportBan,
}

/// A baron's demand. For production kinds: make `amount` of something before
/// `deadline` (progress = current stat - `baseline`). For an `ExportBan`: keep
/// the `target` material off the caravans until `deadline`; `violated` records a
/// defiant sale.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Mandate {
    pub kind: MandateKind,
    pub amount: u32,
    pub deadline: u64,
    /// Stat value when the mandate was issued (progress = current - baseline).
    pub baseline: u32,
    /// The banned material index, for an `ExportBan` (unused otherwise).
    #[serde(default)]
    pub target: u16,
    /// Set when the fort defies an `ExportBan` by selling the banned material.
    #[serde(default)]
    pub violated: bool,
}

impl Mandate {
    /// A player-facing description of the demand.
    pub fn describe(&self, raws: &Raws) -> String {
        match self.kind {
            MandateKind::CookMeals => format!("{} meals be cooked", self.amount),
            MandateKind::BrewDrinks => format!("{} drinks be brewed", self.amount),
            MandateKind::MineBoulders => format!("{} boulders be mined", self.amount),
            MandateKind::ExportBan => {
                format!("no {} leave the fort by caravan", raws.materials.get(self.target).name)
            }
        }
    }
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

/// Trade value of an item, in a common coin. Prices come from the data-driven
/// economy table (`data/economy/prices.ron`); a good is worth
/// `material.value * mat_coeff + flat`, then quality is layered on. Rough and
/// cut gems are the deliberate exception — priced by rarity tier in code.
pub fn item_value(item: &Item, raws: &Raws) -> u32 {
    let base = match item.kind {
        // A rough gem, worth more the rarer the stone; a cut gem is the fort's
        // finest legitimate trade good — a cut diamond (tier 6) fetches far more
        // than a cut agate (tier 1). Priced by tier, not a material multiplier,
        // so no price row governs them.
        ItemKind::RoughGem => 4 * raws.gems.value_tier(item.stuff),
        ItemKind::CutGem => 24 * raws.gems.value_tier(item.stuff),
        // Everything else: material value scaled by the good's coefficient, plus
        // a flat worth. For flat goods (a meal, a bolt of cloth) mat_coeff is 0,
        // so the material term drops out — matching the old hardcoded prices.
        // A container's own worth is priced here; what it holds is valued
        // separately by `stack_value`, so nobody sells a barrel of wine for the
        // price of the barrel. A missing row (impossible after `validate_economy`
        // passes at load) prices at 0 rather than panicking in the render path.
        kind => raws
            .economy
            .price(kind.key())
            .map_or(0, |p| raws.materials.get(item.stuff).value * p.mat_coeff + p.flat),
    };
    // Craftsdwarfship raises the worth: a masterwork (tier 5) is worth 3.5x.
    base + base * item.quality as u32 / 2
}

/// Every priced item kind must have a row in the economy table, or the fort
/// would silently value it at nothing. Called once at load so a mistyped or
/// missing `data/economy/prices.ron` entry fails loudly at startup instead of
/// quietly zeroing a good's worth mid-game. Gems are priced in code, so they
/// are exempt.
pub fn validate_economy(raws: &Raws) -> Result<()> {
    for kind in ItemKind::ALL {
        if matches!(kind, ItemKind::RoughGem | ItemKind::CutGem) {
            continue;
        }
        anyhow::ensure!(
            raws.economy.price(kind.key()).is_some(),
            "economy price list is missing a row for {} (see data/economy/prices.ron)",
            kind.key()
        );
    }
    Ok(())
}

// ---------------------------------------------------------------- wealth

/// Summed trade value per extra raider in a siege wave. A fort worth this much
/// draws one more attacker, up to the cap. Tuned against real material values
/// (see docs/economy-scope.md): 50 common boulders is ~150, a masterwork gold
/// statue ~1715, a fort with a real metal-and-gem industry several thousand —
/// so gravel goes unnoticed and only made wealth invites a siege. Deliberately
/// generous; expect to retune it once forts are played to maturity.
pub const WEALTH_PER_RAIDER: u32 = 1500;

/// Summed trade value per extra migrant in a wave — wealth draws settlers the
/// way it draws raiders. Only applied in a live world (see `maybe_migrants`).
pub const WEALTH_PER_MIGRANT: u32 = 1500;

/// How many raiders a siege brings, by the fort's wealth. A poor fort draws a
/// token raid (one); a rich one draws a wave, capped at five so a masterwork
/// hoard can't summon an endless horde. Pure and total so it can be unit-tested
/// without spawning anything.
pub fn raider_wave_size(wealth: u32) -> u32 {
    (1 + wealth / WEALTH_PER_RAIDER).min(5)
}

// ---------------------------------------------------------------- sieges


/// What a squad is doing. A player's command over their soldiers, where before
/// every soldier hunted every hostile on sight with no say in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SquadOrder {
    /// Hunt down any hostile in the fort. The old always-on behaviour, now a
    /// choice.
    Defend,
    /// Hold a post: march to a point and guard it, striking only what comes
    /// near. A soldier stationed on a bridge does not chase a beast across the
    /// map and leave the gate open.
    Station(Pos),
    /// Walk a beat between two points, back and forth, striking what strays
    /// near the route — a moving guard for a wall or corridor.
    Patrol(Pos, Pos),
    /// Drill at the barracks. Train through peacetime; still defends itself if
    /// something walks up, but does not go looking for a fight.
    Train,
}

/// How a squad is armed — Dwarf Fortress's uniform, pared to the one choice
/// that changes how a soldier fights: melee steel, or the crossbow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Uniform {
    /// Sword, axe, spear, mace, or hammer — close and strike.
    #[default]
    Melee,
    /// Crossbow and bolts — a marksdwarf who fires from afar and falls back to
    /// a bash only when a foe closes in.
    Ranged,
}

/// A squad: the fort's soldiers, organised. Dwarf Fortress caps a squad at ten
/// under one commander; this is that, pared to what a small fort needs — a
/// name, its members, one standing order, and how it is armed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Squad {
    pub name: String,
    /// Dwarf indices. The first is the squad's commander.
    pub members: Vec<usize>,
    pub order: SquadOrder,
    #[serde(default)]
    pub uniform: Uniform,
}

/// The most soldiers in one squad, as in Dwarf Fortress.
pub const SQUAD_MAX: usize = 10;
/// How near a hostile must come to a stationed squad before it strikes.
pub const STATION_ENGAGE_RANGE: u32 = 10;
/// How far a marksdwarf's bolt carries (tiles). A crossbow outranges any
/// melee reach, so a marksdwarf opens fire long before a foe can close.
pub const RANGED_RANGE: u32 = 12;
/// Bolts forged per job — a quiver's worth, so the armoury isn't run ragged
/// making ammo one bolt at a time.
pub const QUIVER: u32 = 25;

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
    pub temples: Vec<Rect>,
    /// Hospital zones: wounded dwarves rest here and mend faster.
    pub hospitals: Vec<Rect>,
    /// Barracks zones: enlisted soldiers spar here to hone their fighting.
    pub barracks: Vec<Rect>,
    /// The fort's squads. A soldier belongs to exactly one.
    pub squads: Vec<Squad>,
    /// The fort's stock of crossbow bolts — a shared quiver the marksdwarves
    /// draw from. Consumable ammo is a pool (unlike the durable arms, which are
    /// items); forged in quivers, spent one bolt per shot.
    #[serde(default)]
    pub bolts: u32,
    /// Burrow zones: safe rooms civilians retreat to when the alarm sounds.
    pub burrows: Vec<Rect>,
    /// Whether the civilian alarm is sounded (retreat to the burrows).
    pub alarm: bool,
    /// Library zones: with one, the fort's scholars set down treatises.
    pub library: Vec<Rect>,
    /// Rooms set aside for sleeping. A bed inside one is a bedroom of its
    /// own, and its owner wakes the better for it.
    pub bedrooms: Vec<Rect>,
    /// The hall where the fort eats together.
    pub dining: Vec<Rect>,
    /// Scholarly works the fort has written — its accumulated knowledge.
    pub treatises: Vec<String>,
    pub fisheries: Vec<Rect>,
    pub animals: Vec<Animal>,
    pub buildings: Vec<Building>,
    pub farms: BTreeMap<Pos, FarmTile>,
    pub designations: BTreeMap<Pos, Designation>,
    /// Engraved wall faces: position -> the scene carved there. Grows as
    /// dwarves smooth walls; read by the UI to describe and tint them.
    pub engravings: BTreeMap<Pos, String>,
    /// Planned constructions: a walkable tile marked to become a wall.
    /// `true` once a builder has claimed it. A dwarf hauls a boulder over and
    /// raises the wall.
    pub constructions: BTreeMap<Pos, bool>,
    /// Trees standing on the surface, each mapped to its wood species (a
    /// `MaterialCategory::Wood` material index). Passable, but a woodcutter can
    /// fell one (designate Chop) for a log of that wood. Empty unless a map is
    /// planted with them.
    pub trees: BTreeMap<Pos, u16>,
    /// Ceiling on natural forest regrowth, set when a map is seeded — a felled
    /// woodland regrows toward this, but never past its original density. Zero
    /// unless `plant_trees` seeded the map, so hand-built test forts never
    /// regrow (keeping the headless suite byte-identical).
    pub tree_cap: usize,
    /// Wild berry shrubs on the surface. Passable; a forager can gather one
    /// (designate Gather) for edible berries, and a tended patch reseeds itself
    /// over time. Empty unless a map is planted with them.
    pub shrubs: BTreeSet<Pos>,
    /// Ceiling on natural shrub regrowth, set when a map is seeded — a berry
    /// patch spreads but never overruns the map.
    pub shrub_cap: usize,
    /// Adamantine tiles that cap the underworld — mining one breaches the abyss
    /// and demons pour out. Empty unless a map was seeded with adamantine (app
    /// embark only), so a fort that never digs one stays byte-identical.
    pub adamantine_breaches: BTreeSet<Pos>,
    /// Solid tiles that lie in a water-bearing aquifer layer: mine one and the
    /// opened tile becomes a spring that seeps until it is walled off. Empty
    /// unless a map was seeded with an aquifer (app embark only), so a fort with
    /// no aquifer stays byte-identical.
    #[serde(default)]
    pub aquifers: BTreeSet<Pos>,
    /// Floor tiles of the great cavern layer dug deep in the rock — lit with a
    /// fungal glow. Empty unless the map was carved with a cavern (app embark
    /// only, and only on full-size maps).
    #[serde(default)]
    pub cavern_floors: BTreeSet<Pos>,
    /// Blood spilled on the ground, by tile -> intensity (0..BLOOD_MAX). Wounds
    /// drip it, deaths pool it; it dries and fades over a day or so. Purely
    /// cosmetic — placed deterministically (no rng), so combat stays identical.
    #[serde(default)]
    pub blood: BTreeMap<Pos, u16>,
    pub stats: SimStats,
    pub clock: Calendar,
    pub weather: Weather,
    /// Tick the last citizen died — the fortress has fallen. `None` while
    /// it still lives.
    pub fallen_at: Option<u64>,
    pub water: WaterSim,
    /// The second fluid: slower, hotter, considerably less forgiving.
    pub magma: FluidSim,
    /// Adventure mode: index of the player-controlled creature, if any.
    pub player: Option<usize>,
    /// The overworld region this fortress was founded on (set by the app at
    /// embark). Used to retire the fort back to its own place in the world.
    pub home_region: Option<(usize, usize)>,
    /// Adventure mode: the overworld region the hero currently stands in
    /// (set by the app; used to drive region travel).
    pub adv_region: Option<(usize, usize)>,
    /// Adventure quest: (target name, completed).
    pub quest: Option<(String, bool)>,
    /// Index of the spawned nemesis, so completion checks the right body.
    pub quest_target: Option<usize>,
    /// Notable feats accomplished by the player.
    pub deeds: Vec<String>,
    /// Named poetic works composed at the fort's taverns — its living culture.
    pub poems: Vec<String>,
    /// Songs the fort has composed and played at the tavern (needs an instrument).
    pub songs: Vec<String>,
    /// Who attacks this fort and why — wired from world history at embark.
    pub siege_roster: Option<SiegeRoster>,
    /// Friendly civ that sends caravans — wired from world history at embark.
    pub trade_partner: Option<String>,
    /// The world year this fort was founded in (set by the app at embark).
    /// `sync_world` reads it to keep the world's clock and the fort's clock
    /// in step. Left 0 for a sim with no world behind it, which then never
    /// advances history.
    pub embark_world_year: u32,
    /// The caravan currently visiting, if any.
    pub caravan: Option<Caravan>,
    /// The fort's created wealth — the summed trade value of everything it
    /// owns — refreshed on the day boundary (`recompute_wealth`) and read by the
    /// HUD, the siege planner, and the migration pull. Cached because a live
    /// recompute every frame would scan every item; a day-old figure is plenty
    /// for a number that only nudges migration and raid size.
    #[serde(default)]
    pub cached_wealth: u32,
    /// Killing traders has consequences: no caravans until this tick.
    pub trade_ban_until: u64,
    /// Standing goodwill with the trade partner, in trade value. Over-pay a
    /// caravan and the surplus is banked here instead of thrown away; a later
    /// trade can draw it down to pay for goods. Always >= 0 this phase (the
    /// fort never owes the caravan — the accept check forbids it); typed i64
    /// for headroom should a debt mechanic ever land. Counts toward fortress
    /// wealth, so banking can't be used to hide masterworks from raiders.
    #[serde(default)]
    pub trade_credit: i64,
    /// Set when a hostile (not the fort) kills a trader — the caravan
    /// scatters but the civ blames the raiders, not you.
    trader_lost_to_raiders: bool,
    /// The vermin that infest this fort — set from the region it embarked on,
    /// and the reason a larder wants barrels and a cat.
    pub vermin: Vec<Vermin>,
    /// What kind of vermin this country breeds, if any. Set by the app at
    /// embark from the region's biome and alignment; `None` leaves the fort
    /// (and every headless test) entirely vermin-free.
    pub vermin_kind: Option<VerminKind>,
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
        if let Some(spring) = natural_spring(&map) {
            water.springs.insert(spring);
        }

        Sim {
            map,
            dwarves,
            items: Vec::new(),
            stockpiles: Vec::new(),
            bedrooms: Vec::new(),
            dining: Vec::new(),
            pastures: Vec::new(),
            taverns: Vec::new(),
            temples: Vec::new(),
            hospitals: Vec::new(),
            barracks: Vec::new(),
            squads: Vec::new(),
            bolts: 0,
            burrows: Vec::new(),
            alarm: false,
            library: Vec::new(),
            treatises: Vec::new(),
            fisheries: Vec::new(),
            animals: Vec::new(),
            buildings: Vec::new(),
            farms: BTreeMap::new(),
            designations: BTreeMap::new(),
            trees: BTreeMap::new(),
            tree_cap: 0,
            shrubs: BTreeSet::new(),
            shrub_cap: 0,
            adamantine_breaches: BTreeSet::new(),
            aquifers: BTreeSet::new(),
            cavern_floors: BTreeSet::new(),
            blood: BTreeMap::new(),
            engravings: BTreeMap::new(),
            constructions: BTreeMap::new(),
            stats: SimStats::default(),
            clock: Calendar::default(),
            weather: Weather::Clear,
            fallen_at: None,
            water,
            magma,
            player: None,
            home_region: None,
            adv_region: None,
            quest: None,
            quest_target: None,
            deeds: Vec::new(),
            poems: Vec::new(),
            songs: Vec::new(),
            siege_roster: None,
            trade_partner: None,
            embark_world_year: 0,
            caravan: None,
            cached_wealth: 0,
            trade_ban_until: 0,
            trade_credit: 0,
            trader_lost_to_raiders: false,
            vermin: Vec::new(),
            vermin_kind: None,
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
                    // Engrave a solid wall that isn't already engraved.
                    DesignationKind::Smooth => {
                        tile.is_solid() && !self.engravings.contains_key(&p)
                    }
                    // Fell a tree standing on this tile.
                    DesignationKind::Chop => self.trees.contains_key(&p),
                    // Forage a wild shrub standing on this tile.
                    DesignationKind::Gather => self.shrubs.contains(&p),
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

    /// Plan a constructed wall on a walkable tile. Fails if the tile is solid,
    /// already carries a building/farm/designation/plan, or has no solid
    /// neighbour to key the masonry to would be fine — we allow open sites.
    pub fn designate_construction(&mut self, p: Pos) -> bool {
        let Some(tile) = self.map.tile_at(p) else { return false };
        if tile.is_solid()
            || self.building_at(p).is_some()
            || self.farms.contains_key(&p)
            || self.designations.contains_key(&p)
            || self.constructions.contains_key(&p)
            // A wall raised over a standing shrub or tree would seal it in.
            || self.shrubs.contains(&p)
            || self.trees.contains_key(&p)
        {
            return false;
        }
        self.constructions.insert(p, false);
        true
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
                        // Stop a digger OR a woodcutter already working this tile
                        // — cancelling must actually cancel (Chop was missing).
                        if matches!(self.dwarves[i].task, Task::Mine { target, .. } if target == p)
                            || matches!(self.dwarves[i].task, Task::Chop { tree, .. } if tree == p)
                            || matches!(self.dwarves[i].task, Task::Gather { shrub, .. } if shrub == p)
                        {
                            self.abandon_task(i);
                        }
                    }
                }
                if self.constructions.remove(&p).is_some() {
                    removed += 1;
                    for i in 0..self.dwarves.len() {
                        if matches!(self.dwarves[i].task, Task::Build { site, .. } if site == p) {
                            self.abandon_task(i);
                        }
                    }
                }
            }
        }
        removed
    }

    /// A pile that takes whatever is brought to it.
    pub fn add_stockpile(&mut self, a: Pos, b: Pos) {
        self.add_filtered_stockpile(a, b, StockFilter::any());
    }

    /// A pile told what it is for: a food pile takes no boulders, and a stone
    /// pile is no place for a meal.
    pub fn add_filtered_stockpile(&mut self, a: Pos, b: Pos, accepts: StockFilter) {
        assert_eq!(a.z, b.z);
        self.stockpiles.push(Stockpile {
            rect: Rect {
                z: a.z,
                x0: a.x.min(b.x),
                y0: a.y.min(b.y),
                x1: a.x.max(b.x),
                y1: a.y.max(b.y),
            },
            accepts,
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

    /// Designate a temple where dwarves worship for solace.
    pub fn add_temple(&mut self, a: Pos, b: Pos) {
        assert_eq!(a.z, b.z);
        self.temples.push(Rect {
            z: a.z,
            x0: a.x.min(b.x),
            y0: a.y.min(b.y),
            x1: a.x.max(b.x),
            y1: a.y.max(b.y),
        });
    }

    pub fn temple_at(&self, p: Pos) -> bool {
        self.temples.iter().any(|t| t.contains(p))
    }

    /// Designate a hospital: the wounded rest here and mend far faster.
    pub fn add_hospital(&mut self, a: Pos, b: Pos) {
        assert_eq!(a.z, b.z);
        self.hospitals.push(Rect {
            z: a.z,
            x0: a.x.min(b.x),
            y0: a.y.min(b.y),
            x1: a.x.max(b.x),
            y1: a.y.max(b.y),
        });
    }

    pub fn hospital_at(&self, p: Pos) -> bool {
        self.hospitals.iter().any(|h| h.contains(p))
    }

    /// Designate a barracks: enlisted soldiers drill here between battles.
    pub fn add_barracks(&mut self, a: Pos, b: Pos) {
        assert_eq!(a.z, b.z);
        self.barracks.push(Rect {
            z: a.z,
            x0: a.x.min(b.x),
            y0: a.y.min(b.y),
            x1: a.x.max(b.x),
            y1: a.y.max(b.y),
        });
    }

    pub fn barracks_at(&self, p: Pos) -> bool {
        self.barracks.iter().any(|b| b.contains(p))
    }

    /// Designate a burrow: a safe room civilians flee to when the alarm sounds.
    pub fn add_burrow(&mut self, a: Pos, b: Pos) {
        assert_eq!(a.z, b.z);
        self.burrows.push(Rect {
            z: a.z,
            x0: a.x.min(b.x),
            y0: a.y.min(b.y),
            x1: a.x.max(b.x),
            y1: a.y.max(b.y),
        });
    }

    pub fn burrow_at(&self, p: Pos) -> bool {
        self.burrows.iter().any(|b| b.contains(p))
    }

    /// Designate a library: with one, the fort's scholars pen treatises.
    /// Set a room aside for sleeping. A bed standing in one becomes its
    /// owner's bedroom, and a dwarf with a room of their own is a contented
    /// dwarf — the cheapest happiness a fort can buy.
    pub fn add_bedroom(&mut self, a: Pos, b: Pos) {
        assert_eq!(a.z, b.z);
        self.bedrooms.push(Rect {
            z: a.z,
            x0: a.x.min(b.x),
            y0: a.y.min(b.y),
            x1: a.x.max(b.x),
            y1: a.y.max(b.y),
        });
    }

    pub fn bedroom_at(&self, p: Pos) -> bool {
        self.bedrooms.iter().any(|r| r.contains(p))
    }

    /// Set aside a hall for the fort to eat in. Dwarves carry their food here
    /// rather than eat standing in the larder — company at a meal is worth
    /// more to a dwarf than the meal is.
    pub fn add_dining_hall(&mut self, a: Pos, b: Pos) {
        assert_eq!(a.z, b.z);
        self.dining.push(Rect {
            z: a.z,
            x0: a.x.min(b.x),
            y0: a.y.min(b.y),
            x1: a.x.max(b.x),
            y1: a.y.max(b.y),
        });
    }

    pub fn dining_at(&self, p: Pos) -> bool {
        self.dining.iter().any(|r| r.contains(p))
    }

    pub fn add_library(&mut self, a: Pos, b: Pos) {
        assert_eq!(a.z, b.z);
        self.library.push(Rect {
            z: a.z,
            x0: a.x.min(b.x),
            y0: a.y.min(b.y),
            x1: a.x.max(b.x),
            y1: a.y.max(b.y),
        });
    }

    pub fn library_at(&self, p: Pos) -> bool {
        self.library.iter().any(|l| l.contains(p))
    }

    /// Sound or lift the civilian alarm. Returns the new state.
    pub fn toggle_alarm(&mut self) -> bool {
        self.alarm = !self.alarm;
        if self.alarm {
            self.log_event("The alarm sounds — civilians take to the burrows!".to_string());
        } else {
            self.log_event("The alarm is lifted; the fort returns to work.".to_string());
        }
        self.alarm
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

    /// Enlist or dismiss the fort dwarf at `p` as a soldier. Returns the
    /// new soldier state, or None if there's no citizen there.
    pub fn toggle_soldier(&mut self, p: Pos) -> Option<bool> {
        let i = self
            .dwarves
            .iter()
            .position(|d| d.alive && d.faction == Faction::Fort && d.pos == p)?;
        self.dwarves[i].soldier = !self.dwarves[i].soldier;
        let (name, now) = (self.dwarves[i].name.clone(), self.dwarves[i].soldier);
        if now {
            self.enlist_in_squad(i);
        } else {
            self.discharge_from_squads(i);
        }
        self.log_event(if now {
            format!("{name} takes up arms as a soldier.")
        } else {
            format!("{name} lays down their arms.")
        });
        Some(now)
    }

    /// Slot a new soldier into a squad — the first with room, or a fresh one.
    /// Dwarf Fortress caps a squad at ten, so an eleventh soldier musters a
    /// second squad rather than crowding the first.
    fn enlist_in_squad(&mut self, i: usize) {
        if self.squads.iter().any(|s| s.members.contains(&i)) {
            return;
        }
        if let Some(sq) = self.squads.iter_mut().find(|s| s.members.len() < SQUAD_MAX) {
            sq.members.push(i);
            return;
        }
        let n = self.squads.len() + 1;
        self.squads.push(Squad {
            name: format!("the {} Company", ordinal(n)),
            members: vec![i],
            order: SquadOrder::Defend,
            uniform: Uniform::Melee,
        });
    }

    /// Take a discharged (or dead) soldier off the rolls. An emptied squad is
    /// struck so it does not linger with no one in it.
    fn discharge_from_squads(&mut self, i: usize) {
        for sq in &mut self.squads {
            sq.members.retain(|&m| m != i);
        }
        self.squads.retain(|s| !s.members.is_empty());
    }

    /// Whether `dwarf` is an enlisted, living Fort soldier — the only kind that
    /// belongs to a squad.
    fn is_enlisted(&self, dwarf: usize) -> bool {
        self.dwarves
            .get(dwarf)
            .is_some_and(|d| d.alive && d.faction == Faction::Fort && d.soldier)
    }

    /// Move a soldier into `squad`, out of whatever squad they were in — the
    /// player editing the muster. Returns false if the dwarf is not an enlisted
    /// soldier, the target index is invalid, the squad is already full, or they
    /// are already in it. An emptied source squad is struck.
    pub fn assign_to_squad(&mut self, dwarf: usize, squad: usize) -> bool {
        if !self.is_enlisted(dwarf) || squad >= self.squads.len() {
            return false;
        }
        if self.squads[squad].members.contains(&dwarf)
            || self.squads[squad].members.len() >= SQUAD_MAX
        {
            return false;
        }
        for sq in &mut self.squads {
            sq.members.retain(|&m| m != dwarf);
        }
        self.squads[squad].members.push(dwarf);
        // The member is already in the target squad object, so pruning empty
        // source squads can't lose them (indices may shift, but membership
        // holds).
        self.squads.retain(|s| !s.members.is_empty());
        let (sname, dname) = (
            self.squads
                .iter()
                .find(|s| s.members.contains(&dwarf))
                .map(|s| s.name.clone())
                .unwrap_or_default(),
            self.dwarves[dwarf].name.clone(),
        );
        self.log_event(format!("{dname} joins {sname}."));
        true
    }

    /// Peel a soldier off into a brand-new squad of their own — how the player
    /// splits the muster into a melee line and a marksdwarf squad. Returns the
    /// new squad's index, or None if the dwarf is not an enlisted soldier or is
    /// already the sole member of their squad (nothing to split off).
    pub fn split_to_new_squad(&mut self, dwarf: usize) -> Option<usize> {
        if !self.is_enlisted(dwarf) {
            return None;
        }
        let cur = self.squad_of(dwarf)?;
        if self.squads[cur].members.len() <= 1 {
            return None; // already a squad of one
        }
        for sq in &mut self.squads {
            sq.members.retain(|&m| m != dwarf);
        }
        // Name it for the next free ordinal so two squads don't share a name.
        let n = (1..=self.squads.len() + 1)
            .find(|&k| {
                let name = format!("the {} Company", ordinal(k));
                !self.squads.iter().any(|s| s.name == name)
            })
            .unwrap_or(self.squads.len() + 1);
        self.squads.push(Squad {
            name: format!("the {} Company", ordinal(n)),
            members: vec![dwarf],
            order: SquadOrder::Defend,
            uniform: Uniform::Melee,
        });
        let (sname, dname) = (
            self.squads.last().unwrap().name.clone(),
            self.dwarves[dwarf].name.clone(),
        );
        self.log_event(format!("{dname} musters {sname}."));
        Some(self.squads.len() - 1)
    }

    /// Give a squad its standing order — the player's command.
    pub fn set_squad_order(&mut self, squad: usize, order: SquadOrder) {
        if let Some(sq) = self.squads.get_mut(squad) {
            sq.order = order;
            let (name, what) = (
                sq.name.clone(),
                match order {
                    SquadOrder::Defend => "will defend the fort".to_string(),
                    SquadOrder::Station(p) => format!("holds a post at ({}, {})", p.x, p.y),
                    SquadOrder::Patrol(a, b) => {
                        format!("patrols from ({}, {}) to ({}, {})", a.x, a.y, b.x, b.y)
                    }
                    SquadOrder::Train => "drills at the barracks".to_string(),
                },
            );
            self.log_event(format!("{name} {what}."));
        }
    }

    /// The standing order for the squad this soldier belongs to. A soldier in
    /// no squad (or a lone enlistee before mustering) simply defends.
    fn squad_order(&self, i: usize) -> SquadOrder {
        self.squads
            .iter()
            .find(|s| s.members.contains(&i))
            .map_or(SquadOrder::Defend, |s| s.order)
    }

    /// Which squad a dwarf belongs to, if any — for the UI to select a squad by
    /// clicking one of its soldiers.
    pub fn squad_of(&self, dwarf: usize) -> Option<usize> {
        self.squads.iter().position(|s| s.members.contains(&dwarf))
    }

    /// How the squad this soldier belongs to is armed. Squad-less soldiers
    /// (and the whole fort before the first muster) default to melee.
    fn squad_uniform(&self, i: usize) -> Uniform {
        self.squads
            .iter()
            .find(|s| s.members.contains(&i))
            .map_or(Uniform::Melee, |s| s.uniform)
    }

    /// Arm a squad as melee steel or crossbows — the player's choice. The
    /// armoury re-issues weapons by kind next time each soldier draws.
    pub fn set_squad_uniform(&mut self, squad: usize, uniform: Uniform) {
        if let Some(sq) = self.squads.get_mut(squad) {
            sq.uniform = uniform;
            let (name, what) = (
                sq.name.clone(),
                match uniform {
                    Uniform::Melee => "takes up sword and shield",
                    Uniform::Ranged => "takes up crossbows",
                },
            );
            self.log_event(format!("{name} {what}."));
        }
    }

    /// How many enlisted soldiers march under the crossbow — the fort's
    /// marksdwarves. Drives how many crossbows and bolts the armoury wants.
    fn marksdwarf_count(&self) -> usize {
        self.squads
            .iter()
            .filter(|s| s.uniform == Uniform::Ranged)
            .map(|s| {
                s.members
                    .iter()
                    .filter(|&&m| self.dwarves[m].alive && self.dwarves[m].soldier)
                    .count()
            })
            .sum()
    }

    pub fn soldier_count(&self) -> usize {
        self.dwarves
            .iter()
            .filter(|d| d.alive && d.faction == Faction::Fort && d.soldier)
            .count()
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
            war_marked: false,
            war: false,
            hp: kind.war_hp(),
            atk_cd: 0,
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
            // Dogs are companions and cats are pest control — neither is
            // livestock, and neither goes to the block. Butchering the cat that
            // guards your larder would be a fine way to lose a fort.
            .filter(|(_, a)| {
                a.alive
                    && !a.marked
                    && !matches!(a.kind, AnimalKind::Dog | AnimalKind::Cat)
            })
            .min_by_key(|(_, a)| a.pos.manhattan(p))
            .map(|(i, _)| i)?;
        self.animals[idx].marked = true;
        let kind = self.animals[idx].kind.name();
        self.log_event(format!("A {kind} is marked for slaughter."));
        Some(idx)
    }

    /// Mark the nearest trainable animal (an untrained adult dog) near `p` for
    /// war training. Returns its index if one was marked.
    pub fn mark_nearest_for_war(&mut self, p: Pos) -> Option<usize> {
        let idx = self
            .animals
            .iter()
            .enumerate()
            .filter(|(_, a)| {
                a.alive && a.kind.trainable() && a.is_adult() && !a.war && !a.war_marked
            })
            .min_by_key(|(_, a)| a.pos.manhattan(p))
            .map(|(i, _)| i)?;
        self.animals[idx].war_marked = true;
        self.log_event("A dog is marked for war training.".to_string());
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
        // Newest first: painting a pile over another is how a player corrects
        // a mis-painted one, so the last word must win. (Piles may overlap;
        // clipping them apart is a bigger change than this deserves.)
        self.stockpiles.iter().rposition(|s| s.contains(p))
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
        }
        // The booze comes in casks, as it does in Dwarf Fortress — a wagon
        // does not carry loose wine. Each holds one stack, and they are the
        // fort's only barrels until a carpenter works more, so drinking the
        // cellar dry is also how a fort frees the casks to brew again.
        let casks = 25usize.div_ceil(BATCH);
        for _ in 0..casks {
            supplies.push((ItemKind::Barrel, 0));
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
                            quality: 0,
                            made_at: 0,
            variant: 0,
                        });
                        placed += 1;
                    }
                }
            }
        }
        // Fill the casks: the embark's drink rides inside the barrels it came
        // in, so a fort never starts with wine on the ground.
        let casks: Vec<usize> = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, it)| it.kind == ItemKind::Barrel && it.active())
            .map(|(c, _)| c)
            .collect();
        let mut poured = 0usize;
        'pour: for c in casks {
            let at = self.items[c].pos;
            for _ in 0..BATCH {
                if poured >= 25 {
                    break 'pour;
                }
                self.items.push(Item {
                    kind: ItemKind::Drink,
                    stuff: 0,
                    name: None,
                    pos: at,
                    state: ItemState::Inside { container: c },
                    reserved_by: None,
                    consumed: false,
                    quality: 0,
                    made_at: 0,
            variant: 0,
                });
                poured += 1;
            }
        }
    }

    /// Set a brace of dogs down near the wagon — pets at first, but any can be
    /// war-trained into a fortress guardian. Kept separate from
    /// `add_embark_supplies` so headless tests keep a stable RNG stream.
    pub fn add_starting_dogs(&mut self) {
        let cx = self.map.width as i32 / 2;
        let cy = self.map.height as i32 / 2;
        for pair in 0..2 {
            let ox = cx + 3 + pair;
            if let Some(z) = self.map.walk_surface_z(ox.max(0) as usize, (cy + 4).max(0) as usize) {
                self.add_animal(AnimalKind::Dog, Pos::new(ox, cy + 4, z as i32), true);
            }
        }
    }

    /// Set a cat down near the wagon. It is good for nothing except killing
    /// the vermin that eat the fort's food, which is reason enough — a fort
    /// that embarked without one and settled evil ground would watch demon rats
    /// carry off its larder with no answer. Kept out of `add_embark_supplies`
    /// so headless tests keep a stable RNG stream.
    pub fn add_starting_cat(&mut self) {
        let cx = self.map.width as i32 / 2;
        let cy = self.map.height as i32 / 2;
        let ox = cx - 5;
        if let Some(z) = self.map.walk_surface_z(ox.max(0) as usize, (cy + 4).max(0) as usize) {
            self.add_animal(AnimalKind::Cat, Pos::new(ox, cy + 4, z as i32), true);
        }
    }

    /// Is there a tree standing on this tile?
    pub fn tree_at(&self, p: Pos) -> bool {
        self.trees.contains_key(&p)
    }

    /// The wood species of the tree on this tile, if any.
    pub fn tree_species(&self, p: Pos) -> Option<u16> {
        self.trees.get(&p).copied()
    }

    /// May a tree stand on this tile? Open walkable surface, clear of another
    /// tree, a shrub, a building, a farm, water, and any planned wall (a tree
    /// grown onto a construction tile would be sealed inside the raised wall).
    fn can_plant_tree(&self, p: Pos) -> bool {
        !self.trees.contains_key(&p)
            && !self.shrubs.contains(&p)
            && self.building_at(p).is_none()
            && !self.farms.contains_key(&p)
            && !self.constructions.contains_key(&p)
            && self.map.water_at(p) == 0
    }

    /// Scatter `count` trees across walkable surface tiles, each a random wood
    /// species, and set the regrowth ceiling to half again their number.
    /// Deterministic given the sim's rng; called at embark (like the starting
    /// dogs) so headless tests that don't ask for a forest stay byte-identical.
    pub fn plant_trees(&mut self, count: usize, raws: &Raws) {
        let woods = raws.materials.indices_in_category(MaterialCategory::Wood);
        let (w, h) = (self.map.width as i32, self.map.height as i32);
        let mut placed = 0;
        // Bounded attempts so a map with little open ground can't spin forever.
        for _ in 0..(count * 20) {
            if placed >= count {
                break;
            }
            let x = self.rng.gen_range(0..w);
            let y = self.rng.gen_range(0..h);
            let Some(z) = self.map.walk_surface_z(x as usize, y as usize) else { continue };
            let p = Pos::new(x, y, z as i32);
            if !self.can_plant_tree(p) {
                continue;
            }
            let species = if woods.is_empty() {
                0
            } else {
                woods[self.rng.gen_range(0..woods.len())]
            };
            self.trees.insert(p, species);
            placed += 1;
        }
        // A felled woodland regrows toward half again its planted size.
        self.tree_cap = self.trees.len() + self.trees.len() / 2;
    }

    /// Is there a berry shrub standing on this tile?
    pub fn shrub_at(&self, p: Pos) -> bool {
        self.shrubs.contains(&p)
    }

    /// May a wild shrub stand on this tile? Open, walkable surface, clear of
    /// trees, buildings, farms, water, another shrub, and any planned wall (a
    /// shrub seeded onto a construction tile would be sealed inside the raised
    /// wall and could never be foraged).
    fn can_plant_shrub(&self, p: Pos) -> bool {
        !self.shrubs.contains(&p)
            && !self.trees.contains_key(&p)
            && self.building_at(p).is_none()
            && !self.farms.contains_key(&p)
            && !self.constructions.contains_key(&p)
            && self.map.water_at(p) == 0
    }

    /// Scatter `count` wild berry shrubs across walkable surface tiles, and set
    /// the regrowth ceiling to half again their number. Deterministic given the
    /// sim's rng; called at embark (like the trees) so headless forts that don't
    /// ask for foraging stay byte-identical.
    pub fn plant_shrubs(&mut self, count: usize) {
        let (w, h) = (self.map.width as i32, self.map.height as i32);
        let mut placed = 0;
        for _ in 0..(count * 20) {
            if placed >= count {
                break;
            }
            let x = self.rng.gen_range(0..w);
            let y = self.rng.gen_range(0..h);
            let Some(z) = self.map.walk_surface_z(x as usize, y as usize) else { continue };
            let p = Pos::new(x, y, z as i32);
            if !self.can_plant_shrub(p) {
                continue;
            }
            self.shrubs.insert(p);
            placed += 1;
        }
        // A patch may spread to half again its planted size, then holds.
        self.shrub_cap = self.shrubs.len() + self.shrubs.len() / 2;
    }

    /// Sow wild cave mushrooms across the dry cavern floor — the fort's food in
    /// the deep, gathered like the surface berry shrubs (the very same system),
    /// but only reachable once a fort digs into the cavern. App-embark-only, so
    /// a fort with no cavern is unchanged.
    pub fn plant_cave_mushrooms(&mut self, count: usize) {
        let floors: Vec<Pos> = self
            .cavern_floors
            .iter()
            .copied()
            .filter(|&p| self.map.water_at(p) == 0 && self.map.walkable(p))
            .collect();
        if floors.is_empty() {
            return;
        }
        let mut placed = 0;
        for _ in 0..(count * 20) {
            if placed >= count {
                break;
            }
            let p = floors[self.rng.gen_range(0..floors.len())];
            if self.shrubs.insert(p) {
                placed += 1;
            }
        }
        // Cave mushrooms spread through the dark as surface patches do.
        self.shrub_cap = self.shrubs.len() + self.shrubs.len() / 2;
    }

    /// A random in-bounds walkable-surface cardinal neighbour of `parent`, or
    /// `None` if the step runs off the map. DRAWS RNG (the direction) — only
    /// call it past a feature's regrowth gate, never on a bare fort.
    fn random_surface_neighbour(&mut self, parent: Pos) -> Option<Pos> {
        const DIRS: [(i32, i32); 4] = [(-1, 0), (1, 0), (0, -1), (0, 1)];
        let (dx, dy) = DIRS[self.rng.gen_range(0..4)];
        let (nx, ny) = (parent.x + dx, parent.y + dy);
        if nx < 0 || ny < 0 || nx >= self.map.width as i32 || ny >= self.map.height as i32 {
            return None;
        }
        let z = self.map.walk_surface_z(nx as usize, ny as usize)?;
        Some(Pos::new(nx, ny, z as i32))
    }

    /// The living surface reseeds itself: a tended berry patch spreads once a
    /// day, and a felled woodland grows a sapling every few days — each up to
    /// its planted ceiling. Both branches are WHOLLY GATED on the plant already
    /// existing AND under its cap (set only by plant_shrubs/plant_trees at
    /// embark), so a bare or hand-built fort draws no rng and is untouched —
    /// the headless suite stays byte-identical.
    fn tick_regrowth(&mut self) {
        // Berry shrubs reseed once a day.
        if self.clock.tick % SHRUB_REGROW_INTERVAL == 0
            && !self.shrubs.is_empty()
            && self.shrubs.len() < self.shrub_cap
        {
            let n = self.shrubs.len();
            let parent = *self.shrubs.iter().nth(self.rng.gen_range(0..n)).unwrap();
            if let Some(np) = self.random_surface_neighbour(parent) {
                if self.can_plant_shrub(np) && !self.designations.contains_key(&np) {
                    self.shrubs.insert(np);
                }
            }
        }
        // Forests regrow more slowly: a sapling of the same wood takes root
        // near a standing tree.
        if self.clock.tick % TREE_REGROW_INTERVAL == 0
            && !self.trees.is_empty()
            && self.trees.len() < self.tree_cap
        {
            let n = self.trees.len();
            let (&parent, &species) = self.trees.iter().nth(self.rng.gen_range(0..n)).unwrap();
            if let Some(np) = self.random_surface_neighbour(parent) {
                if self.can_plant_tree(np) && !self.designations.contains_key(&np) {
                    self.trees.insert(np, species);
                }
            }
        }
    }

    /// App/embark only: curse one of the founding seven as a secret vampire.
    /// Deliberately kept out of `add_embark_supplies` — the many headless tests
    /// that build a fort by hand must stay byte-identical, so a fort only
    /// harbours a vampire when the app chooses to plant one. No log line: the
    /// whole point is that not even the player knows who it is.
    pub fn curse_a_vampire(&mut self) {
        let candidates: Vec<usize> = self
            .dwarves
            .iter()
            .enumerate()
            .filter(|(_, d)| d.alive && d.faction == Faction::Fort)
            .map(|(i, _)| i)
            .collect();
        if candidates.is_empty() {
            return;
        }
        let pick = candidates[self.rng.gen_range(0..candidates.len())];
        self.dwarves[pick].vampire = true;
        self.dwarves[pick].last_fed = self.clock.tick;
    }

    /// App/embark only: a vampire is a RARE curse. Roughly one fort in ten is
    /// founded with one hidden among the seven; most forts never see one, so
    /// finding drained corpses is a genuine (and unwelcome) surprise.
    pub fn maybe_curse_a_vampire(&mut self) {
        if self.rng.gen_ratio(1, 10) {
            self.curse_a_vampire();
        }
    }

    /// App/embark only: afflict one of the founders with the werebeast curse.
    pub fn curse_a_werebeast(&mut self) {
        let candidates: Vec<usize> = self
            .dwarves
            .iter()
            .enumerate()
            .filter(|(_, d)| d.alive && d.faction == Faction::Fort)
            .map(|(i, _)| i)
            .collect();
        if candidates.is_empty() {
            return;
        }
        let pick = candidates[self.rng.gen_range(0..candidates.len())];
        self.dwarves[pick].werebeast = true;
    }

    /// App/embark only: rarely (about one fort in twelve) a founder carries the
    /// werebeast curse, unknown until the first full moon.
    pub fn maybe_curse_a_werebeast(&mut self) {
        if self.rng.gen_ratio(1, 12) {
            self.curse_a_werebeast();
        }
    }

    /// Under the full moon, cursed dwarves twist into beasts and turn on the
    /// fort; at dawn after, the survivors revert. Draws no rng and changes
    /// nothing unless a cursed dwarf exists, so a fort without one is identical.
    fn tick_werebeasts(&mut self) {
        if !self.dwarves.iter().any(|d| d.alive && d.werebeast) {
            return;
        }
        let day = self.clock.tick / TICKS_PER_DAY;
        let full_moon = day % WERE_MOON_CYCLE < WERE_MOON_NIGHTS;
        for i in 0..self.dwarves.len() {
            if !self.dwarves[i].alive || !self.dwarves[i].werebeast {
                continue;
            }
            let were_form = self.dwarves[i].were_form;
            let is_fort = self.dwarves[i].faction == Faction::Fort;
            if full_moon && !were_form && is_fort {
                // The beast wakes: it turns hostile and falls upon the fort.
                let name = self.dwarves[i].name.clone();
                self.abandon_task(i);
                self.dwarves[i].were_form = true;
                self.dwarves[i].faction = Faction::Hostile;
                self.dwarves[i].beast = true;
                self.log_event(format!(
                    "Under the full moon, {name} twists into a snarling beast!"
                ));
            } else if !full_moon && were_form {
                // Dawn: the beast passes, leaving a shaken dwarf.
                let name = self.dwarves[i].name.clone();
                self.dwarves[i].were_form = false;
                self.dwarves[i].beast = false;
                self.dwarves[i].faction = Faction::Fort;
                self.dwarves[i].task = Task::Idle { wander_cd: 0 };
                self.log_event(format!("{name} returns to their senses, the curse sated for now."));
            }
        }
    }

    /// A necromancer raises the fort's fallen: a nearby corpse claws its way up
    /// as a hostile undead. Draws no rng and changes nothing unless a living
    /// necromancer walks the map, so a fort without one (every headless test —
    /// necromancers arrive only with invasion sieges) steps byte-identically.
    fn tick_necromancers(&mut self, raws: &Raws) {
        if !self.dwarves.iter().any(|d| d.alive && d.necromancer) {
            return;
        }
        if self.clock.tick % NECRO_INTERVAL != 0 {
            return;
        }
        for ni in 0..self.dwarves.len() {
            if !(self.dwarves[ni].alive && self.dwarves[ni].necromancer) {
                continue;
            }
            let npos = self.dwarves[ni].pos;
            // The first raisable corpse in range (by index, deterministic). Only
            // corpses settled on the ground count — never one a hauler has
            // claimed and is carrying to a tomb (else it could be raised out of
            // their arms and still get "buried").
            let corpse = self.items.iter().position(|it| {
                it.active()
                    && it.kind == ItemKind::Corpse
                    && it.reserved_by.is_none()
                    && matches!(it.state, ItemState::OnGround)
                    && it.pos.manhattan(npos) <= NECRO_RANGE
            });
            let Some(ci) = corpse else { continue };
            let cpos = self.items[ci].pos;
            self.items[ci].consumed = true;
            // Raising the body lays its unquiet spirit to rest — otherwise a
            // ghost whose corpse is consumed could never again be buried.
            let buried_dwarf = self.items[ci].stuff as usize;
            if buried_dwarf < self.dwarves.len()
                && !self.dwarves[buried_dwarf].alive
                && self.dwarves[buried_dwarf].ghost
            {
                self.dwarves[buried_dwarf].ghost = false;
            }
            let mut undead = new_dwarf(&mut self.rng, cpos, Faction::Hostile, raws);
            undead.name = "a shambling corpse".to_string();
            self.dwarves.push(undead);
            self.stats.raised += 1;
            self.log_event("The dead claw their way up to serve the necromancer!".to_string());
        }
    }

    /// A vampire feeds on the blood of an adjacent sleeping fort-mate. This
    /// draws no rng and changes nothing unless a *living fort vampire* exists,
    /// so a fort without one steps byte-for-byte identically — the whole
    /// mechanic is gated behind the secret it keeps.
    fn tick_vampires(&mut self) {
        if !self
            .dwarves
            .iter()
            .any(|d| d.alive && d.vampire && d.faction == Faction::Fort)
        {
            return;
        }
        let tick = self.clock.tick;
        for vi in 0..self.dwarves.len() {
            let v = &self.dwarves[vi];
            if !(v.alive && v.vampire && v.faction == Faction::Fort) {
                continue;
            }
            if tick.saturating_sub(v.last_fed) < VAMPIRE_FEED_INTERVAL {
                continue;
            }
            let vpos = v.pos;
            // The first adjacent sleeping fort-mate (never another vampire),
            // chosen by index so the hunt is fully deterministic.
            let victim = (0..self.dwarves.len()).find(|&j| {
                let d = &self.dwarves[j];
                j != vi
                    && d.alive
                    && d.faction == Faction::Fort
                    && !d.vampire
                    && matches!(d.task, Task::Sleep { .. })
                    && d.pos.z == vpos.z
                    && (d.pos.x - vpos.x).abs() <= 1
                    && (d.pos.y - vpos.y).abs() <= 1
            });
            let Some(vic) = victim else { continue };
            self.dwarves[vi].last_fed = tick;
            let victim_name = self.dwarves[vic].name.clone();
            self.dwarves[vic].blood = (self.dwarves[vic].blood - VAMPIRE_DRAIN).max(0.0);
            if self.dwarves[vic].blood <= 0.0 {
                // Drained white — the fort wakes to a bloodless corpse and the
                // dark knowledge that one of their own is not what it seems.
                self.stats.drained += 1;
                self.log_event(format!(
                    "{victim_name} was found in the morning pale and bloodless -- a vampire walks among us!"
                ));
                self.kill_dwarf(vic);
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
                .any(|s| x <= s.rect.x1 && s.rect.x0 <= x1 && y <= s.rect.y1 && s.rect.y0 <= y1);
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

    /// The nearest free cell of a pile that will TAKE this kind. A fort that
    /// has told its piles what they are for does not want boulders in the
    /// larder — and a good no pile accepts simply stays where it lies, which
    /// is how a player says "leave that there".
    fn find_free_cell(&self, kind: ItemKind, near: Pos, from_region: u32) -> Option<Pos> {
        self.stockpiles
            .iter()
            .filter(|s| s.takes(kind))
            .flat_map(|s| s.cells())
            .filter(|&c| self.regions.id(c) == from_region && self.cell_free(c))
            .min_by_key(|&c| c.manhattan(near))
    }

    // ---------------------------------------------------------- containers

    /// Every item a container holds. Derived by scanning, never stored — the
    /// contained item's own state is the single truth about where it lives.
    pub fn contents_of(&self, container: usize) -> Vec<usize> {
        self.items
            .iter()
            .enumerate()
            .filter(|(_, it)| it.active() && it.state == ItemState::Inside { container })
            .map(|(i, _)| i)
            .collect()
    }

    /// How full a container is, and of what.
    fn container_load(&self, container: usize) -> (usize, Option<ItemKind>) {
        let held = self.contents_of(container);
        let kind = held.first().map(|&i| self.items[i].kind);
        (held.len(), kind)
    }

    /// Can this container take one more of `kind`?
    ///
    /// A container holds ONE kind at a time — a barrel of wine is a barrel of
    /// wine, not a barrel of wine and fish. (Dwarf Fortress is explicit that a
    /// drink barrel holds a single stack; whether its food barrels mix types is
    /// undocumented, so we take the readable rule its naming implies.)
    fn container_accepts(&self, container: usize, kind: ItemKind) -> bool {
        let c = &self.items[container];
        if !c.active() || c.reserved_by.is_some() {
            return false;
        }
        // A container must be resting somewhere to be filled, and containers
        // never nest — no bin inside a barrel.
        if !matches!(c.state, ItemState::OnGround | ItemState::Stored { .. }) {
            return false;
        }
        let cap = container_capacity(c.kind, kind);
        if cap == 0 {
            return false;
        }
        let (load, held) = self.container_load(container);
        held.is_none_or(|h| h == kind) && load < cap
    }

    /// The nearest container in a stockpile that will take `kind`. Preferred
    /// over an empty cell, so the fort packs its goods away instead of
    /// carpeting the floor with them.
    /// An empty barrel standing somewhere a brewer could use it.
    ///
    /// Dwarf Fortress: "Brewers need a still, a brewable plant, and one empty
    /// barrel or water-tight pot per job in order to brew drinks." The barrel
    /// is what the drink goes home in — no empty barrel, no brewing, and a
    /// fort that never works its wood eventually drinks its cellar dry. It is
    /// the fort's most famous supply chain, and it starts at a tree.
    pub fn empty_barrel(&self, near: Pos, region: u32) -> Option<usize> {
        self.items
            .iter()
            .enumerate()
            .filter(|(c, it)| {
                it.kind == ItemKind::Barrel
                    && it.active()
                    && it.reserved_by.is_none()
                    && matches!(it.state, ItemState::OnGround | ItemState::Stored { .. })
                    && self.regions.id(it.pos) == region
                    && self.contents_of(*c).is_empty()
            })
            .min_by_key(|(_, it)| it.pos.manhattan(near))
            .map(|(c, _)| c)
    }

    /// Is this food out where a rat can get at it?
    ///
    /// Dwarf Fortress's rule is one word: vermin "attempt to eat EXPOSED
    /// food". A container is the counterplay, and we model it as absolute —
    /// a closed cask is rat-proof.
    ///
    /// That is a simplification, and here is its size. DF rolls the vermin's
    /// `PENETRATEPOWER` — 1, 2 or 3 — against 0-100 (0-95 for wood, 0-90 for
    /// cloth), so a cask turns away about 97 attempts in 100 whatever it is
    /// made of. Modelling the roll would buy a 3% leak and a per-material
    /// spread of a third of a percentage point: "metal barrels resist vermin"
    /// is true and practically meaningless. Not worth the die.
    ///
    /// So: a cask is not a pantry — it will not stop food ROTTING, that takes
    /// a stockpile — but it is a rat-proof box, which is the other half of why
    /// a fort wants one.
    pub fn food_exposed(&self, i: usize) -> bool {
        match self.items[i].state {
            ItemState::Inside { container } => {
                // In a container, and the container still exists: the rat has
                // to get through it first.
                !self
                    .items
                    .get(container)
                    .is_some_and(|c| c.active() && is_container(c.kind))
            }
            ItemState::Carried { .. } => false, // in someone's hands
            _ => true,
        }
    }

    /// A cat works the larder.
    ///
    /// Dwarf Fortress: a vermin hunter goes "randomly walking between places
    /// with food laying on the ground or in stockpiles, to check for possible
    /// VERMIN_EATER vermin". So a cat walks toward the food, which is where the
    /// rats will be — it does not graze, and it does not need to be told.
    ///
    /// Gated on there being a cat at all, so a catless fort draws no RNG here.
    fn tick_cats(&mut self) {
        if !self.animals.iter().any(|a| a.alive && a.kind == AnimalKind::Cat) {
            return;
        }
        // The larder: wherever exposed food is lying. A cat with nothing to
        // guard just sits.
        let larder: Vec<Pos> = self
            .items
            .iter()
            .enumerate()
            .filter(|(i, it)| {
                it.active()
                    && matches!(it.kind, ItemKind::Meal | ItemKind::Crop | ItemKind::Berry)
                    && self.food_exposed(*i)
            })
            .map(|(_, it)| it.pos)
            .collect();
        for idx in 0..self.animals.len() {
            if !self.animals[idx].alive || self.animals[idx].kind != AnimalKind::Cat {
                continue;
            }
            if self.animals[idx].move_cd > 0 {
                self.animals[idx].move_cd -= 1;
                continue;
            }
            let cp = self.animals[idx].pos;
            // A rat in sight comes first; otherwise walk the larder.
            let target = self
                .vermin
                .iter()
                .filter(|v| v.alive)
                .min_by_key(|v| v.pos.manhattan(cp))
                .map(|v| v.pos)
                .or_else(|| larder.iter().min_by_key(|p| p.manhattan(cp)).copied());
            let Some(tp) = target else { continue };
            if tp == cp {
                continue;
            }
            self.animals[idx].move_cd = WALK_COOLDOWN;
            if tp.z == cp.z {
                // Same floor: a cheap step is enough.
                let (sx, sy) = ((tp.x - cp.x).signum(), (tp.y - cp.y).signum());
                for step in [
                    Pos::new(cp.x + sx, cp.y + sy, cp.z),
                    Pos::new(cp.x + sx, cp.y, cp.z),
                    Pos::new(cp.x, cp.y + sy, cp.z),
                ] {
                    if step != cp && self.map.walkable(step) {
                        self.animals[idx].pos = step;
                        break;
                    }
                }
            } else if let Some(p) = path::astar(&self.map, cp, tp, MAX_ASTAR_NODES / 4) {
                // Another floor: walk the fort's own stairs down to it. A cat
                // that snapped to `walk_surface_z` instead climbed out through
                // the rock onto the mountaintop and could never come back —
                // which made it useless in every fort that keeps its larder
                // underground, i.e. every fort.
                if let Some(&next) = p.get(1) {
                    self.animals[idx].pos = next;
                }
            }
        }
    }

    /// Vermin eat the fort's food, and cats eat the vermin.
    ///
    /// The country decides which vermin you get — evil ground breeds demon
    /// rats, savage ground rhino lizards, good ground the fluffy wambler, which
    /// is exactly as harmless as it sounds and still eats your stores. They
    /// spawn from the land itself ("do not breed, but 'spawn', spontaneously
    /// appearing in their natural environment"), not from refuse.
    ///
    /// Gated on `vermin_kind`, which only the app sets at embark — so a
    /// headless fort has no vermin, draws no RNG here, and is byte-identical.
    ///
    /// The rate and the amount are OURS. The wiki documents neither anywhere,
    /// and I would rather pick a number and say so than dress a guess up as a
    /// fact.
    fn tick_vermin(&mut self) {
        let Some(kind) = self.vermin_kind else { return };
        // Cats hunt: "randomly walking between places with food laying on the
        // ground or in stockpiles, to check for possible VERMIN_EATER vermin".
        // A cat near a vermin kills it and leaves the remains.
        for v in 0..self.vermin.len() {
            if !self.vermin[v].alive {
                continue;
            }
            let vp = self.vermin[v].pos;
            let cat = self.animals.iter().any(|a| {
                a.alive
                    && a.kind == AnimalKind::Cat
                    && a.pos.z == vp.z
                    && a.pos.manhattan(vp) <= CAT_REACH
            });
            if cat {
                self.vermin[v].alive = false;
                self.stats.vermin_slain += 1;
            }
        }
        self.vermin.retain(|v| v.alive);

        // A new one creeps in now and then, up to what the country supports —
        // and it creeps in near the food, because that is what draws it. (A rat
        // spawned at random across the map would as often as not appear on
        // another z-level and never find the larder at all.)
        if self.vermin.len() < VERMIN_CAP && self.clock.tick % VERMIN_SPAWN_INTERVAL == 0 {
            let larder = self
                .items
                .iter()
                .enumerate()
                .find(|(i, it)| {
                    it.active()
                        && matches!(it.kind, ItemKind::Meal | ItemKind::Crop | ItemKind::Berry)
                        && self.food_exposed(*i)
                })
                .map(|(_, it)| it.pos);
            let spot = match larder {
                Some(p) => self.walkable_near(p, 6),
                None => self.random_surface_spot(),
            };
            if let Some(pos) = spot {
                self.vermin.push(Vermin {
                    kind,
                    pos,
                    alive: true,
                    move_cd: 0,
                    eat_cd: VERMIN_EAT_INTERVAL,
                });
            }
        }

        // They go for the food, and eat what is not put away properly.
        for v in 0..self.vermin.len() {
            // Hunger counts down on every tick, not only the ones it acts on.
            self.vermin[v].eat_cd = self.vermin[v].eat_cd.saturating_sub(1);
            if self.vermin[v].move_cd > 0 {
                self.vermin[v].move_cd -= 1;
                continue;
            }
            self.vermin[v].move_cd = VERMIN_WALK_COOLDOWN;
            let vp = self.vermin[v].pos;
            // The nearest exposed food, which is the only kind they can eat.
            // Any exposed food anywhere, nearest first. Not just this level:
            // a rat that could only smell its own z-level would sit one floor
            // above the larder forever.
            let prey = self
                .items
                .iter()
                .enumerate()
                .filter(|(i, it)| {
                    it.active()
                        && matches!(it.kind, ItemKind::Meal | ItemKind::Crop | ItemKind::Berry)
                        && self.food_exposed(*i)
                })
                .min_by_key(|(_, it)| {
                    it.pos.manhattan(vp) + (it.pos.z - vp.z).unsigned_abs() * 4
                })
                .map(|(i, it)| (i, it.pos));
            let Some((food, fp)) = prey else { continue };
            if fp == vp {
                // Dinner — but only now and then. A rat sitting on the larder
                // nibbles; it does not inhale it.
                if self.vermin[v].eat_cd > 0 {
                    continue;
                }
                self.vermin[v].eat_cd = VERMIN_EAT_INTERVAL;
                self.items[food].consumed = true;
                self.stats.food_gnawed += 1;
                if self.stats.food_gnawed % 5 == 1 {
                    self.log_event(format!(
                        "Vermin are at the food — a {} is eating what has not been packed away.",
                        self.vermin[v].kind.name()
                    ));
                }
                continue;
            }
            // Standing over it but a level off: squeeze through. A floor is
            // no obstacle to a rat — but rock is. One level at a time, and
            // only onto somewhere it could actually stand, or a sealed larder
            // under bedrock would be no safer than an open table.
            if fp.x == vp.x && fp.y == vp.y && fp.z != vp.z {
                let step = Pos::new(vp.x, vp.y, vp.z + (fp.z - vp.z).signum());
                if self.map.walkable(step) {
                    self.vermin[v].pos = step;
                }
                continue;
            }
            // Shuffle toward it. No pathfinding — they are vermin, they get
            // where they are going eventually. Try the diagonal, then either
            // axis alone, so a wall does not pin them.
            //
            // Each column is tried at the rat's own level, one level toward
            // the food, and at whatever the ground there is: a rat that could
            // only walk its own z would sit at the foot of a hill forever
            // watching the larder on top of it.
            let (sx, sy) = ((fp.x - vp.x).signum(), (fp.y - vp.y).signum());
            let dz = (fp.z - vp.z).signum();
            'step: for (nx, ny) in [
                (vp.x + sx, vp.y + sy),
                (vp.x + sx, vp.y),
                (vp.x, vp.y + sy),
            ] {
                if nx < 0 || ny < 0 || nx >= self.map.width as i32 || ny >= self.map.height as i32 {
                    continue;
                }
                let ground = self.map.walk_surface_z(nx as usize, ny as usize);
                let levels = [
                    Some(vp.z),
                    (dz != 0).then_some(vp.z + dz),
                    ground.map(|z| z as i32),
                ];
                for nz in levels.into_iter().flatten() {
                    let step = Pos::new(nx, ny, nz);
                    if step != vp && self.map.walkable(step) {
                        self.vermin[v].pos = step;
                        break 'step;
                    }
                }
            }
        }
    }

    /// A walkable tile within `radius` of `p` — where a vermin slips in from.
    ///
    /// Snaps to each column's own walkable surface rather than insisting on
    /// `p`'s exact level: on rough ground almost nothing at one fixed z is
    /// walkable, and a rat that cannot find a way in is a mechanic that never
    /// fires.
    fn walkable_near(&mut self, p: Pos, radius: i32) -> Option<Pos> {
        for _ in 0..12 {
            let dx = self.rng.gen_range(-radius..=radius);
            let dy = self.rng.gen_range(-radius..=radius);
            let (x, y) = (p.x + dx, p.y + dy);
            if x < 1 || y < 1 || x >= self.map.width as i32 - 1 || y >= self.map.height as i32 - 1 {
                continue;
            }
            let q = Pos::new(x, y, p.z);
            if self.map.walkable(q) {
                return Some(q);
            }
            if let Some(z) = self.map.walk_surface_z(x as usize, y as usize) {
                return Some(Pos::new(x, y, z as i32));
            }
        }
        self.map.walkable(p).then_some(p)
    }

    /// A walkable surface tile somewhere on the map, for vermin to creep in at.
    fn random_surface_spot(&mut self) -> Option<Pos> {
        for _ in 0..8 {
            let x = self.rng.gen_range(1..self.map.width - 1);
            let y = self.rng.gen_range(1..self.map.height - 1);
            if let Some(z) = self.map.walk_surface_z(x, y) {
                return Some(Pos::new(x as i32, y as i32, z as i32));
            }
        }
        None
    }

    /// Is this food somewhere it will keep?
    ///
    /// In Dwarf Fortress this is a question about WHERE, not about what the
    /// food is packed in: "Food will never spoil while in a stockpile", and
    /// "it does not matter if the food is in a container; a barrel full of
    /// meat left in a corridor will rot". A barrel is a hauling convenience,
    /// not a pantry — the widely-repeated belief that casks preserve food was
    /// true two versions ago and has been a myth ever since.
    ///
    /// So: a pile keeps food. A cask keeps food only because the cask stands
    /// in a pile. And food in a dwarf's hands is on its way somewhere.
    pub fn food_keeps(&self, i: usize) -> bool {
        match self.items[i].state {
            ItemState::Stored { .. } | ItemState::Carried { .. } => true,
            ItemState::Inside { container } => self
                .items
                .get(container)
                .is_some_and(|c| c.active() && matches!(c.state, ItemState::Stored { .. })),
            ItemState::OnGround => false,
        }
    }

    /// Food left out of the fort's stores goes bad.
    ///
    /// Two fates, as in DF. Meals ROT — they stink, and a dwarf who passes the
    /// heap is the worse for it. Crops and berries merely WITHER: useless, but
    /// nobody's day is ruined by a shrivelled plant. Drink and seeds keep
    /// forever, which is the whole reason a fort brews its harvest instead of
    /// eating it.
    ///
    /// Deterministic: a fixed scan, no RNG. Daily, and only over food.
    fn tick_spoilage(&mut self) {
        let now = self.clock.tick;
        let shelf = SHELF_LIFE_DAYS * TICKS_PER_DAY;
        let mut rotted: Vec<Pos> = Vec::new();
        let mut withered = 0usize;
        for i in 0..self.items.len() {
            let it = &self.items[i];
            if !it.active() || !matches!(it.kind, ItemKind::Meal | ItemKind::Crop | ItemKind::Berry)
            {
                continue;
            }
            if self.food_keeps(i) || now.saturating_sub(it.made_at) < shelf {
                continue;
            }
            let (kind, pos) = (it.kind, it.pos);
            self.items[i].consumed = true;
            self.stats.food_spoiled += 1;
            if kind == ItemKind::Meal {
                rotted.push(pos);
            } else {
                withered += 1;
            }
        }
        if withered > 0 {
            self.log_event(format!(
                "{withered} harvest(s) left out of the stores have withered away."
            ));
        }
        // A rotting meal stinks. Anyone near it is the worse for having smelled
        // it — our small answer to DF's miasma, without the cloud.
        for pos in &rotted {
            self.log_event("Food left to rot fouls the air.".to_string());
            for j in 0..self.dwarves.len() {
                let d = &self.dwarves[j];
                if d.alive
                    && d.faction == Faction::Fort
                    && d.pos.z == pos.z
                    && d.pos.manhattan(*pos) <= MIASMA_RANGE
                {
                    self.push_thought(j, ThoughtKind::SmelledRot);
                }
            }
        }
    }

    /// Would this container take one more of `kind`? Test/UI window onto the
    /// containment rules.
    pub fn container_accepts_kind(&self, container: usize, kind: ItemKind) -> bool {
        self.container_accepts(container, kind)
    }

    /// Send a container and its contents out of the fort, as a trade does.
    /// Exposed so tests can exercise the rule without staging a caravan.
    pub fn debug_consume_with_contents(&mut self, container: usize) {
        self.consume_with_contents(container);
    }

    /// Should the carpenter work another barrel (or bin)?
    ///
    /// The question is not "how many do we own" — a fort can own nine casks
    /// and still have nowhere to put a meal, because every one of them is full
    /// of wine and a container holds one kind. It is "is there something we
    /// cannot put away": a good lying loose that no standing container will
    /// take. One more container is wanted, and once it fills, the question
    /// gets asked again. That converges without ever counting anything twice.
    ///
    /// A still needs an EMPTY barrel to brew into, so a fort that owns a still
    /// and no empty cask wants one even with its larder tidy — otherwise the
    /// brewery stops the moment the last cask fills.
    ///
    /// Bounded by the stockpile floor, which is Dwarf Fortress's own rule that
    /// a pile takes as many containers as it has tiles.
    fn wants_another_container(&self, container: ItemKind) -> bool {
        let cells: usize = self.stockpiles.iter().map(|s| s.cells().count()).sum();
        if self.count_kind(container) >= cells.max(1) {
            return false;
        }
        let homeless = self.items.iter().enumerate().any(|(_, it)| {
            it.active()
                && matches!(it.state, ItemState::OnGround | ItemState::Stored { .. })
                && container_capacity(container, it.kind) > 0
                && !self
                    .items
                    .iter()
                    .enumerate()
                    .any(|(c, o)| o.kind == container && self.container_accepts(c, it.kind))
        });
        if homeless {
            return true;
        }
        container == ItemKind::Barrel
            && self.buildings.iter().any(|b| b.kind == BuildingKind::Still)
            && !self.items.iter().enumerate().any(|(c, it)| {
                it.kind == ItemKind::Barrel
                    && it.active()
                    && matches!(it.state, ItemState::OnGround | ItemState::Stored { .. })
                    && self.contents_of(c).is_empty()
            })
    }

    /// What an item is worth to a trader, contents and all. A barrel of wine
    /// leaves with the wine, so it must be priced with the wine — otherwise a
    /// caravan buys the fort's whole larder for the price of a barrel.
    pub fn stack_value(&self, idx: usize, raws: &Raws) -> u32 {
        let mut v = item_value(&self.items[idx], raws);
        if is_container(self.items[idx].kind) {
            for c in self.contents_of(idx) {
                v += item_value(&self.items[c], raws);
            }
        }
        v
    }

    /// The fort's created wealth: the summed trade value of everything it owns.
    /// Each active item is counted exactly once — `item_value` never recurses
    /// into a container's contents (only `stack_value` does), so a barrel and
    /// the wine inside it are both summed with no double-count and no O(n²)
    /// contents scan. Whether the wine sits loose or packed in the barrel, the
    /// total is the same.
    pub fn fortress_wealth(&self, raws: &Raws) -> u32 {
        let goods: u32 = self
            .items
            .iter()
            .filter(|it| it.active())
            .map(|it| item_value(it, raws))
            .sum();
        // Banked trade goodwill is wealth too — otherwise a fort could launder
        // its masterworks into credit to hide them from wealth-scaled sieges.
        goods + self.trade_credit.max(0) as u32
    }

    /// Refresh the cached wealth figure. Called on the day boundary; the HUD,
    /// siege planner, and migration pull all read `cached_wealth` rather than
    /// rescanning every item.
    pub fn recompute_wealth(&mut self, raws: &Raws) {
        self.cached_wealth = self.fortress_wealth(raws);
    }

    /// The fort's last-computed wealth (refreshed daily). See `fortress_wealth`.
    pub fn wealth(&self) -> u32 {
        self.cached_wealth
    }

    fn find_container_for(&self, kind: ItemKind, near: Pos, region: u32) -> Option<usize> {
        self.items
            .iter()
            .enumerate()
            .filter(|(c, it)| {
                it.active()
                    && matches!(it.state, ItemState::Stored { .. })
                    && self.regions.id(it.pos) == region
                    && self.container_accepts(*c, kind)
                    // The pile the cask stands in must be one that wants this
                    // cargo. Otherwise an empty barrel parked among the beds
                    // would quietly swallow the fort's larder into the
                    // furniture pile.
                    && self
                        .stockpile_at(it.pos)
                        .is_some_and(|s| self.stockpiles[s].takes(kind))
            })
            .min_by_key(|(_, it)| it.pos.manhattan(near))
            .map(|(c, _)| c)
    }

    /// Sell or destroy a container and everything packed inside it goes with
    /// it — a barrel of wine leaves with the wine. Never leave contents
    /// pointing at a container that no longer exists.
    fn consume_with_contents(&mut self, container: usize) {
        for i in self.contents_of(container) {
            self.items[i].consumed = true;
        }
        self.items[container].consumed = true;
    }

    /// A contained item rides with its container. Called once a tick so the
    /// invariant "contents sit on their container's tile" holds no matter who
    /// moved the container — carried by a hauler, dropped, or walked to a new
    /// region. Touches only `pos`, draws no RNG.
    fn sync_container_contents(&mut self) {
        for i in 0..self.items.len() {
            let ItemState::Inside { container } = self.items[i].state else {
                continue;
            };
            // A container that died out from under its contents (traded away,
            // eaten by obsidian) spills them onto its last tile rather than
            // leaving them referencing a corpse.
            match self.items.get(container) {
                Some(c) if c.active() => {
                    let p = c.pos;
                    self.items[i].pos = p;
                }
                _ => {
                    self.items[i].state = ItemState::OnGround;
                }
            }
        }
    }

    pub fn pending_designations(&self) -> usize {
        self.designations.len()
    }

    /// How many items the fort has put away — counting both those resting on
    /// a stockpile tile and those packed into a barrel or bin standing on one.
    /// (Packed goods are `Inside`, not `Stored`; counting only the latter would
    /// report a fort that had just tidied its whole larder into barrels as
    /// having stored nothing.)
    /// How many items the fort has put away — counting both those resting on
    /// a stockpile tile and those packed into a barrel or bin standing on one.
    /// (Packed goods are `Inside`, not `Stored`; counting only the latter would
    /// report a fort that had just tidied its whole larder into barrels as
    /// having stored nothing.)
    pub fn stored_items(&self) -> usize {
        self.items
            .iter()
            .filter(|i| {
                i.active()
                    && matches!(i.state, ItemState::Stored { .. } | ItemState::Inside { .. })
            })
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

    /// Living citizens who have grown into seasoned fighters (skill level 3+).
    pub fn veterans(&self) -> usize {
        self.dwarves
            .iter()
            .filter(|d| {
                d.alive && d.faction == Faction::Fort && d.skill_level(Skill::Fighting) >= 3
            })
            .count()
    }

    /// Is an item claimable as a consumable/ingredient right now?
    /// Can a dwarf walk up and take this item? Packed items count: they sit on
    /// their container's tile, so "go to it and pick it up" works the same
    /// whether it's lying on the floor or in a barrel. This one predicate gates
    /// every larder, workshop and forge query in the fort — if a contained item
    /// were not takeable here, dwarves would starve beside a full barrel.
    fn item_takeable(&self, it: &Item) -> bool {
        it.active()
            && it.reserved_by.is_none()
            && matches!(
                it.state,
                ItemState::OnGround | ItemState::Stored { .. } | ItemState::Inside { .. }
            )
    }

    /// Enter adventure mode: the first living fort dwarf becomes the
    /// player, and (if history supplied enemies) their nemesis spawns
    /// somewhere on the map as a quest target.
    pub fn begin_adventure(&mut self, raws: &Raws) -> Option<usize> {
        let hero = self
            .dwarves
            .iter()
            .position(|d| d.alive && d.faction == Faction::Fort)?;
        // Drop whatever the hero was doing (releasing any reservation / carried
        // item), since as the player their task never advances to completion.
        self.abandon_task(hero);
        self.player = Some(hero);
        self.invasions = false; // the world stands still for a duel
        let name = self.dwarves[hero].name.clone();
        self.log_event(format!("{name} sets out on an adventure."));

        // No hero walks into a saga bare-handed. If they carry no weapon,
        // hand them a sword to start — the roads are armed now.
        if self.wielded_weapon(hero, raws).is_none() {
            let metal = raws
                .materials
                .indices_in_category(MaterialCategory::Ore)
                .into_iter()
                .find(|&m| raws.materials.get(m).combat.sharpness >= 1.0)
                .unwrap_or(0);
            let pos = self.dwarves[hero].pos;
            self.spawn_item(ItemKind::Weapon, metal, pos);
            self.set_last_weapon(raws.weapons.index_of("sword").unwrap_or(0));
            let w = self.items.len() - 1;
            self.items[w].state = ItemState::Carried { by: hero };
        }

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

    /// Adventure mode: ask a nearby townsfolk to join the hunt. Recruits the
    /// nearest living, un-recruited fort-folk standing beside the hero and
    /// returns their name, or None if nobody is at hand.
    pub fn recruit_companion(&mut self) -> Option<String> {
        let hero = self.player?;
        let hero_pos = self.dwarves[hero].pos;
        let cand = self
            .dwarves
            .iter()
            .enumerate()
            .filter(|&(j, d)| {
                j != hero
                    && d.alive
                    && d.faction == Faction::Fort
                    && !d.follower
                    && d.pos.z == hero_pos.z
                    && d.pos.x.abs_diff(hero_pos.x) + d.pos.y.abs_diff(hero_pos.y) <= 1
            })
            .map(|(j, _)| j)
            .next()?;
        // Release whatever job the recruit was mid-way through (freeing its
        // item/animal/designation/farm/construction reservation) before taking
        // them off the job board as a follower.
        self.abandon_task(cand);
        self.dwarves[cand].follower = true;
        self.dwarves[cand].task = Task::Idle { wander_cd: 0 };
        let name = self.dwarves[cand].name.clone();
        self.log_event(format!("{name} joins your band."));
        Some(name)
    }

    /// Adventure travel: carry the hero into a fresh land. Their body,
    /// needs, skills, deeds, and quest come along; the old region's people
    /// and things are left behind. If the quest is unfinished, the nemesis
    /// follows to this new land so the hunt continues.
    pub fn relocate_player(&mut self, new_map: Map, raws: &Raws) {
        let Some(hero) = self.player else { return };
        let mut wanderer = self.dwarves[hero].clone();
        // The old land's bed stays in the old land. `Dwarf.bed` is an index
        // into the item vec, and that vec is about to be cleared — carried
        // across, it would point at whatever now sits in that slot, or off
        // the end of it.
        wanderer.bed = None;
        // Companions journey on with the hero; nobody else does.
        let mut companions: Vec<Dwarf> = self
            .dwarves
            .iter()
            .enumerate()
            .filter(|&(j, d)| j != hero && d.alive && d.follower)
            .map(|(_, d)| {
                let mut c = d.clone();
                c.bed = None; // as above: their beds do not travel either
                c
            })
            .collect();
        // Old dwarf index -> new index for everyone who travels: the hero
        // becomes 0, the companions follow in order. Used to keep carried
        // gear tracking its owner across the journey.
        let comp_old: Vec<usize> = self
            .dwarves
            .iter()
            .enumerate()
            .filter(|&(j, d)| j != hero && d.alive && d.follower)
            .map(|(j, _)| j)
            .collect();
        let mut remap: BTreeMap<usize, usize> = BTreeMap::new();
        remap.insert(hero, 0);
        for (k, &oj) in comp_old.iter().enumerate() {
            remap.insert(oj, k + 1);
        }
        // Carry the travellers' held items with them; everything left on the
        // ground or stored in the old land stays behind.
        let mut carried: Vec<Item> = self
            .items
            .iter()
            .filter(|it| it.active())
            .filter_map(|it| match it.state {
                ItemState::Carried { by } => remap.get(&by).map(|&nb| {
                    let mut c = it.clone();
                    c.state = ItemState::Carried { by: nb };
                    c.reserved_by = None;
                    c
                }),
                _ => None,
            })
            .collect();

        // A clean land: replace the map and clear everything of the old one.
        self.map = new_map;
        self.items.clear();
        self.animals.clear();
        self.stockpiles.clear();
        self.pastures.clear();
        self.taverns.clear();
        self.temples.clear();
        self.fisheries.clear();
        self.hospitals.clear();
        self.barracks.clear();
        // The squads reference fort dwarf indices, gone with the old roster.
        self.squads.clear();
        self.bolts = 0;
        self.burrows.clear();
        self.alarm = false;
        self.library.clear();
        self.bedrooms.clear();
        self.dining.clear();
        self.treatises.clear();
        self.poems.clear();
        self.songs.clear();
        // The barony is left behind with the fort; its stale dwarf index would
        // otherwise make tick_nobility panic on the small new-land roster.
        self.baron = None;
        self.mandate = None;
        self.buildings.clear();
        self.farms.clear();
        self.designations.clear();
        self.engravings.clear();
        self.constructions.clear();
        self.trees.clear();
        self.tree_cap = 0;
        self.shrubs.clear();
        self.shrub_cap = 0;
        self.adamantine_breaches.clear();
        self.caravan = None;
        self.water = WaterSim::default();
        if let Some(spring) = natural_spring(&self.map) {
            self.water.springs.insert(spring);
        }
        self.magma = FluidSim::magma();
        self.rebuild_caches();

        // Set the wanderer down at a walkable spot near the map's edge.
        let entry = self
            .map
            .walk_surface_z(2, self.map.height / 2)
            .map(|z| Pos::new(2, self.map.height as i32 / 2, z as i32))
            .or_else(|| {
                (0..self.map.height).find_map(|y| {
                    (0..self.map.width).find_map(|x| {
                        self.map
                            .walk_surface_z(x, y)
                            .map(|z| Pos::new(x as i32, y as i32, z as i32))
                    })
                })
            });
        let Some(entry) = entry else { return };
        wanderer.pos = entry;
        wanderer.task = Task::Idle { wander_cd: 0 };
        self.dwarves = vec![wanderer];
        self.player = Some(0);
        self.quest_target = None;

        // Set the companions down on walkable ground beside the hero,
        // fanning outward so they don't all stack on one tile.
        let occupied = |dwarves: &[Dwarf], p: Pos| dwarves.iter().any(|d| d.alive && d.pos == p);
        for mut comp in companions.drain(..) {
            let spot = 'find: {
                for r in 1..6i32 {
                    for dy in -r..=r {
                        for dx in -r..=r {
                            let (x, y) = (entry.x + dx, entry.y + dy);
                            if x < 0 || y < 0 {
                                continue;
                            }
                            if let Some(z) = self.map.walk_surface_z(x as usize, y as usize) {
                                let p = Pos::new(x, y, z as i32);
                                if !occupied(&self.dwarves, p) {
                                    break 'find p;
                                }
                            }
                        }
                    }
                }
                entry
            };
            comp.pos = spot;
            comp.task = Task::Idle { wander_cd: 0 };
            self.dwarves.push(comp);
        }
        // The travellers' gear comes to rest wherever its owner now stands.
        for mut it in carried.drain(..) {
            if let ItemState::Carried { by } = it.state {
                if let Some(d) = self.dwarves.get(by) {
                    it.pos = d.pos;
                }
            }
            self.items.push(it);
        }

        let name = self.dwarves[0].name.clone();
        self.log_event(format!("{name} travels into a new land."));

        // The quarry follows the hunt to this new country.
        if let Some((target_name, false)) = self.quest.clone() {
            let hero_pos = self.dwarves[0].pos;
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
                self.spawn_raider_at(p, raws);
                let idx = self.dwarves.len() - 1;
                self.dwarves[idx].name = target_name.clone();
                self.quest_target = Some(idx);
                self.log_event(format!("{target_name} has followed you here."));
            }
        }
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
            PlayerAction::Grab => {
                let me = self.dwarves[hero].pos;
                if let Some(idx) = self.items.iter().position(|it| {
                    it.pos == me && it.active() && it.state == ItemState::OnGround
                }) {
                    self.items[idx].state = ItemState::Carried { by: hero };
                    self.items[idx].reserved_by = Some(hero);
                    let label = match self.items[idx].kind {
                        ItemKind::Weapon => "a weapon",
                        _ => "something",
                    };
                    let name = self.dwarves[hero].name.clone();
                    self.log_event(format!("{name} takes up {label}."));
                }
            }
            PlayerAction::Move(dx, dy) => {
                let me = self.dwarves[hero].pos;
                // Creatures can share tiles; an enemy standing on ours gets
                // dealt with before anything else.
                let here_enemy = self.dwarves.iter().position(|d| {
                    d.alive && d.faction == Faction::Hostile && d.pos == me
                });
                if let Some(enemy) = here_enemy {
                    self.dwarves[hero].attack_cd = 0; // one swing per turn
                    self.melee(hero, enemy, raws);
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
                        self.melee(hero, enemy, raws);
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
                    && matches!(it.kind, ItemKind::Meal | ItemKind::Crop | ItemKind::Berry)
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
    /// Strike a dwarf dead where they stand, for tests that need a corpse or
    /// an heir without staging a siege.
    /// Put an item into a dwarf's hands — for tests that need an armed raider.
    pub fn debug_carry_item(&mut self, item: usize, by: usize) {
        self.items[item].state = ItemState::Carried { by };
    }

    /// Drill combat prowess into a dwarf — for tests that need a veteran
    /// without a season of sparring.
    pub fn debug_add_xp(&mut self, i: usize, skill: Skill, xp: u32) {
        self.add_xp(i, skill, xp);
    }

    /// Drop a single bare-handed raider on a tile — for combat tests that
    /// want a fight without staging a whole siege.
    pub fn debug_spawn_raider_at(&mut self, pos: Pos, raws: &Raws) -> usize {
        let mut r = new_dwarf(&mut self.rng, pos, Faction::Hostile, raws);
        r.name = format!("raider {}", names::dwarf_name(&mut self.rng));
        self.dwarves.push(r);
        self.dwarves.len() - 1
    }

    pub fn debug_kill_dwarf(&mut self, i: usize) {
        self.kill_dwarf(i);
    }

    pub fn debug_spawn_boulder(&mut self, material: u16, pos: Pos) {
        self.spawn_item(ItemKind::Boulder, material, pos);
    }

    /// Drop a mug of drink on the ground (scenarios/tests).
    pub fn debug_spawn_drink(&mut self, pos: Pos) {
        self.spawn_item(ItemKind::Drink, 0, pos);
    }

    /// Drop a suit of armor on the ground (scenarios/tests). The armory issues
    /// it to enlisted soldiers by index, so this armors the fort's soldiers.
    pub fn debug_spawn_armor(&mut self, material: u16, pos: Pos) {
        self.spawn_item(ItemKind::Armor, material, pos);
    }

    /// Drop a bed on the ground (scenarios/tests). Beds are claimed by citizens
    /// in index order, so this gives the fort's first citizens a bed to rest in.
    pub fn debug_spawn_bed(&mut self, material: u16, pos: Pos) {
        self.spawn_item(ItemKind::Bed, material, pos);
    }

    /// Drop a bolt of cloth on the ground (scenarios/tests) — the clothier's stock.
    pub fn debug_spawn_cloth(&mut self, pos: Pos) {
        self.spawn_item(ItemKind::Cloth, 0, pos);
    }

    /// Drop a felled log on the ground (scenarios/tests) — the carpenter's stock.
    pub fn debug_spawn_log(&mut self, pos: Pos) {
        self.spawn_item(ItemKind::Log, 0, pos);
    }

    /// Drop a statue on the ground (scenarios/tests). Any statue beautifies the
    /// whole fort, easing every citizen's stress a touch.
    pub fn debug_spawn_statue(&mut self, material: u16, pos: Pos) {
        self.spawn_item(ItemKind::Statue, material, pos);
    }

    /// Drop a raw hide on the ground (scenarios/tests) — the tanner's stock.
    pub fn debug_spawn_hide(&mut self, pos: Pos) {
        self.spawn_item(ItemKind::Hide, 0, pos);
    }

    /// Drop a corpse on the ground (scenarios/tests) — a necromancer's fodder.
    pub fn debug_spawn_corpse(&mut self, pos: Pos) {
        self.spawn_item(ItemKind::Corpse, 0, pos);
    }

    /// Drop a skeletonized bone on the ground (scenarios/tests) — the
    /// craftsdwarf's stock for bone trinkets. `stuff` = 2 is the skeletal stage.
    pub fn debug_spawn_bone(&mut self, pos: Pos) {
        self.spawn_named_item(ItemKind::BodyPart, 2, pos, Some("left arm".to_string()));
    }

    /// Drop an arbitrary item on the ground (scenarios/tests).
    pub fn debug_spawn_item(&mut self, kind: ItemKind, stuff: u16, pos: Pos) {
        self.spawn_item(kind, stuff, pos);
    }

    /// Spawn a weapon of a specific kind (scenarios/tests) — e.g. a crossbow
    /// for a marksdwarf. Ordinary forging cycles the melee kinds, so this is
    /// how a test puts a particular arm in the armoury.
    pub fn debug_spawn_weapon(&mut self, variant: u16, stuff: u16, pos: Pos) {
        self.spawn_item(ItemKind::Weapon, stuff, pos);
        self.set_last_weapon(variant);
    }

    /// The kind of weapon this soldier would draw from the armoury right now —
    /// The weapon-registry index of what `wielded_weapon` resolves to, exposed
    /// for scenarios/tests (compare against `raws.weapons.index_of(id)`).
    pub fn debug_weapon_variant(&self, i: usize, raws: &Raws) -> Option<u16> {
        self.wielded_weapon(i, raws).and_then(|w| self.items[w].weapon_variant())
    }

    /// Drop a set of clothes on the ground (scenarios/tests). Clothes are worn
    /// by citizens in index order, so this dresses the fort's first citizens.
    pub fn debug_spawn_clothes(&mut self, pos: Pos) {
        self.spawn_item(ItemKind::Clothes, 0, pos);
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
        // A vampire betrays itself once the fort knows a killer walks among
        // them: folk realise they've never seen this one eat, drink, or sleep.
        if d.alive && d.vampire && self.stats.drained > 0 {
            out.push_str(
                " Unsettlingly, no one can recall ever seeing them eat, drink, or sleep \
                 — and folk have begun to whisper.",
            );
        }
        if !d.alive {
            out.push_str(" They are gone now, and missed.");
        }
        out
    }

    /// Bring a suspected vampire to justice. If the accused truly is the fort's
    /// secret vampire, they are put to death and the killings end; if not, an
    /// innocent hangs, the fort is shaken by the miscarriage, and the true
    /// horror walks free. Returns whether the accused was guilty.
    pub fn accuse(&mut self, i: usize) -> bool {
        if i >= self.dwarves.len()
            || !self.dwarves[i].alive
            || self.dwarves[i].faction != Faction::Fort
        {
            return false;
        }
        let guilty = self.dwarves[i].vampire;
        let name = self.dwarves[i].name.clone();
        self.kill_dwarf(i);
        if guilty {
            self.log_event(format!(
                "{name} is dragged into the light — a vampire! Justice is done, \
                 and the fort may sleep easy at last."
            ));
        } else {
            self.log_event(format!(
                "{name} is put to death on the fort's suspicion — but was no vampire. \
                 An innocent is dead, and the true horror still walks among us."
            ));
            for j in 0..self.dwarves.len() {
                if self.dwarves[j].alive && self.dwarves[j].faction == Faction::Fort {
                    self.push_thought(j, ThoughtKind::SawPunishment);
                }
            }
        }
        guilty
    }

    // ------------------------------------------------------------- stepping

    pub fn step(&mut self, raws: &Raws) {
        self.clock.advance();
        // Whatever moved a container last tick, its contents ride with it.
        self.sync_container_contents();

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
        // Bleeding creatures drip blood as they stagger about; it dries and
        // fades over the following day. Positions collected first so the spatter
        // (which takes &mut self) doesn't clash with the iteration.
        if self.clock.tick % 20 == 0 {
            let drips: Vec<Pos> = self
                .dwarves
                .iter()
                .filter(|d| d.alive && d.body.iter().any(|p| p.bleeding > 0))
                .map(|d| d.pos)
                .collect();
            for p in drips {
                self.spatter_blood(p, 22);
            }
        }
        if self.clock.tick % BLOOD_DRY_INTERVAL == 0 && self.clock.tick > 0 {
            self.dry_blood();
        }
        // Gore rots on the same lazy cadence — the process spans days.
        if self.clock.tick % (TICKS_PER_DAY / 4) == 0 && self.clock.tick > 0 {
            self.decay_gore();
        }
        self.tick_footprints();
        // Region rebuilds are throttled; A* remains the authority in between.
        if self.regions.dirty && self.clock.tick % REGION_REBUILD_INTERVAL == 0 {
            self.regions.rebuild(&self.map);
        }

        // The fort's beds are handed out once a day; nothing changes between.
        if self.clock.tick % TICKS_PER_DAY == 0 {
            self.tick_bedrooms();
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
            if self.dwarves[i].follower && self.player.is_some() {
                self.follow_hero(i, raws);
                continue;
            }
            match self.dwarves[i].faction {
                Faction::Fort => self.update_dwarf(i, raws),
                Faction::Hostile => self.update_hostile(i, raws),
                Faction::Visitor => self.update_visitor(i, raws),
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
            self.tick_nobility(raws);
            self.tick_weather();
            self.tick_culture();
        }
        // The unquiet dead stir every other day.
        if self.clock.tick % (2 * TICKS_PER_DAY) == 0 && self.clock.tick > 0 {
            self.tick_ghosts();
        }
        // Livestock wander a little each tick; herd bookkeeping is daily.
        self.tick_animals_movement();
        self.tick_war_animals();
        self.tick_vampires();
        self.tick_werebeasts();
        self.tick_necromancers(raws);
        self.tick_regrowth();
        if self.clock.tick % TICKS_PER_DAY == 0 && self.clock.tick > 0 {
            self.tick_animals_husbandry();
            self.tick_spoilage();
            // Reckon the fort's worth once a day, after spoilage has taken its
            // due, so the HUD and the season-boundary siege below read a fresh
            // figure. (A season boundary is always a day boundary too.)
            self.recompute_wealth(raws);
        }
        self.tick_cats();
        self.tick_vermin();

        // Season boundary: migrants, moods, and (later years) raiders.
        if self.clock.tick % season_ticks == 0 && self.clock.tick > 0 {
            self.maybe_migrants(raws);
            self.maybe_strange_mood();
            let seasons_elapsed = self.clock.tick / season_ticks;
            // Cap active hostiles so stuck raiders don't accumulate season
            // over season into an unbounded horde.
            if self.invasions && seasons_elapsed >= 2 && self.alive_hostiles() < 8 {
                // Sieges scale to the fort's WEALTH, not its clutter: a hoard of
                // worthless gravel draws nothing, but made goods — crafts, arms,
                // gems, statues — invite raiders. `cached_wealth` was refreshed
                // in today's daily block above.
                let n = raider_wave_size(self.cached_wealth) as usize;
                self.spawn_raiders(n, raws);
                // Some sieges bring a necromancer who raises the fort's own
                // dead against it. Chosen deterministically (no rng) so it never
                // perturbs the raider spawns; gated behind invasions, which no
                // headless test enables.
                if seasons_elapsed % 4 == 0 {
                    if let Some(d) = self
                        .dwarves
                        .iter_mut()
                        .rev()
                        .find(|d| d.alive && d.faction == Faction::Hostile && !d.necromancer)
                    {
                        d.necromancer = true;
                        self.log_event(
                            "A necromancer marches with them -- the dead will not rest!".to_string(),
                        );
                    }
                }
                // Timid dwarves take the news badly.
                for i in 0..self.dwarves.len() {
                    let d = &self.dwarves[i];
                    if d.alive && d.faction == Faction::Fort && d.personality.bravery < 40.0 {
                        self.push_thought(i, ThoughtKind::ScaredBySiege);
                    }
                }
                // Dig greedily and you may wake something older than any
                // grudge. Rare, and never more than one at a time.
                let deep = self.stats.boulders_mined > 40;
                let a_beast_walks = self.dwarves.iter().any(|d| d.alive && d.beast);
                if deep && !a_beast_walks && self.rng.gen_ratio(1, 4) {
                    self.emerge_beast(raws);
                }
            }
        }

        // The fortress falls when its last citizen is gone. (In adventure
        // mode the lone hero's death is the story's end, handled elsewhere.)
        // A citizen transformed by the full moon still lives — it will revert to
        // a fort-mate at dawn — so it must not count as a fallen fort. Without
        // this guard, a fort whose only survivors are all werebeasts at moonrise
        // would latch a permanent, false game-over.
        let a_beast_will_return = self.dwarves.iter().any(|d| d.alive && d.were_form);
        if self.fallen_at.is_none()
            && self.player.is_none()
            && self.alive_dwarves() == 0
            && !a_beast_will_return
        {
            self.fallen_at = Some(self.clock.tick);
            self.log_event("The fortress has fallen. Its halls stand silent.".to_string());
        }
    }

    /// True once the last citizen has died.
    pub fn fallen(&self) -> bool {
        self.fallen_at.is_some()
    }

    /// The fortress's legacy — a short account of what it achieved, shown
    /// when it falls.
    pub fn epitaph(&self) -> String {
        let s = &self.stats;
        format!(
            "The fortress endured {} year(s) and {} season(s).\n\
             It lost {} of its own, and slew {} raiders and {} forgotten beast(s).\n\
             Its people harvested {} crops, cooked {} meals, brewed {} drinks,\n\
             mined {} stone, cut {} gems, wove {} cloth, and created works of legend.\n\
             {} migrants sought its gates; {} caravans came to trade.\n\
             Let it be remembered.",
            self.clock.year().saturating_sub(1),
            (self.clock.tick / (TICKS_PER_DAY * DAYS_PER_SEASON)) % SEASONS_PER_YEAR,
            s.deaths,
            s.raiders_slain,
            s.beasts_slain,
            s.crops_harvested,
            s.meals_cooked,
            s.drinks_brewed,
            s.boulders_mined,
            s.gems_cut,
            s.cloth_woven,
            s.migrants_arrived,
            s.caravans_arrived,
        )
    }

    /// A forgotten beast surfaces at a deep, fort-reachable spot (a mined
    /// tile far below), or the map edge if none is found.
    fn emerge_beast(&mut self, raws: &Raws) {
        let Some(anchor) = self.dwarves.iter().find(|d| d.alive && d.faction == Faction::Fort).map(|d| d.pos) else {
            return;
        };
        let anchor_region = self.regions.id(anchor);
        // Prefer the deepest reachable floor away from the citizens.
        let mut spot: Option<Pos> = None;
        let mut best_z = i32::MAX;
        for z in 0..(self.map.depth as i32).min(anchor.z) {
            for y in (0..self.map.height).step_by(3) {
                for x in (0..self.map.width).step_by(3) {
                    let p = Pos::new(x as i32, y as i32, z);
                    if self.map.walkable(p)
                        && self.regions.id(p) == anchor_region
                        && p.manhattan(anchor) > 15
                        && z < best_z
                    {
                        best_z = z;
                        spot = Some(p);
                    }
                }
            }
        }
        if let Some(p) = spot {
            self.spawn_forgotten_beast(p, raws);
            for i in 0..self.dwarves.len() {
                if self.dwarves[i].alive && self.dwarves[i].faction == Faction::Fort {
                    self.push_thought(i, ThoughtKind::ScaredBySiege);
                }
            }
        }
    }

    /// Spawn a single raider at an exact position (tests/scenarios).
    pub fn spawn_raider_at(&mut self, pos: Pos, raws: &Raws) {
        let mut r = new_dwarf(&mut self.rng, pos, Faction::Hostile, raws);
        r.name = format!("raider {}", names::dwarf_name(&mut self.rng));
        self.dwarves.push(r);
        let idx = self.dwarves.len() - 1;
        self.arm_raider(idx, raws);
        self.stats.raiders_arrived += 1;
    }

    /// A forgotten beast rises from the deep — a hostile of monstrous
    /// toughness with a generated name and form. Returns its dwarf index.
    pub fn spawn_forgotten_beast(&mut self, pos: Pos, raws: &Raws) -> usize {
        let (name, form) = names::beast_name(&mut self.rng);
        let mut b = new_dwarf(&mut self.rng, pos, Faction::Hostile, raws);
        b.name = name.clone();
        b.beast = true;
        b.body = beast_body();
        self.dwarves.push(b);
        let idx = self.dwarves.len() - 1;
        self.log_event(format!(
            "A forgotten beast has risen from the depths! {name}, {form}, stalks the caverns."
        ));
        idx
    }

    /// A lesser cave creature at `pos` — a hostile beast of the caverns,
    /// hardier than a raider but no forgotten beast. Returns its dwarf index.
    pub fn spawn_cave_creature(&mut self, pos: Pos, raws: &Raws) -> usize {
        let kind = CAVE_CREATURES[self.rng.gen_range(0..CAVE_CREATURES.len())];
        let mut c = new_dwarf(&mut self.rng, pos, Faction::Hostile, raws);
        c.name = format!("a {kind}");
        c.beast = true; // a monster: it cannot dodge, but strikes hard and is tough
        c.body = cave_beast_body();
        self.dwarves.push(c);
        self.dwarves.len() - 1
    }

    /// Populate the cavern with lurking creatures — a fort that breaks into the
    /// deep meets them. App-embark-only (needs cavern floors), so a fort with no
    /// cavern, and every headless test, is unchanged.
    pub fn populate_caverns(&mut self, count: usize, raws: &Raws) {
        let spots: Vec<Pos> = self
            .cavern_floors
            .iter()
            .copied()
            .filter(|&p| self.map.walkable(p))
            .collect();
        if spots.is_empty() {
            return;
        }
        for _ in 0..count {
            let p = spots[self.rng.gen_range(0..spots.len())];
            self.spawn_cave_creature(p, raws);
        }
    }

    /// A demon of the underworld: a hostile of monstrous form, nastier even
    /// than a forgotten beast. Returns its dwarf index.
    pub fn spawn_demon(&mut self, pos: Pos, raws: &Raws) -> usize {
        let (name, _form) = names::beast_name(&mut self.rng);
        let mut d = new_dwarf(&mut self.rng, pos, Faction::Hostile, raws);
        d.name = format!("{name}, a demon");
        d.beast = true;
        d.body = beast_body();
        // The abyss-born strike harder still.
        for part in &mut d.body {
            part.max_hp = (part.max_hp * 3) / 2;
            part.hp = part.max_hp;
        }
        self.dwarves.push(d);
        self.dwarves.len() - 1
    }

    /// Dig too deep and pay for it: mining a hollow adamantine cap opens the
    /// underworld and looses a horde of demons at the breach. Gated on a breach
    /// existing (adamantine is seeded only at app embark), so a fort that never
    /// strikes one draws no rng here and steps byte-identically.
    fn breach_underworld(&mut self, pos: Pos, raws: &Raws) {
        self.log_event(
            "The miners break into a hollow of adamantine — and beyond it, the \
             underworld gapes. You have unleashed what waited below."
                .to_string(),
        );
        // Clear a small pocket at the breach so the demons have somewhere to be.
        let mut spots = Vec::new();
        for (dx, dy) in [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (-1, -1)] {
            let p = Pos::new(pos.x + dx, pos.y + dy, pos.z);
            if let Some(t) = self.map.tile_at(p) {
                if t.is_solid() {
                    self.map.set_at(
                        p,
                        Tile { material: t.material, shape: TileShape::Floor, water: 0, magma: 0 },
                    );
                }
                spots.push(p);
            }
        }
        self.map_changed = true;
        self.regions.dirty = true;
        let horde = 3 + self.rng.gen_range(0..4); // 3..6 demons
        for k in 0..horde {
            let p = spots.get(k % spots.len().max(1)).copied().unwrap_or(pos);
            self.spawn_demon(p, raws);
            self.stats.demons_loosed += 1;
        }
    }

    /// Put a weapon in a raider's hand. Invaders come armed — an unarmed
    /// horde is no threat at all now that a bare fist barely bruises — with a
    /// random weapon of a middling metal, forged somewhere in their own lands.
    fn arm_raider(&mut self, i: usize, raws: &Raws) {
        // A weapon-grade metal from the raws, so a raider's blade actually
        // cuts. Falls back to the first material if none is marked.
        let metal = raws
            .materials
            .indices_in_category(MaterialCategory::Ore)
            .into_iter()
            .find(|&m| raws.materials.get(m).combat.sharpness >= 1.0)
            .unwrap_or(0);
        // Raiders close and strike — they carry melee steel, never a crossbow
        // (they have no marksmanship AI, and a crossbow is a feeble club). Drawn
        // from the five melee kinds, which also keeps the rng stream identical
        // to before the crossbow was added.
        let melee = raws.weapons.melee_indices();
        let variant = melee[self.rng.gen_range(0..melee.len())];
        let pos = self.dwarves[i].pos;
        self.spawn_item(ItemKind::Weapon, metal, pos);
        self.set_last_weapon(variant);
        let w = self.items.len() - 1;
        self.items[w].state = ItemState::Carried { by: i };
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
                    let idx = self.dwarves.len() - 1;
                    self.arm_raider(idx, raws);
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
        // Rain waters the crops: they grow half again as fast.
        let step = if self.weather == Weather::Rain { 2 } else { 1 };
        for tile in self.farms.values_mut() {
            if let FarmState::Growing { progress } = tile.state {
                let plant = raws.plants.get(tile.crop);
                if !plant.grows_in(season) {
                    continue; // dormant out of season
                }
                let done = plant.grow_days as u64 * TICKS_PER_DAY;
                let next = progress as u64 + step;
                tile.state = if next >= done {
                    FarmState::Grown
                } else {
                    FarmState::Growing { progress: next as u32 }
                };
            }
        }
    }

    /// The sky turns with the seasons: rainy springs and autumns, snowy
    /// winters, mostly clear summers. Rolled once a day.
    fn tick_weather(&mut self) {
        let roll = self.rng.gen_range(0..100u32);
        self.weather = match self.clock.season() {
            Season::Winter => {
                if roll < 45 { Weather::Snow } else { Weather::Clear }
            }
            Season::Summer => {
                if roll < 15 { Weather::Rain } else { Weather::Clear }
            }
            // Spring and autumn are the wet seasons.
            _ => {
                if roll < 40 { Weather::Rain } else { Weather::Clear }
            }
        };
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
            // War dogs don't graze — they patrol and charge, handled entirely
            // in tick_war_animals, which also owns their move_cd. Skip them
            // here so the cooldown isn't decremented twice (they'd charge
            // faster than WALK_COOLDOWN intends) and no wander RNG is drawn.
            if self.animals[idx].war {
                continue;
            }
            // Nor do cats: they work the larder, and `tick_cats` owns their
            // move_cd. A cat that wandered off like a sheep would be no use at
            // all — which is exactly what happened when they did.
            if self.animals[idx].kind == AnimalKind::Cat {
                continue;
            }
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

    /// War dogs guard the fort: each charges the nearest raider and savages
    /// any it can reach. Runs only when trained guardians exist, so ordinary
    /// fortress play draws no extra RNG.
    fn tick_war_animals(&mut self) {
        for idx in 0..self.animals.len() {
            if !self.animals[idx].alive || !self.animals[idx].war {
                continue;
            }
            if self.animals[idx].atk_cd > 0 {
                self.animals[idx].atk_cd -= 1;
            }
            if self.animals[idx].move_cd > 0 {
                self.animals[idx].move_cd -= 1;
            }
            let apos = self.animals[idx].pos;
            // The nearest raider on this level is the quarry.
            let target = self
                .dwarves
                .iter()
                .enumerate()
                .filter(|(_, d)| d.alive && d.faction == Faction::Hostile && d.pos.z == apos.z)
                .min_by_key(|(_, d)| d.pos.manhattan(apos))
                .map(|(j, _)| j);
            let Some(target) = target else { continue };
            let tpos = self.dwarves[target].pos;
            let adjacent = tpos.x.abs_diff(apos.x) + tpos.y.abs_diff(apos.y) <= 1;
            if adjacent {
                if self.animals[idx].atk_cd == 0 {
                    self.animals[idx].atk_cd = ATTACK_COOLDOWN;
                    self.dog_bite(idx, target);
                }
            } else if apos.manhattan(tpos) <= WAR_DOG_ENGAGE && self.animals[idx].move_cd == 0 {
                let mut opts = Vec::with_capacity(8);
                path::neighbors(&self.map, apos, &mut opts);
                if let Some(&next) = opts.iter().min_by_key(|q| q.manhattan(tpos)) {
                    if next.manhattan(tpos) < apos.manhattan(tpos) {
                        self.animals[idx].pos = next;
                        self.animals[idx].move_cd = WALK_COOLDOWN;
                    }
                }
            }
        }
    }

    /// A war dog's bite: maul a random body part of the raider, and if a
    /// vital gives way, the raider falls.
    fn dog_bite(&mut self, dog: usize, defender: usize) {
        let roll = self.rng.gen_range(0..8usize);
        let part_kind = match roll {
            0 => PartKind::Head,
            1 | 2 | 3 => PartKind::Torso,
            4 => PartKind::LeftArm,
            5 => PartKind::RightArm,
            6 => PartKind::LeftLeg,
            _ => PartKind::RightLeg,
        };
        let dmg = self.rng.gen_range(6..=14) as i16;
        let bleed = self.rng.gen_range(1..=2) as u8;
        let kind = self.animals[dog].kind.name();
        let def_name = self.dwarves[defender].name.clone();
        let d = &mut self.dwarves[defender];
        let Some(part) = d.body.iter_mut().find(|pt| pt.kind == part_kind) else { return };
        part.hp -= dmg;
        part.bleeding = part.bleeding.saturating_add(bleed);
        let destroyed = part.hp <= 0;
        let just_destroyed = destroyed && part.hp + dmg > 0;
        let vital = part.kind.vital();
        self.log_event(format!("A war {kind} savages {def_name}'s {}!", part_kind.name()));
        // A limb hewn off maims; a head hewn off decapitates on top of death.
        if just_destroyed && (!vital || part_kind == PartKind::Head) {
            self.sever_part(defender, part_kind);
        }
        if destroyed && vital {
            self.log_event(format!("{def_name} falls dead!"));
            let was_beast = self.dwarves[defender].beast;
            let was_hostile = self.dwarves[defender].faction == Faction::Hostile;
            self.kill_dwarf(defender);
            if was_beast {
                self.stats.beasts_slain += 1;
            } else if was_hostile {
                self.stats.raiders_slain += 1;
            }
        }
    }

    /// Is there an armed weapon trap on this tile?
    fn trap_at(&self, p: Pos) -> bool {
        self.buildings
            .iter()
            .any(|b| b.pos == p && b.kind == BuildingKind::Trap)
    }

    /// Hidden blades tear into a raider that stepped onto a weapon trap.
    fn spring_trap(&mut self, i: usize) {
        let roll = self.rng.gen_range(0..8usize);
        let part_kind = match roll {
            0 => PartKind::Head,
            1 | 2 | 3 => PartKind::Torso,
            4 => PartKind::LeftArm,
            5 => PartKind::RightArm,
            6 => PartKind::LeftLeg,
            _ => PartKind::RightLeg,
        };
        let dmg = self.rng.gen_range(15..=35) as i16;
        let bleed = self.rng.gen_range(2..=4) as u8;
        let name = self.dwarves[i].name.clone();
        let d = &mut self.dwarves[i];
        let Some(part) = d.body.iter_mut().find(|pt| pt.kind == part_kind) else { return };
        part.hp -= dmg;
        part.bleeding = part.bleeding.saturating_add(bleed);
        let destroyed = part.hp <= 0;
        let just_destroyed = destroyed && part.hp + dmg > 0;
        let vital = part.kind.vital();
        self.log_event(format!("A weapon trap tears into {name}!"));
        // A limb hewn off maims; a head hewn off decapitates on top of death.
        if just_destroyed && (!vital || part_kind == PartKind::Head) {
            self.sever_part(i, part_kind);
        }
        if destroyed && vital {
            self.log_event(format!("{name} falls dead!"));
            let was_beast = self.dwarves[i].beast;
            self.kill_dwarf(i);
            if was_beast {
                self.stats.beasts_slain += 1;
            } else {
                self.stats.raiders_slain += 1;
            }
        }
    }

    /// An alive war dog standing next to dwarf `i`, on the same level.
    fn adjacent_war_dog(&self, i: usize) -> Option<usize> {
        let me = self.dwarves[i].pos;
        self.animals
            .iter()
            .enumerate()
            .filter(|(_, a)| a.alive && a.war && a.pos.z == me.z)
            .find(|(_, a)| a.pos.x.abs_diff(me.x) + a.pos.y.abs_diff(me.y) <= 1)
            .map(|(j, _)| j)
    }

    /// A raider strikes back at a war dog blocking its way.
    fn maul_dog(&mut self, attacker: usize, dog: usize) {
        if self.dwarves[attacker].attack_cd > 0 {
            self.dwarves[attacker].attack_cd -= 1;
            return;
        }
        self.dwarves[attacker].attack_cd = ATTACK_COOLDOWN;
        let base = if self.dwarves[attacker].beast {
            self.rng.gen_range(20..=40) as i16
        } else {
            self.rng.gen_range(6..=16) as i16
        };
        let dmg = base + fighting_bonus(self.dwarves[attacker].skill_level(Skill::Fighting));
        self.animals[dog].hp -= dmg;
        let name = self.animals[dog].kind.name();
        let att = self.dwarves[attacker].name.clone();
        if self.animals[dog].hp <= 0 {
            self.animals[dog].alive = false;
            self.animals[dog].reserved_by = None;
            self.log_event(format!("{att} cuts down a war {name}."));
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
    fn tick_nobility(&mut self, raws: &Raws) {
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

        let deadline = self.clock.tick + MANDATE_DAYS * TICKS_PER_DAY;
        match self.mandate {
            None => {
                // With a caravan to sell to, the baron may instead forbid an
                // export (a DF noble's caprice). This branch — and its rng — runs
                // ONLY for a fort with a trade partner, so forts without trade
                // (every headless nobility test) keep the exact production-mandate
                // stream, gated by the short-circuit on `trade_partner`.
                let mut export_ban = None;
                if self.trade_partner.is_some() && self.rng.gen_ratio(1, 3) {
                    let cands: Vec<u16> = self
                        .items
                        .iter()
                        .filter(|it| {
                            it.active()
                                && matches!(
                                    it.kind,
                                    ItemKind::Boulder
                                        | ItemKind::Bar
                                        | ItemKind::Craft
                                        | ItemKind::Weapon
                                        | ItemKind::Armor
                                        | ItemKind::Statue
                                )
                        })
                        .map(|it| it.stuff)
                        .collect();
                    if !cands.is_empty() {
                        export_ban = Some(cands[self.rng.gen_range(0..cands.len())]);
                    }
                }
                let mandate = if let Some(target) = export_ban {
                    Mandate {
                        kind: MandateKind::ExportBan,
                        amount: 0,
                        deadline,
                        baseline: 0,
                        target,
                        violated: false,
                    }
                } else {
                    // The production quota, colored by the baron's tastes —
                    // unchanged from before, so the rng stream is identical.
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
                        MandateKind::ExportBan => 0,
                    };
                    Mandate { kind, amount, deadline, baseline, target: 0, violated: false }
                };
                self.mandate = Some(mandate);
                let name = self.dwarves[baron].name.clone();
                let verb = if mandate.kind == MandateKind::ExportBan { "decrees" } else { "demands" };
                self.log_event(format!(
                    "Baron {name} {verb} that {} within {MANDATE_DAYS} days!",
                    mandate.describe(raws)
                ));
            }
            Some(m) if m.kind == MandateKind::ExportBan => {
                // A passive prohibition: nothing to check until it lapses, then
                // the baron judges whether it was honoured.
                if self.clock.tick >= m.deadline {
                    self.mandate = None;
                    let name = self.dwarves[baron].name.clone();
                    if m.violated {
                        self.stats.mandates_failed += 1;
                        self.punish_for_mandate(baron, m, raws);
                    } else {
                        self.stats.mandates_met += 1;
                        self.push_thought(baron, ThoughtKind::MandateMet);
                        self.log_event(format!("Baron {name}'s edict held; the ban lifts."));
                    }
                }
            }
            Some(m) => {
                let progress = match m.kind {
                    MandateKind::CookMeals => self.stats.meals_cooked - m.baseline,
                    MandateKind::BrewDrinks => self.stats.drinks_brewed - m.baseline,
                    MandateKind::MineBoulders => self.stats.boulders_mined - m.baseline,
                    MandateKind::ExportBan => 0,
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
                    self.punish_for_mandate(baron, m, raws);
                }
            }
        }
    }

    /// Justice, of a sort: some poor soul answers for the shortfall.
    fn punish_for_mandate(&mut self, baron: usize, m: Mandate, raws: &Raws) {
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
                m.describe(raws)
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
            // A tile hardening to obsidian is another way a surface square turns
            // solid — never leave a tree or shrub sealed inside it (mirrors the
            // wall-raise cleanup).
            self.trees.remove(&p);
            self.shrubs.remove(&p);
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
                    quality: 0,
                    made_at: 0,
            variant: 0,
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
                quality: 0,
                made_at: 0,
            variant: 0,
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
            // A container leaves with everything in it, so its contents must be
            // free to go too. Without this, a barrel could be sold out from
            // under the dwarf already walking across the fort to eat from it —
            // the same claim the check above protects a loose loaf with.
            if is_container(it.kind)
                && self
                    .contents_of(i)
                    .iter()
                    .any(|&c| self.items[c].reserved_by.is_some())
            {
                return Err(format!(
                    "someone is already coming for what's in that {:?}",
                    it.kind
                ));
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
        // A container is offered with its contents, so it is priced with them.
        let offered: u32 = offer.iter().map(|&i| self.stack_value(i, raws)).sum();
        let asked: u32 = request
            .iter()
            .map(|&g| item_value(&caravan.goods[g], raws))
            .sum();
        // What the merchants must be paid: the asked value plus their margin
        // (they came a long way). Standing goodwill counts toward it, so a fort
        // that over-paid last season can draw the balance down now — even buy
        // outright with no goods offered.
        let margin = raws.economy.trade_margin;
        let required = (asked as f32 * margin).ceil() as i64;
        let available = offered as i64 + self.trade_credit;
        if available < required {
            let short = required - self.trade_credit;
            return Err(if self.trade_credit > 0 {
                format!(
                    "the merchants want {short} more in goods (you offered {offered}; \
                     {} credit applied)",
                    self.trade_credit
                )
            } else {
                format!("the merchants scoff: they ask {required} in goods (you offered {offered})")
            });
        }

        // Defying the baron's export ban: selling a good of the forbidden
        // material is allowed, but noted — the baron answers it when the edict
        // lapses (see tick_nobility). Checked before the goods leave.
        if let Some(target) = self
            .mandate
            .filter(|m| m.kind == MandateKind::ExportBan)
            .map(|m| m.target)
        {
            let banned_kind = |k: ItemKind| {
                matches!(
                    k,
                    ItemKind::Boulder
                        | ItemKind::Bar
                        | ItemKind::Craft
                        | ItemKind::Weapon
                        | ItemKind::Armor
                        | ItemKind::Statue
                )
            };
            let defied = offer
                .iter()
                .filter_map(|&i| self.items.get(i))
                .any(|it| it.stuff == target && banned_kind(it.kind));
            if defied {
                if let Some(m) = &mut self.mandate {
                    m.violated = true;
                }
            }
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
            // The wagon takes the barrel and the wine in it.
            self.consume_with_contents(i);
        }
        // Remove bought goods from the wagon (descending order keeps indices valid).
        let mut bought: Vec<usize> = request.to_vec();
        bought.sort_unstable_by(|a, b| b.cmp(a));
        let caravan = self.caravan.as_mut().unwrap();
        let mut received = Vec::new();
        for g in bought {
            received.push(caravan.goods.remove(g));
        }
        let bought_at = self.clock.tick;
        for mut it in received {
            it.pos = drop_at;
            it.state = ItemState::OnGround;
            // Provisions off the wagon are as fresh as the day you bought
            // them. A caravan's own clock means nothing in fort time, and
            // without this a purchased meal is born already a month old and
            // rots at the next dawn.
            it.made_at = bought_at;
            self.items.push(it);
        }
        // Settle the goodwill: whatever the fort paid over (or under) the
        // required amount rolls into the standing balance. `available >=
        // required` was just checked, so this can never go negative — the fort
        // never ends a trade owing the caravan.
        let old_credit = self.trade_credit;
        self.trade_credit = available - required;
        self.stats.trades_completed += 1;
        self.stats.value_exported += offered as u64;
        self.stats.value_imported += asked as u64;
        let delta = self.trade_credit - old_credit;
        let credit_note = if delta > 0 {
            format!(" (+{delta} credit banked, {} total)", self.trade_credit)
        } else if delta < 0 {
            format!(" ({} credit drawn, {} left)", -delta, self.trade_credit)
        } else {
            String::new()
        };
        self.log_event(format!(
            "Trade completed: {offered} in goods for {asked} received.{credit_note}"
        ));
        Ok(())
    }

    /// Visitors mill about near where they stand; no jobs, no needs (they
    /// carry their own provisions), but they will defend themselves.
    fn update_visitor(&mut self, i: usize, raws: &Raws) {
        self.tick_vitals(i);
        if !self.dwarves[i].alive {
            return;
        }
        if let Some(enemy) = self.adjacent_enemy(i) {
            self.melee(i, enemy, raws);
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
        let food = self.count_kind(ItemKind::Meal)
            + self.count_kind(ItemKind::Crop)
            + self.count_kind(ItemKind::Berry);
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
        // Wealth attracts settlers the way it attracts raiders: a prosperous
        // fort draws bigger waves. This is an ADDITIVE pull on top of the base
        // draw, never a floor — the food/drink guard above is the only thing
        // that can turn migrants away, so a broke-but-fed fort still grows.
        //
        // The base `gen_range` draw is taken first and unchanged, so the rng
        // stream is untouched by wealth. The bonus itself is gated behind
        // `invasions`: every headless test runs with invasions off, so migrant
        // counts there stay byte-identical (more migrants would mean more
        // `new_dwarf` draws and a diverged stream — the determinism landmine).
        let base = self.rng.gen_range(1..=3usize);
        let pull = if self.invasions {
            (self.cached_wealth / WEALTH_PER_MIGRANT) as usize
        } else {
            0
        };
        let count = (base + pull).min(POP_CAP - alive);
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
        let mut pending_weapons = 0usize;
        let mut pending_glass = 0usize;
        let mut pending_bars = 0usize;
        let mut pending_armor = 0usize;
        let mut pending_shields = 0usize;
        let mut pending_crossbows = 0usize;
        let mut pending_bolts = 0usize;
        let mut pending_furniture = 0usize;
        let mut pending_clothes = 0usize;
        let mut pending_barrels = 0usize;
        let mut pending_bins = 0usize;
        let mut pending_statues = 0usize;
        let mut pending_instruments = 0usize;
        let mut pending_leather = 0usize;
        let mut pending_wood_beds = 0usize;
        let mut pending_wood_statues = 0usize;
        for d in &self.dwarves {
            if d.alive {
                match d.task {
                    Task::Craft { kind: CraftKind::Brew, .. } => pending_brews += 1,
                    Task::Craft { kind: CraftKind::Cook, .. } => pending_cooks += 1,
                    Task::Craft { kind: CraftKind::Stonecraft, .. } => pending_crafts += 1,
                    Task::Craft { kind: CraftKind::BoneCraft, .. } => pending_crafts += 1,
                    Task::Craft { kind: CraftKind::ForgeWeapon, .. } => pending_weapons += 1,
                    Task::Craft { kind: CraftKind::ForgeCrossbow, .. } => pending_crossbows += 1,
                    Task::Craft { kind: CraftKind::ForgeBolts, .. } => pending_bolts += 1,
                    Task::Craft { kind: CraftKind::MakeGlass, .. } => pending_glass += 1,
                    Task::Craft { kind: CraftKind::Smelt, .. } => pending_bars += 1,
                    Task::Craft { kind: CraftKind::ForgeArmor, .. } => pending_armor += 1,
                    Task::Craft { kind: CraftKind::ForgeShield, .. } => pending_shields += 1,
                    Task::Craft { kind: CraftKind::MakeFurniture, .. } => pending_furniture += 1,
                    Task::Craft { kind: CraftKind::SewClothes, .. } => pending_clothes += 1,
                    Task::Craft { kind: CraftKind::MakeBarrel, .. } => pending_barrels += 1,
                    Task::Craft { kind: CraftKind::MakeBin, .. } => pending_bins += 1,
                    Task::Craft { kind: CraftKind::CarveStatue, .. } => pending_statues += 1,
                    Task::Craft { kind: CraftKind::MakeInstrument, .. } => pending_instruments += 1,
                    Task::Craft { kind: CraftKind::TanHide, .. } => pending_leather += 1,
                    Task::Craft { kind: CraftKind::MakeWoodBed, .. } => pending_wood_beds += 1,
                    Task::Craft { kind: CraftKind::CarveWoodStatue, .. } => pending_wood_statues += 1,
                    _ => {}
                }
            }
        }
        let has_craftsdwarf = self
            .buildings
            .iter()
            .any(|b| b.kind == BuildingKind::Craftsdwarf);
        let has_forge = self.buildings.iter().any(|b| b.kind == BuildingKind::Forge);
        let has_smelter = self.buildings.iter().any(|b| b.kind == BuildingKind::Smelter);
        let has_mason = self.buildings.iter().any(|b| b.kind == BuildingKind::Mason);
        let has_clothier = self.buildings.iter().any(|b| b.kind == BuildingKind::Clothier);
        let has_carpenter = self.buildings.iter().any(|b| b.kind == BuildingKind::Carpenter);
        let has_tanner = self.buildings.iter().any(|b| b.kind == BuildingKind::Tanner);
        let has_glassworks = self
            .buildings
            .iter()
            .any(|b| b.kind == BuildingKind::GlassFurnace);
        let soldiers = self
            .dwarves
            .iter()
            .filter(|d| d.alive && d.faction == Faction::Fort && d.soldier)
            .count();
        // The forge arms melee soldiers and marksdwarves from separate racks:
        // blades for the one, crossbows and bolts for the other.
        let marksdwarves = self.marksdwarf_count();
        let melee_soldiers = soldiers.saturating_sub(marksdwarves);
        let crossbows_on_hand = self
            .items
            .iter()
            .filter(|it| {
                it.active()
                    && it.kind == ItemKind::Weapon
                    && it.weapon_variant().is_some_and(|v| raws.weapons.is_ranged(v))
            })
            .count();
        let melee_weapons_on_hand = self
            .items
            .iter()
            .filter(|it| {
                it.active()
                    && it.kind == ItemKind::Weapon
                    && it.weapon_variant().is_some_and(|v| !raws.weapons.is_ranged(v))
            })
            .count();
        for i in 0..self.dwarves.len() {
            if self.player == Some(i) || self.dwarves[i].follower {
                // The player and their sworn companions follow no job board —
                // and handing a follower a fort job would leak its reservation
                // (follow_hero overwrites the task without abandon_task).
                continue;
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
                // Bone trinkets need no stone — any skeletonized part on hand is
                // free trade stock. Carve them whenever the craft shelf has room.
                let want_bone_crafts = has_craftsdwarf
                    && crafts + pending_crafts < 20
                    && self.items.iter().any(|it| {
                        it.kind == ItemKind::BodyPart && it.stuff >= 2 && self.item_takeable(it)
                    });
                // Arm and armor the soldiers: the forge works smelted bars into
                // weapons and plate until every enlistee has both. Bars feed both
                // lines, so each checks there's a bar free of the other's claims.
                let bars = self.count_kind(ItemKind::Bar);
                let armor_on_hand = self.count_kind(ItemKind::Armor);
                let claimed_bars =
                    pending_weapons + pending_armor + pending_crossbows + pending_bolts;
                // A blade for every melee soldier; a crossbow for every
                // marksdwarf; a suit of plate for all.
                let want_weapons = has_forge
                    && bars > claimed_bars
                    && melee_weapons_on_hand + pending_weapons < melee_soldiers;
                let want_crossbows = has_forge
                    && bars > claimed_bars
                    && crossbows_on_hand + pending_crossbows < marksdwarves;
                // Keep the quivers full: a marksdwarf wants a good forty bolts
                // behind them, forged a quiver at a time.
                let bolt_stock = self.bolts as usize + pending_bolts * QUIVER as usize;
                let want_bolts = has_forge
                    && marksdwarves > 0
                    && bars > claimed_bars
                    && bolt_stock < marksdwarves * 40;
                let want_armor = has_forge
                    && bars > claimed_bars
                    && armor_on_hand + pending_armor < soldiers;
                // A shield for every soldier too — the arm that turns a blow
                // aside is worth as much as the plate that softens it.
                let shields_on_hand = self.count_kind(ItemKind::Shield);
                let want_shields = has_forge
                    && bars > claimed_bars + pending_shields
                    && shields_on_hand + pending_shields < soldiers;
                // Smelt ore down into bars — the forge's stock — while stone is
                // surplus, keeping enough to arm AND armor every soldier plus a
                // few bars over to trade.
                let want_bars = has_smelter
                    && boulders > 4 + pending_crafts + pending_bars
                    && bars + pending_bars < soldiers * 2 + 4;
                // Blow glass — the finest trade good — when stone is plentiful.
                let glass = self.count_kind(ItemKind::Glass);
                let want_glass = has_glassworks
                    && boulders > 4 + pending_crafts + pending_glass
                    && glass + pending_glass < 20;
                // Furnish the fort: build beds from surplus stone until there's
                // one for every citizen (plus a couple over to trade). Count
                // in-flight WOOD beds too, so the mason and carpenter share one
                // target instead of overshooting it together.
                let beds = self.count_kind(ItemKind::Bed);
                let want_furniture = has_mason
                    && boulders > 4 + pending_crafts + pending_furniture
                    && beds + pending_furniture + pending_wood_beds < alive + 2;
                // Adorn the halls: carve a few statues from surplus stone. A
                // handful beautifies the whole fort, so the target is small.
                // Count in-flight wood statues too (same shared-target reason).
                let statues = self.count_kind(ItemKind::Statue);
                let want_statues = has_mason
                    && boulders > 4 + pending_crafts + pending_furniture + pending_statues
                    && statues + pending_statues + pending_wood_statues < 3;
                // Sew clothes from any cloth on hand until the fort is dressed
                // (plus a couple of sets over to trade).
                let cloth = self.count_kind(ItemKind::Cloth);
                let clothes = self.count_kind(ItemKind::Clothes);
                let want_clothes = has_clothier
                    && cloth > pending_clothes
                    && clothes + pending_clothes < alive + 2;
                // Logs are the carpenter's one stock, shared across every wooden
                // good: barrels, instruments, wooden beds, and wooden statues.
                let logs = self.count_kind(ItemKind::Log);
                let log_jobs = pending_barrels
                    + pending_bins
                    + pending_instruments
                    + pending_wood_beds
                    + pending_wood_statues;
                // Barrels and bins are the fort's storage, not ornaments: it
                // works another whenever something has nowhere to go (see
                // `wants_another_container`), not a fixed few per head. One at
                // a time — the next job is only wanted once this one has filled.
                let want_barrels = has_carpenter
                    && logs > log_jobs
                    && pending_barrels == 0
                    && self.wants_another_container(ItemKind::Barrel);
                let want_bins = has_carpenter
                    && logs > log_jobs
                    && pending_bins == 0
                    && self.wants_another_container(ItemKind::Bin);
                // Craft an instrument or two so the fort can make music.
                let instruments = self.count_kind(ItemKind::Instrument);
                let want_instruments =
                    has_carpenter && logs > log_jobs && instruments + pending_instruments < 2;
                // Furnish the fort in wood too: a carpenter builds wooden beds
                // from logs, each in its own species. Shares the bed target with
                // the mason's stone beds so the two don't overshoot together.
                let want_wood_beds = has_carpenter
                    && logs > log_jobs
                    && beds + pending_furniture + pending_wood_beds < alive + 2;
                // And wooden art: carve the odd wooden statue from a spare log.
                let want_wood_statues = has_carpenter
                    && logs > log_jobs
                    && statues + pending_statues + pending_wood_statues < 3;
                // Tan any hides on hand into leather at the tanner's shop.
                let hides = self.count_kind(ItemKind::Hide);
                let want_leather = has_tanner && hides > pending_leather;
                let wants = Wants {
                    drinks: want_drinks,
                    meals: want_meals,
                    crafts: want_crafts,
                    bone_crafts: want_bone_crafts,
                    weapons: want_weapons,
                    crossbows: want_crossbows,
                    bolts: want_bolts,
                    glass: want_glass,
                    bars: want_bars,
                    armor: want_armor,
                    shields: want_shields,
                    furniture: want_furniture,
                    clothes: want_clothes,
                    barrels: want_barrels,
                    bins: want_bins,
                    statues: want_statues,
                    instruments: want_instruments,
                    leather: want_leather,
                    wood_beds: want_wood_beds,
                    wood_statues: want_wood_statues,
                };
                match self.assign_one(i, raws, wants) {
                    Some(CraftKind::Brew) => pending_brews += 1,
                    Some(CraftKind::Cook) => pending_cooks += 1,
                    Some(CraftKind::Stonecraft) => pending_crafts += 1,
                    Some(CraftKind::BoneCraft) => pending_crafts += 1,
                    Some(CraftKind::ForgeWeapon) => pending_weapons += 1,
                    Some(CraftKind::MakeGlass) => pending_glass += 1,
                    Some(CraftKind::Smelt) => pending_bars += 1,
                    Some(CraftKind::ForgeArmor) => pending_armor += 1,
                    Some(CraftKind::ForgeShield) => pending_shields += 1,
                    Some(CraftKind::ForgeCrossbow) => pending_crossbows += 1,
                    Some(CraftKind::ForgeBolts) => pending_bolts += 1,
                    Some(CraftKind::MakeFurniture) => pending_furniture += 1,
                    Some(CraftKind::SewClothes) => pending_clothes += 1,
                    Some(CraftKind::MakeBarrel) => pending_barrels += 1,
                    Some(CraftKind::MakeBin) => pending_bins += 1,
                    Some(CraftKind::CarveStatue) => pending_statues += 1,
                    Some(CraftKind::MakeInstrument) => pending_instruments += 1,
                    Some(CraftKind::TanHide) => pending_leather += 1,
                    Some(CraftKind::MakeWoodBed) => pending_wood_beds += 1,
                    Some(CraftKind::CarveWoodStatue) => pending_wood_statues += 1,
                    Some(CraftKind::Weave) | Some(CraftKind::CutGem) | None => {}
                }
            }
        }
    }

    fn assign_one(&mut self, i: usize, raws: &Raws, w: Wants) -> Option<CraftKind> {
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
            // No brewed drink within reach — draw water from a well if one
            // stands in this region. (A pure fallback; if unreachable it simply
            // falls through, exactly as a fort without a well would.)
            let well = self
                .buildings
                .iter()
                .filter(|b| b.kind == BuildingKind::Well && self.regions.id(b.pos) == my_region)
                .map(|b| b.pos)
                .min_by_key(|p| p.manhattan(dwarf_pos));
            if let Some(well) = well {
                if let Some(p) = path::astar(&self.map, dwarf_pos, well, MAX_ASTAR_NODES) {
                    self.dwarves[i].task = Task::DrinkWell { spot: well, path: p };
                    return None;
                }
            }
        }
        // The wounded seek the hospital FIRST — a bleeding dwarf must mend
        // before it goes to drink away its troubles (bleeding, or a part below
        // half health — not every scratch).
        let hurt = self.dwarves[i]
            .body
            .iter()
            .any(|p| p.bleeding > 0 || p.hp * 2 < p.max_hp);
        if hurt && !self.hospitals.is_empty() {
            if let Some(spot) = self
                .hospitals
                .iter()
                .flat_map(|h| h.cells())
                .filter(|&c| self.regions.id(c) == my_region && self.map.walkable(c))
                .min_by_key(|&c| c.manhattan(dwarf_pos))
            {
                if let Some(p) = path::astar(&self.map, dwarf_pos, spot, MAX_ASTAR_NODES) {
                    self.dwarves[i].task = Task::Recover { spot, path: p, remaining: REST_TICKS };
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
        // Between battles, an idle soldier drills at the barracks — but only
        // when no enemy walks the fort (then they fight instead).
        if self.dwarves[i].soldier
            && !self.barracks.is_empty()
            && self.alive_hostiles() == 0
        {
            if let Some(spot) = self
                .barracks
                .iter()
                .flat_map(|b| b.cells())
                .filter(|&c| self.regions.id(c) == my_region && self.map.walkable(c))
                .min_by_key(|&c| c.manhattan(dwarf_pos))
            {
                if let Some(p) = path::astar(&self.map, dwarf_pos, spot, MAX_ASTAR_NODES) {
                    self.dwarves[i].task = Task::Spar { spot, path: p, remaining: SPAR_TICKS };
                    return None;
                }
            }
        }
        // The devout seek the temple when their worship is overdue.
        let overdue = self.clock.tick.saturating_sub(self.dwarves[i].last_prayer)
            >= PRAYER_INTERVAL;
        if overdue && !self.temples.is_empty() {
            if let Some(spot) = self
                .temples
                .iter()
                .flat_map(|t| t.cells())
                .filter(|&c| self.regions.id(c) == my_region && self.map.walkable(c))
                .min_by_key(|&c| c.manhattan(dwarf_pos))
            {
                if let Some(p) = path::astar(&self.map, dwarf_pos, spot, MAX_ASTAR_NODES) {
                    self.dwarves[i].task = Task::Pray { spot, path: p, remaining: PRAY_TICKS };
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
            Train { animal: usize },
            Build { site: Pos, input: usize },
            Fish { spot: Pos },
            Chop { tree: Pos },
            Gather { shrub: Pos },
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
            // Chopping and foraging: the worker stands on the (passable) tree
            // or shrub tile itself, rather than working from an adjacent square.
            if des.kind == DesignationKind::Chop || des.kind == DesignationKind::Gather {
                if self.regions.id(target) == my_region {
                    let cand = if des.kind == DesignationKind::Chop {
                        Cand::Chop { tree: target }
                    } else {
                        Cand::Gather { shrub: target }
                    };
                    consider(target.manhattan(dwarf_pos), cand, &mut best);
                }
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
        // Brewing needs an empty barrel for the drink to live in. Without one
        // the still stands idle, however much barley the fort has — the famous
        // Dwarf Fortress bind, and the reason a fort keeps a carpenter.
        if w.drinks && self.empty_barrel(dwarf_pos, my_region).is_some() {
            if let Some((shop, input)) =
                self.craft_pair(BuildingKind::Still, true, dwarf_pos, my_region, raws)
            {
                let d = self.items[input].pos.manhattan(dwarf_pos);
                consider(d, Cand::Craft { shop, input, kind: CraftKind::Brew }, &mut best);
            }
        }
        if w.meals {
            if let Some((shop, input)) =
                self.craft_pair(BuildingKind::Kitchen, false, dwarf_pos, my_region, raws)
            {
                let d = self.items[input].pos.manhattan(dwarf_pos);
                consider(d, Cand::Craft { shop, input, kind: CraftKind::Cook }, &mut best);
            }
        }
        // Stone crafts for trade — only when boulders are plentiful, so the
        // fort keeps stone for building.
        if w.crafts {
            if let Some((shop, input)) = self.craft_stone_pair(dwarf_pos, my_region) {
                let d = self.items[input].pos.manhattan(dwarf_pos);
                consider(d, Cand::Craft { shop, input, kind: CraftKind::Stonecraft }, &mut best);
            }
        }
        // Bone trinkets: the battlefield's leavings, once skeletonized, become a
        // renewable trade good rather than clutter.
        if w.bone_crafts {
            if let Some((shop, input)) = self.craft_bone_pair(dwarf_pos, my_region) {
                let d = self.items[input].pos.manhattan(dwarf_pos);
                consider(d, Cand::Craft { shop, input, kind: CraftKind::BoneCraft }, &mut best);
            }
        }
        // Weaving: any wool on hand becomes cloth at the loom.
        if let Some((shop, input)) = self.craft_wool_pair(dwarf_pos, my_region) {
            let d = self.items[input].pos.manhattan(dwarf_pos);
            consider(d, Cand::Craft { shop, input, kind: CraftKind::Weave }, &mut best);
        }
        // Gem cutting: any rough gem becomes a brilliant one at the jeweler.
        if let Some((shop, input)) = self.craft_gem_pair(dwarf_pos, my_region) {
            let d = self.items[input].pos.manhattan(dwarf_pos);
            consider(d, Cand::Craft { shop, input, kind: CraftKind::CutGem }, &mut best);
        }
        // Smelting: melt an ore boulder down into a refined metal bar.
        if w.bars {
            if let Some((shop, input)) = self.craft_smelt_pair(dwarf_pos, my_region) {
                let d = self.items[input].pos.manhattan(dwarf_pos);
                consider(d, Cand::Craft { shop, input, kind: CraftKind::Smelt }, &mut best);
            }
        }
        // Weaponsmithing: forge a metal bar into a weapon to arm the soldiers.
        if w.weapons {
            if let Some((shop, input)) = self.craft_forge_pair(raws, dwarf_pos, my_region) {
                let d = self.items[input].pos.manhattan(dwarf_pos);
                consider(d, Cand::Craft { shop, input, kind: CraftKind::ForgeWeapon }, &mut best);
            }
        }
        // Armoring: forge a metal bar into plate to protect the soldiers. Shares
        // the forge and its bar stock with weaponsmithing.
        if w.armor {
            if let Some((shop, input)) = self.craft_forge_pair(raws, dwarf_pos, my_region) {
                let d = self.items[input].pos.manhattan(dwarf_pos);
                consider(d, Cand::Craft { shop, input, kind: CraftKind::ForgeArmor }, &mut best);
            }
        }
        if w.shields {
            if let Some((shop, input)) = self.craft_forge_pair(raws, dwarf_pos, my_region) {
                let d = self.items[input].pos.manhattan(dwarf_pos);
                consider(d, Cand::Craft { shop, input, kind: CraftKind::ForgeShield }, &mut best);
            }
        }
        // Crossbows for the marksdwarves, and the bolts they loose. Both draw a
        // bar at the forge like the other arms.
        if w.crossbows {
            if let Some((shop, input)) = self.craft_forge_pair(raws, dwarf_pos, my_region) {
                let d = self.items[input].pos.manhattan(dwarf_pos);
                consider(d, Cand::Craft { shop, input, kind: CraftKind::ForgeCrossbow }, &mut best);
            }
        }
        if w.bolts {
            if let Some((shop, input)) = self.craft_forge_pair(raws, dwarf_pos, my_region) {
                let d = self.items[input].pos.manhattan(dwarf_pos);
                consider(d, Cand::Craft { shop, input, kind: CraftKind::ForgeBolts }, &mut best);
            }
        }
        // Furniture: work a boulder into a bed at the mason's workshop.
        if w.furniture {
            if let Some((shop, input)) = self.craft_mason_pair(dwarf_pos, my_region) {
                let d = self.items[input].pos.manhattan(dwarf_pos);
                consider(d, Cand::Craft { shop, input, kind: CraftKind::MakeFurniture }, &mut best);
            }
        }
        // Statuary: carve a boulder into a statue at the mason's workshop.
        if w.statues {
            if let Some((shop, input)) = self.craft_mason_pair(dwarf_pos, my_region) {
                let d = self.items[input].pos.manhattan(dwarf_pos);
                consider(d, Cand::Craft { shop, input, kind: CraftKind::CarveStatue }, &mut best);
            }
        }
        // Tailoring: sew a bolt of cloth into clothes at the clothier's shop.
        if w.clothes {
            if let Some((shop, input)) = self.craft_clothier_pair(dwarf_pos, my_region) {
                let d = self.items[input].pos.manhattan(dwarf_pos);
                consider(d, Cand::Craft { shop, input, kind: CraftKind::SewClothes }, &mut best);
            }
        }
        // Carpentry: work a log into a barrel at the carpenter's shop.
        if w.barrels {
            if let Some((shop, input)) = self.craft_carpenter_pair(dwarf_pos, my_region) {
                let d = self.items[input].pos.manhattan(dwarf_pos);
                consider(d, Cand::Craft { shop, input, kind: CraftKind::MakeBarrel }, &mut best);
            }
        }
        // Bins come after barrels deliberately: both are worked from the same
        // log at the same bench, so they tie on distance, and `consider` keeps
        // the first of a tie. Food before goods.
        if w.bins {
            if let Some((shop, input)) = self.craft_carpenter_pair(dwarf_pos, my_region) {
                let d = self.items[input].pos.manhattan(dwarf_pos);
                consider(d, Cand::Craft { shop, input, kind: CraftKind::MakeBin }, &mut best);
            }
        }
        // Instrument-making: work a log into an instrument at the carpenter's shop.
        if w.instruments {
            if let Some((shop, input)) = self.craft_carpenter_pair(dwarf_pos, my_region) {
                let d = self.items[input].pos.manhattan(dwarf_pos);
                consider(d, Cand::Craft { shop, input, kind: CraftKind::MakeInstrument }, &mut best);
            }
        }
        // Wooden furniture: work a log into a bed at the carpenter's shop.
        if w.wood_beds {
            if let Some((shop, input)) = self.craft_carpenter_pair(dwarf_pos, my_region) {
                let d = self.items[input].pos.manhattan(dwarf_pos);
                consider(d, Cand::Craft { shop, input, kind: CraftKind::MakeWoodBed }, &mut best);
            }
        }
        // Wooden art: carve a log into a statue at the carpenter's shop.
        if w.wood_statues {
            if let Some((shop, input)) = self.craft_carpenter_pair(dwarf_pos, my_region) {
                let d = self.items[input].pos.manhattan(dwarf_pos);
                consider(d, Cand::Craft { shop, input, kind: CraftKind::CarveWoodStatue }, &mut best);
            }
        }
        // Tanning: tan a raw hide into leather at the tanner's shop.
        if w.leather {
            if let Some((shop, input)) = self.craft_tanner_pair(dwarf_pos, my_region) {
                let d = self.items[input].pos.manhattan(dwarf_pos);
                consider(d, Cand::Craft { shop, input, kind: CraftKind::TanHide }, &mut best);
            }
        }
        // Glassblowing: melt a boulder into fine glass at the furnace.
        if w.glass {
            if let Some((shop, input)) = self.craft_glass_pair(dwarf_pos, my_region) {
                let d = self.items[input].pos.manhattan(dwarf_pos);
                consider(d, Cand::Craft { shop, input, kind: CraftKind::MakeGlass }, &mut best);
            }
        }
        // Construction: haul a boulder to a planned wall and raise it.
        if let Some((site, input)) = self.build_pair(dwarf_pos, my_region) {
            let d = self.items[input].pos.manhattan(dwarf_pos);
            consider(d, Cand::Build { site, input }, &mut best);
        }
        // Fishing: when the larder runs low, cast a line from a fishery bank.
        if w.meals && !self.fisheries.is_empty() {
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

        // War training: dogs marked for training, reachable from here.
        for (idx, animal) in self.animals.iter().enumerate() {
            if !animal.alive || !animal.war_marked || animal.war || animal.reserved_by.is_some() {
                continue;
            }
            if self.regions.id(animal.pos) != my_region {
                continue;
            }
            consider(animal.pos.manhattan(dwarf_pos), Cand::Train { animal: idx }, &mut best);
        }

        // Hauling: corpses go to open tombs, loose goods to stockpiles — and
        // goods already resting on a stockpile floor get packed into a
        // container that will take them, which is how a larder that predates
        // its first barrel (or a fort loaded from an older save) ever gets put
        // away. A packed item is neither loose nor resting, so it is never
        // picked up again by this scan: the shuffling terminates.
        // Repacking an already-stored larder is only worth scanning for when
        // the fort actually owns a container to pack it into. A fort with none
        // (every headless test, and every fort before its first barrel) does
        // exactly the work it always did.
        let any_containers = self.items.iter().any(|it| {
            it.active() && is_container(it.kind) && matches!(it.state, ItemState::Stored { .. })
        });
        for (idx, item) in self.items.iter().enumerate() {
            if !item.active() || item.reserved_by.is_some() {
                continue;
            }
            let loose = item.state == ItemState::OnGround;
            let resting = matches!(item.state, ItemState::Stored { .. }) && any_containers;
            if !loose && !resting {
                continue;
            }
            if self.haul_retry.get(&idx).is_some_and(|&t| t > tick) {
                continue;
            }
            if self.regions.id(item.pos) != my_region {
                continue;
            }
            let dest = if item.kind == ItemKind::Corpse && loose {
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
                // A barrel or bin that will take this comes first — packing it
                // away costs the same walk and spends no floor.
                let into = self
                    .find_container_for(item.kind, item.pos, my_region)
                    .map(|c| self.items[c].pos);
                match into {
                    // An item already put away only moves to be packed; it must
                    // never be shuffled from one bare cell to another, or
                    // haulers would carry the larder in circles forever.
                    None if resting => None,
                    None => self.find_free_cell(item.kind, item.pos, my_region),
                    some => some,
                }
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
            Cand::Chop { tree } => {
                match path::astar(&self.map, dwarf_pos, tree, MAX_ASTAR_NODES) {
                    Some(p) => {
                        self.designations.get_mut(&tree).unwrap().assigned = true;
                        self.dwarves[i].task = Task::Chop { tree, path: p, progress: 0 };
                    }
                    None => {
                        self.designations.get_mut(&tree).unwrap().retry_at = tick + RETRY_DELAY;
                    }
                }
            }
            Cand::Gather { shrub } => {
                match path::astar(&self.map, dwarf_pos, shrub, MAX_ASTAR_NODES) {
                    Some(p) => {
                        self.designations.get_mut(&shrub).unwrap().assigned = true;
                        self.dwarves[i].task = Task::Gather { shrub, path: p, progress: 0 };
                    }
                    None => {
                        self.designations.get_mut(&shrub).unwrap().retry_at = tick + RETRY_DELAY;
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
            Cand::Train { animal } => {
                let animal_pos = self.animals[animal].pos;
                if let Some(p) = path::astar(&self.map, dwarf_pos, animal_pos, MAX_ASTAR_NODES) {
                    self.animals[animal].reserved_by = Some(i);
                    self.dwarves[i].task = Task::Train { animal, path: p, progress: 0 };
                }
            }
            Cand::Build { site, input } => {
                let input_pos = self.items[input].pos;
                if let Some(p) = path::astar(&self.map, dwarf_pos, input_pos, MAX_ASTAR_NODES) {
                    self.items[input].reserved_by = Some(i);
                    if let Some(a) = self.constructions.get_mut(&site) {
                        *a = true;
                    }
                    self.dwarves[i].task = Task::Build {
                        site,
                        input,
                        path: p,
                        stage: FetchStage::ToInput,
                        progress: 0,
                    };
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
        // Foraged berries are eaten straight from the heap, no kitchen needed.
        if let Some(berry) = self.nearest_kind(ItemKind::Berry, near, region) {
            return Some(berry);
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

    /// Nearest (craftsdwarf workshop, skeletonized bone) pair for carving bone
    /// trinkets. Only a fully skeletal part (rot stage 2) is dry, clean bone —
    /// fresh or rotting gore is left to finish decaying.
    fn craft_bone_pair(&self, near: Pos, region: u32) -> Option<(Pos, usize)> {
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
                it.kind == ItemKind::BodyPart
                    && it.stuff >= 2
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

    /// Nearest (clothier's shop, bolt of cloth) pair for sewing clothes.
    fn craft_clothier_pair(&self, near: Pos, region: u32) -> Option<(Pos, usize)> {
        let shop = self
            .buildings
            .iter()
            .filter(|b| b.kind == BuildingKind::Clothier && self.regions.id(b.pos) == region)
            .min_by_key(|b| b.pos.manhattan(near))?;
        let input = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, it)| {
                it.kind == ItemKind::Cloth
                    && self.item_takeable(it)
                    && self.regions.id(it.pos) == region
            })
            .min_by_key(|(_, it)| it.pos.manhattan(near))
            .map(|(i, _)| i)?;
        Some((shop.pos, input))
    }

    /// Nearest (tanner's shop, raw hide) pair for tanning leather.
    fn craft_tanner_pair(&self, near: Pos, region: u32) -> Option<(Pos, usize)> {
        let shop = self
            .buildings
            .iter()
            .filter(|b| b.kind == BuildingKind::Tanner && self.regions.id(b.pos) == region)
            .min_by_key(|b| b.pos.manhattan(near))?;
        let input = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, it)| {
                it.kind == ItemKind::Hide
                    && self.item_takeable(it)
                    && self.regions.id(it.pos) == region
            })
            .min_by_key(|(_, it)| it.pos.manhattan(near))
            .map(|(i, _)| i)?;
        Some((shop.pos, input))
    }

    /// Nearest (carpenter's shop, log) pair for working a barrel.
    fn craft_carpenter_pair(&self, near: Pos, region: u32) -> Option<(Pos, usize)> {
        let shop = self
            .buildings
            .iter()
            .filter(|b| b.kind == BuildingKind::Carpenter && self.regions.id(b.pos) == region)
            .min_by_key(|b| b.pos.manhattan(near))?;
        let input = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, it)| {
                it.kind == ItemKind::Log
                    && self.item_takeable(it)
                    && self.regions.id(it.pos) == region
            })
            .min_by_key(|(_, it)| it.pos.manhattan(near))
            .map(|(i, _)| i)?;
        Some((shop.pos, input))
    }

    /// Nearest (jeweler, rough gem) pair for cutting gems.
    fn craft_gem_pair(&self, near: Pos, region: u32) -> Option<(Pos, usize)> {
        let shop = self
            .buildings
            .iter()
            .filter(|b| b.kind == BuildingKind::Jeweler && self.regions.id(b.pos) == region)
            .min_by_key(|b| b.pos.manhattan(near))?;
        let input = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, it)| {
                it.kind == ItemKind::RoughGem
                    && self.item_takeable(it)
                    && self.regions.id(it.pos) == region
            })
            .min_by_key(|(_, it)| it.pos.manhattan(near))
            .map(|(i, _)| i)?;
        Some((shop.pos, input))
    }

    /// Nearest (forge, boulder) pair for forging a weapon.
    /// Nearest (forge, metal bar) pair for forging a weapon. Bars are the
    /// forge's stock now — raw stone must be smelted at a smelter first. Of the
    /// bars on hand the smith reaches for the HARDEST metal first (steel over
    /// iron over bronze), nearest as the tiebreak, so hard-won steel is spent on
    /// arms rather than left in the corner.
    fn craft_forge_pair(&self, raws: &Raws, near: Pos, region: u32) -> Option<(Pos, usize)> {
        let shop = self
            .buildings
            .iter()
            .filter(|b| b.kind == BuildingKind::Forge && self.regions.id(b.pos) == region)
            .min_by_key(|b| b.pos.manhattan(near))?;
        let input = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, it)| {
                it.kind == ItemKind::Bar
                    && self.item_takeable(it)
                    && self.regions.id(it.pos) == region
            })
            .min_by_key(|(_, it)| {
                let h = raws.materials.get(it.stuff).combat.hardness;
                (std::cmp::Reverse((h * 100.0) as i64), it.pos.manhattan(near))
            })
            .map(|(i, _)| i)?;
        Some((shop.pos, input))
    }

    /// Nearest takeable flux boulder in the region — the second ingredient of
    /// steel, consumed with iron ore at the smelter.
    fn nearest_flux_boulder(&self, raws: &Raws, near: Pos, region: u32) -> Option<usize> {
        self.items
            .iter()
            .enumerate()
            .filter(|(_, it)| {
                it.kind == ItemKind::Boulder
                    && self.item_takeable(it)
                    && self.regions.id(it.pos) == region
                    && raws.materials.get(it.stuff).is_flux
            })
            .min_by_key(|(_, it)| it.pos.manhattan(near))
            .map(|(i, _)| i)
    }

    /// Nearest (smelter, boulder) pair for smelting ore down into a metal bar.
    fn craft_smelt_pair(&self, near: Pos, region: u32) -> Option<(Pos, usize)> {
        let shop = self
            .buildings
            .iter()
            .filter(|b| b.kind == BuildingKind::Smelter && self.regions.id(b.pos) == region)
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

    /// Nearest (mason's workshop, boulder) pair for building a piece of furniture.
    fn craft_mason_pair(&self, near: Pos, region: u32) -> Option<(Pos, usize)> {
        let shop = self
            .buildings
            .iter()
            .filter(|b| b.kind == BuildingKind::Mason && self.regions.id(b.pos) == region)
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

    /// A tile a mason can stand on to raise the wall at `site`: adjacent,
    /// walkable, same region as `from`, and NEVER the site itself or another
    /// planned wall (else the builder walls itself in, or two adjacent plans
    /// deadlock standing on each other's sites).
    fn build_work_position(&self, site: Pos, from: Pos) -> Option<Pos> {
        let region = self.regions.id(from);
        let mut work = Vec::with_capacity(6);
        path::work_positions(&self.map, site, &mut work);
        work.into_iter()
            .filter(|&w| {
                w != site
                    && !self.constructions.contains_key(&w)
                    && self.regions.id(w) == region
            })
            .min_by_key(|w| w.manhattan(from))
    }

    /// Nearest (construction site, boulder) pair for raising a wall: an
    /// unclaimed plan reachable in this region, and a boulder to build it with.
    fn build_pair(&self, near: Pos, region: u32) -> Option<(Pos, usize)> {
        let site = self
            .constructions
            .iter()
            .filter(|(&p, &assigned)| !assigned && self.regions.id(p) == region)
            .min_by_key(|(&p, _)| p.manhattan(near))
            .map(|(&p, _)| p)?;
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
        Some((site, input))
    }

    /// Nearest (glass furnace, boulder) pair for blowing glass.
    fn craft_glass_pair(&self, near: Pos, region: u32) -> Option<(Pos, usize)> {
        let shop = self
            .buildings
            .iter()
            .filter(|b| b.kind == BuildingKind::GlassFurnace && self.regions.id(b.pos) == region)
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

    /// How many soldiers the fort has enlisted, and how many forged weapons
    /// are on hand — an armed soldier is one of the first `weapons` enlistees.
    pub fn armed_soldiers(&self) -> usize {
        let soldiers = self
            .dwarves
            .iter()
            .filter(|d| d.alive && d.faction == Faction::Fort && d.soldier)
            .count();
        let weapons = self
            .items
            .iter()
            .filter(|it| it.active() && it.kind == ItemKind::Weapon)
            .count();
        soldiers.min(weapons)
    }

/// How many soldiers are armored: the armory issues its forged suits to
    /// enlistees in index order, just like weapons.
    pub fn armored_soldiers(&self) -> usize {
        let soldiers = self
            .dwarves
            .iter()
            .filter(|d| d.alive && d.faction == Faction::Fort && d.soldier)
            .count();
        let armor = self
            .items
            .iter()
            .filter(|it| it.active() && it.kind == ItemKind::Armor)
            .count();
        soldiers.min(armor)
    }

/// A soldier's rank in the armoury's issue order — the fort hands out its
    /// weapons and armour to enlistees by dwarf index, one each.
    fn armory_rank(&self, i: usize) -> usize {
        self.dwarves
            .iter()
            .take(i)
            .filter(|d| d.alive && d.faction == Faction::Fort && d.soldier)
            .count()
    }

    /// A soldier's rank among the enlistees who share its uniform — so a
    /// marksdwarf draws the n-th crossbow from the rack of crossbows, and a
    /// melee soldier the n-th blade from the rack of blades, without the two
    /// racks fighting over the same index.
    fn armory_rank_in_uniform(&self, i: usize) -> usize {
        let mine = self.squad_uniform(i);
        self.dwarves
            .iter()
            .enumerate()
            .take(i)
            .filter(|(j, d)| {
                d.alive
                    && d.faction == Faction::Fort
                    && d.soldier
                    && self.squad_uniform(*j) == mine
            })
            .count()
    }

    /// The actual weapon in this fighter's hands, as an item index — a soldier's
    /// issued arm, or an adventurer's carried one. Combat reads its kind and
    /// material off the real item rather than settling for "armed: yes/no".
    fn wielded_weapon(&self, i: usize, raws: &Raws) -> Option<usize> {
        // An adventurer or the player carries their own blade.
        if let Some(idx) = self.items.iter().position(|it| {
            it.active()
                && it.kind == ItemKind::Weapon
                && it.state == ItemState::Carried { by: i }
        }) {
            return Some(idx);
        }
        // A soldier draws the rank-th weapon from the armoury — a crossbow if
        // its squad marches under the crossbow, a melee arm otherwise, each
        // ranked within its own kind so the racks don't collide.
        if !self.dwarves[i].soldier || !self.dwarves[i].alive {
            return None;
        }
        let want_ranged = self.squad_uniform(i) == Uniform::Ranged;
        let rank = self.armory_rank_in_uniform(i);
        self.items
            .iter()
            .enumerate()
            .filter(|(_, it)| {
                it.active()
                    && it.kind == ItemKind::Weapon
                    && it.weapon_variant().is_some_and(|v| raws.weapons.is_ranged(v) == want_ranged)
            })
            .nth(rank)
            .map(|(idx, _)| idx)
    }

    /// Does this fighter have a shield on their arm? Soldiers are issued them
    /// by the armoury's rank order, like everything else.
    fn wields_shield(&self, i: usize) -> bool {
        if !self.dwarves[i].soldier || !self.dwarves[i].alive {
            return false;
        }
        let rank = self.armory_rank(i);
        self.items
            .iter()
            .filter(|it| it.active() && it.kind == ItemKind::Shield)
            .nth(rank)
            .is_some()
    }

    /// Does the blow miss? Before any damage, a defender who can gets a chance
    /// to avoid it — Dwarf Fortress's three independent defences rolled in
    /// turn: dodge aside, block with a shield, or parry with one's own weapon.
    /// Returns the word for the log if the blow is turned, `None` if it lands.
    ///
    /// Skill is what separates a veteran from a recruit here: a raw dwarf
    /// scarcely defends, a seasoned one slips half the blows aimed at them.
    /// Beasts are too vast to dodge or parry, which is why they must be worn
    /// down. Draws RNG only when a defence is actually possible, so an
    /// undefended blow (a raw dwarf, a beast) shifts no stream it didn't before.
    fn try_defend(&mut self, defender: usize, raws: &Raws) -> Option<&'static str> {
        let d = &self.dwarves[defender];
        // The sleeping, the fallen, and the fleeing do not defend.
        if !d.alive || matches!(d.task, Task::Sleep { .. }) {
            return None;
        }
        let skill = d.skill_level(Skill::Fighting) as f32;
        let beast = d.beast;

        // Dodge: nimble on the feet, and it improves fast with training.
        let dodge = if beast { 0.0 } else { 0.03 + skill * 0.04 };
        if dodge > 0.0 && self.rng.gen_bool(dodge.min(0.9) as f64) {
            self.add_xp(defender, Skill::Fighting, 3);
            return Some("dodges");
        }
        // Block: a shield turns aside far more than bare skill can, and the
        // arm behind it only gets better.
        let block = if !beast && self.wields_shield(defender) {
            0.20 + skill * 0.04
        } else {
            0.0
        };
        if block > 0.0 && self.rng.gen_bool(block.min(0.9) as f64) {
            self.add_xp(defender, Skill::Fighting, 4);
            return Some("blocks the blow with a shield");
        }
        // Parry: an armed fighter turns a blow with their own weapon.
        let parry = if !beast && self.wielded_weapon(defender, raws).is_some() {
            skill * 0.03
        } else {
            0.0
        };
        if parry > 0.0 && self.rng.gen_bool(parry.min(0.9) as f64) {
            self.add_xp(defender, Skill::Fighting, 2);
            return Some("parries");
        }
        None
    }

    /// The suit of armour this fighter wears, as an item index. Only the fort's
    /// enlisted soldiers are issued armour, by the same rank order.
    fn worn_armor(&self, i: usize) -> Option<usize> {
        if !self.dwarves[i].soldier || !self.dwarves[i].alive {
            return None;
        }
        let rank = self.armory_rank(i);
        self.items
            .iter()
            .enumerate()
            .filter(|(_, it)| it.active() && it.kind == ItemKind::Armor)
            .nth(rank)
            .map(|(idx, _)| idx)
    }

    /// Whether citizen `i` has a bed to sleep in: the fort's beds are claimed by
    /// its citizens in index order, one each, just as the armory issues weapons
    /// and armor. A dwarf with a bed rests more soundly than one on bare stone.
    /// Is this dwarf asleep in their own bed? Not "does the fort own enough
    /// beds" — a bed you are not lying in warms nobody.
    fn sleeps_in_bed(&self, i: usize) -> bool {
        let Some(bed) = self.dwarves[i].bed else { return false };
        self.items
            .get(bed)
            .is_some_and(|b| b.active() && b.pos == self.dwarves[i].pos)
    }

    /// Hand out the fort's beds, one to a dwarf, nearest first. A bed standing
    /// in a bedroom is claimed ahead of one out in the open, so the rooms a
    /// player troubles to build are the ones that get slept in.
    ///
    /// Deterministic: fixed iteration order, no RNG. A fort with no beds does
    /// nothing here.
    fn tick_bedrooms(&mut self) {
        // First, give up beds that are no longer beds to their owner: sold,
        // burned, or walled off behind a cave-in. A claim nobody reaps is
        // worse than no claim at all — the owner is skipped here forever
        // (they "have" a bed) while sleeping on stone every night, and the
        // bed itself never returns to the pool.
        for i in 0..self.dwarves.len() {
            let Some(b) = self.dwarves[i].bed else { continue };
            let gone = !self.items.get(b).is_some_and(|it| it.active());
            let walled_off = !gone
                && self.regions.id(self.items[b].pos) != self.regions.id(self.dwarves[i].pos);
            if gone || walled_off {
                self.dwarves[i].bed = None;
            }
        }
        let free: Vec<usize> = self
            .items
            .iter()
            .enumerate()
            .filter(|(b, it)| {
                it.active()
                    && it.kind == ItemKind::Bed
                    && !self.dwarves.iter().any(|d| d.alive && d.bed == Some(*b))
            })
            .map(|(b, _)| b)
            .collect();
        if free.is_empty() {
            return;
        }
        for i in 0..self.dwarves.len() {
            let d = &self.dwarves[i];
            if !d.alive || d.faction != Faction::Fort || d.bed.is_some() {
                continue;
            }
            let taken: Vec<usize> =
                self.dwarves.iter().filter_map(|d| d.bed).collect();
            let pick = free
                .iter()
                .copied()
                .filter(|b| !taken.contains(b))
                .filter(|&b| self.regions.id(self.items[b].pos) == self.regions.id(d.pos))
                .min_by_key(|&b| {
                    // A bed in a bedroom first; then the nearest.
                    let p = self.items[b].pos;
                    (!self.bedroom_at(p), p.manhattan(d.pos))
                });
            if let Some(b) = pick {
                self.dwarves[i].bed = Some(b);
            }
        }

        // Last, let a dwarf move up. A player usually builds the beds and
        // designates the rooms afterwards — without this, everyone would be
        // stuck in whatever bed they grabbed on day one and the bedrooms would
        // stand empty forever, which is exactly the wrong lesson to teach.
        for i in 0..self.dwarves.len() {
            let d = &self.dwarves[i];
            if !d.alive || d.faction != Faction::Fort {
                continue;
            }
            let Some(current) = d.bed else { continue };
            if self.bedroom_at(self.items[current].pos) {
                continue; // already housed
            }
            let taken: Vec<usize> = self.dwarves.iter().filter_map(|d| d.bed).collect();
            let better = self
                .items
                .iter()
                .enumerate()
                .filter(|(b, it)| {
                    it.active()
                        && it.kind == ItemKind::Bed
                        && !taken.contains(b)
                        && self.bedroom_at(it.pos)
                        && self.regions.id(it.pos) == self.regions.id(d.pos)
                })
                .min_by_key(|(_, it)| it.pos.manhattan(d.pos))
                .map(|(b, _)| b);
            if let Some(b) = better {
                self.dwarves[i].bed = Some(b);
            }
        }
    }

    /// A dwarf's bed is theirs until they die or it does. Called when either
    /// happens, so a bed never stays claimed by a corpse.
    fn release_bed(&mut self, i: usize) {
        self.dwarves[i].bed = None;
    }

    /// Whether citizen `i` is dressed in the fort's sewn clothes — claimed by
    /// citizens in index order, one set each, like beds and the armory. A
    /// well-dressed dwarf frets a little less.
    fn wears_clothes(&self, i: usize) -> bool {
        if self.dwarves[i].faction != Faction::Fort || !self.dwarves[i].alive {
            return false;
        }
        let clothes = self
            .items
            .iter()
            .filter(|it| it.active() && it.kind == ItemKind::Clothes)
            .count();
        if clothes == 0 {
            return false;
        }
        let rank = self
            .dwarves
            .iter()
            .take(i)
            .filter(|d| d.alive && d.faction == Faction::Fort)
            .count();
        rank < clothes
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
            self.melee(i, enemy, raws);
            return;
        }

        // The alarm: civilians drop everything and flee to a burrow; when it
        // lifts they return to work. Soldiers ignore it (they hold the line).
        // A dwarf with a pressing hunger or thirst is let out to eat or drink
        // first, so a long-held alarm never quietly starves the fort.
        let famished = self.dwarves[i].hunger >= NEED_AT || self.dwarves[i].thirst >= NEED_AT;
        if self.alarm && !famished && !self.dwarves[i].soldier && !self.burrows.is_empty() {
            if !matches!(self.dwarves[i].task, Task::Shelter { .. }) {
                let here = self.dwarves[i].pos;
                let region = self.regions.id(here);
                if let Some(spot) = self
                    .burrows
                    .iter()
                    .flat_map(|b| b.cells())
                    .filter(|&c| self.regions.id(c) == region && self.map.walkable(c))
                    .min_by_key(|&c| c.manhattan(here))
                {
                    if let Some(p) = path::astar(&self.map, here, spot, MAX_ASTAR_NODES) {
                        self.abandon_task(i);
                        self.dwarves[i].task = Task::Shelter { spot, path: p };
                    }
                }
            }
            // fall through to walk/hold the Shelter task
        } else if matches!(self.dwarves[i].task, Task::Shelter { .. }) {
            // Released from the burrow: the alarm lifted, or a pressing need
            // (hunger/thirst) sends this one out to be fed. Back to Idle so the
            // job board (or the needs handler) picks them up; if still famished
            // under an active alarm, they'll re-shelter once fed.
            self.dwarves[i].task = Task::Idle { wander_cd: 3 };
        }

        // Soldiers hunt as their squad's standing order directs. A defending
        // squad marches on the nearest hostile anywhere in the fort; a
        // stationed one only strikes what comes near its post and otherwise
        // holds it; a training squad drills (below) and defends only itself.
        if self.dwarves[i].soldier {
            let my_pos = self.dwarves[i].pos;
            let my_region = self.regions.id(my_pos);
            let order = self.squad_order(i);
            // A soldier still holding a Station/Patrol task after its squad's
            // order changed away from it: release it back to normal duties, or
            // it would stay frozen on the old post/beat (never eating, drilling,
            // or defending) forever.
            let stale_post = match (&self.dwarves[i].task, order) {
                (Task::Station { .. }, SquadOrder::Station(_)) => false,
                (Task::Patrol { .. }, SquadOrder::Patrol(..)) => false,
                (Task::Station { .. } | Task::Patrol { .. }, _) => true,
                _ => false,
            };
            if stale_post {
                self.dwarves[i].task = Task::Idle { wander_cd: 3 };
            }
            // A stationed squad measures threats from its post, not from the
            // soldier — so the whole line reacts to a foe nearing the gate. A
            // patrol reacts to what strays near the soldier as it walks the beat.
            let watch = match order {
                SquadOrder::Station(p) => p,
                _ => my_pos,
            };
            let quarry = self
                .dwarves
                .iter()
                .enumerate()
                .filter(|(_, d)| {
                    d.alive
                        && d.faction == Faction::Hostile
                        && self.regions.id(d.pos) == my_region
                        && match order {
                            // Train: never go looking (self-defense is handled
                            // above by the adjacent-enemy check).
                            SquadOrder::Train => false,
                            // Station: only what strays within reach of the post.
                            SquadOrder::Station(p) => d.pos.manhattan(p) <= STATION_ENGAGE_RANGE,
                            // Patrol: only what strays within reach of the soldier
                            // as it walks the beat.
                            SquadOrder::Patrol(..) => {
                                d.pos.manhattan(my_pos) <= STATION_ENGAGE_RANGE
                            }
                            SquadOrder::Defend => true,
                        }
                })
                .min_by_key(|(_, d)| d.pos.manhattan(watch))
                .map(|(j, _)| j);
            // A stationed soldier with nothing to fight marches to its post and
            // holds there, rather than idling wherever it happened to be.
            if quarry.is_none() {
                if let SquadOrder::Station(post) = order {
                    // A stationed soldier still breaks for a pressing need —
                    // hunger, thirst, or exhaustion — so holding a post never
                    // quietly starves it. Drop to Idle and fall through to the
                    // needs/job handler, which feeds and rests it; it marches
                    // back to its post once the need is met.
                    let needs_break = self.dwarves[i].hunger >= NEED_AT
                        || self.dwarves[i].thirst >= NEED_AT
                        || self.dwarves[i].fatigue >= 100.0;
                    if needs_break {
                        if matches!(self.dwarves[i].task, Task::Station { .. }) {
                            self.dwarves[i].task = Task::Idle { wander_cd: 3 };
                        }
                    } else {
                        // Already holding near the post: stand guard, step no more.
                        if my_pos.manhattan(post) <= 1 || !self.map.walkable(post) {
                            self.dwarves[i].task =
                                Task::Station { spot: post, path: Vec::new() };
                            return;
                        }
                        // March to the post — reuse a cached route unless it
                        // points elsewhere (a fresh order) or has run out.
                        let mut path = match self.dwarves[i].task.clone() {
                            Task::Station { spot, path } if spot == post => path,
                            other => {
                                // Drop any stale pursuit or job before marching.
                                if !matches!(other, Task::Station { .. }) {
                                    self.abandon_task(i);
                                }
                                Vec::new()
                            }
                        };
                        if path.is_empty() {
                            path = path::astar(&self.map, my_pos, post, MAX_ASTAR_NODES)
                                .unwrap_or_default();
                        }
                        if !path.is_empty() && !self.step_along(i, &mut path) {
                            path.clear();
                        }
                        self.dwarves[i].task = Task::Station { spot: post, path };
                        return;
                    }
                } else if let SquadOrder::Patrol(a, b) = order {
                    // A patrolling soldier walks its beat between the two ends,
                    // flipping on arrival — unless a pressing need pulls it off
                    // to be fed (same starvation guard as Station).
                    let needs_break = self.dwarves[i].hunger >= NEED_AT
                        || self.dwarves[i].thirst >= NEED_AT
                        || self.dwarves[i].fatigue >= 100.0;
                    if needs_break {
                        if matches!(self.dwarves[i].task, Task::Patrol { .. }) {
                            self.dwarves[i].task = Task::Idle { wander_cd: 3 };
                        }
                    } else {
                        // Carry forward which end we're heading for and the
                        // cached route; a fresh order (or a task that isn't a
                        // patrol of THIS beat) starts toward the farther end so a
                        // soldier dropped mid-beat sweeps the whole line.
                        let (mut toward_b, mut path) = match self.dwarves[i].task.clone() {
                            Task::Patrol { a: pa, b: pb, toward_b, path } if pa == a && pb == b => {
                                (toward_b, path)
                            }
                            other => {
                                if !matches!(other, Task::Patrol { .. }) {
                                    self.abandon_task(i);
                                }
                                (my_pos.manhattan(a) >= my_pos.manhattan(b), Vec::new())
                            }
                        };
                        // Reached this end: flip, aim for the other, drop the
                        // spent route so a fresh one is charted below.
                        let target = if toward_b { b } else { a };
                        if my_pos.manhattan(target) <= 1 {
                            toward_b = !toward_b;
                            path.clear();
                        }
                        if path.is_empty() {
                            let goal = if toward_b { b } else { a };
                            path = path::astar(&self.map, my_pos, goal, MAX_ASTAR_NODES)
                                .unwrap_or_default();
                        }
                        if !path.is_empty() && !self.step_along(i, &mut path) {
                            path.clear();
                        }
                        self.dwarves[i].task = Task::Patrol { a, b, toward_b, path };
                        return;
                    }
                }
            }
            if let Some(q) = quarry {
                // A marksdwarf with a loaded crossbow and a clear shot looses a
                // bolt from where it stands rather than closing to melee. It
                // fires only inside its range and line of sight; otherwise it
                // falls through and advances until it has one. If the quiver is
                // dry, fire_bolt returns false and it closes in to bash instead.
                let marks = self.squad_uniform(i) == Uniform::Ranged
                    && self
                        .wielded_weapon(i, raws)
                        .and_then(|w| self.items[w].weapon_variant())
                        .is_some_and(|v| raws.weapons.is_ranged(v));
                if marks
                    && self.dwarves[q].pos.manhattan(my_pos) <= RANGED_RANGE
                    && self.map.clear_shot(my_pos, self.dwarves[q].pos)
                    && self.fire_bolt(i, q, raws)
                {
                    // Stand and shoot: hold position, keep the target.
                    self.dwarves[i].task = Task::Fight { target: q, path: Vec::new(), repath_cd: 0 };
                    return;
                }
                // Reuse the cached route unless we've retargeted or the
                // repath timer expired — a full A* every tick is wasteful
                // when a siege sends many soldiers at one distant beast.
                let (mut path, mut repath_cd) = match self.dwarves[i].task.clone() {
                    Task::Fight { target, path, repath_cd } if target == q => (path, repath_cd),
                    Task::Fight { .. } => (Vec::new(), 0),
                    _ => {
                        // Answering the call: drop the current job first.
                        self.abandon_task(i);
                        (Vec::new(), 0)
                    }
                };
                if repath_cd == 0 && path.is_empty() {
                    path = path::astar(&self.map, my_pos, self.dwarves[q].pos, MAX_ASTAR_NODES)
                        .unwrap_or_default();
                    repath_cd = 60;
                }
                repath_cd = repath_cd.saturating_sub(1);
                if !path.is_empty() {
                    // step_along handles the walk cooldown itself.
                    if !self.step_along(i, &mut path) {
                        path.clear();
                    }
                } else if self.dwarves[i].move_cd == 0 {
                    // No route (walls between us): press greedily.
                    let goal = self.dwarves[q].pos;
                    let mut opts = Vec::with_capacity(8);
                    path::neighbors(&self.map, my_pos, &mut opts);
                    if let Some(&next) = opts.iter().min_by_key(|c| c.manhattan(goal)) {
                        if next.manhattan(goal) < my_pos.manhattan(goal) {
                            self.dwarves[i].pos = next;
                            self.dwarves[i].move_cd = WALK_COOLDOWN;
                        }
                    }
                } else {
                    self.dwarves[i].move_cd -= 1;
                }
                self.dwarves[i].task = Task::Fight { target: q, path, repath_cd };
                return;
            }
        }

        // Stress boils over into an episode (never interrupts a mood, and
        // never while huddled in a burrow — a tantrum there would fight the
        // alarm retreat every tick; it can break out once the alarm lifts).
        if self.dwarves[i].stress >= 100.0
            && !matches!(
                self.dwarves[i].task,
                Task::Tantrum { .. }
                    | Task::Sulk { .. }
                    | Task::StrangeMood { .. }
                    | Task::Shelter { .. }
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
                    // A dwarf with a bed of their own walks to it. One without
                    // — or one who cannot reach theirs — drops where they
                    // stand and sleeps the worse for it.
                    let own = self.dwarves[i].bed.filter(|&b| self.items[b].active());
                    if let Some(bed) = own {
                        let bpos = self.items[bed].pos;
                        if self.dwarves[i].pos != bpos {
                            if let Some(p) =
                                path::astar(&self.map, self.dwarves[i].pos, bpos, MAX_ASTAR_NODES)
                            {
                                self.dwarves[i].task = Task::GoToBed { bed, path: p };
                                return;
                            }
                        }
                    }
                    let remaining = if self.sleeps_in_bed(i) { 800 } else { 1200 };
                    self.dwarves[i].task = Task::Sleep { remaining };
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
            Task::DineAt { item, mut path } => {
                if !self.items[item].active() {
                    self.abandon_task(i);
                    return;
                }
                if !path.is_empty() {
                    if self.step_along(i, &mut path) {
                        self.carry_item_along(i);
                        self.dwarves[i].task = Task::DineAt { item, path };
                    } else {
                        // The way to the hall closed: eat where you stand.
                        self.dwarves[i].task = Task::Eat { item, path: Vec::new() };
                    }
                    return;
                }
                // At table.
                let kind = self.items[item].kind;
                self.items[item].consumed = true;
                self.items[item].reserved_by = None;
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
                self.push_thought(i, ThoughtKind::DinedInHall);
                self.dwarves[i].task = Task::Idle { wander_cd: 5 };
            }
            Task::GoToBed { bed, mut path } => {
                // The bed may have been sold or burned while its owner walked.
                if !self.items[bed].active() {
                    self.dwarves[i].bed = None;
                    self.dwarves[i].task = Task::Sleep { remaining: 1200 };
                    return;
                }
                if !path.is_empty() {
                    if self.step_along(i, &mut path) {
                        self.dwarves[i].task = Task::GoToBed { bed, path };
                    } else {
                        // Can't get there from here: sleep on the stone.
                        self.dwarves[i].task = Task::Sleep { remaining: 1200 };
                    }
                } else {
                    self.dwarves[i].task = Task::Sleep { remaining: 800 };
                }
            }
            Task::Sleep { remaining } => {
                if remaining == 0 {
                    self.dwarves[i].fatigue = 0.0;
                    // How a dwarf slept is one of the cheapest things a fort
                    // can get right, and one of the first they complain about.
                    let thought = if !self.sleeps_in_bed(i) {
                        ThoughtKind::SleptOnFloor
                    } else if self.bedroom_at(self.dwarves[i].pos) {
                        ThoughtKind::SleptInOwnRoom
                    } else {
                        ThoughtKind::SleptInBed
                    };
                    self.push_thought(i, thought);
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
                    // Corpse delivered to an open tomb: a burial. (Guard active()
                    // so a corpse a necromancer consumed mid-haul is never buried.)
                    if self.items[item].active() && self.items[item].kind == ItemKind::Corpse {
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
                    // A container standing here that will take this swallows
                    // it — that was the point of the walk. Re-checked on
                    // arrival rather than trusted from when the job was
                    // claimed: a barrel can fill up while a hauler crosses
                    // the fort.
                    let into = self
                        .items
                        .iter()
                        .enumerate()
                        .find(|(c, it)| {
                            *c != item
                                && it.pos == here
                                && is_container(it.kind)
                                && self.container_accepts(*c, self.items[item].kind)
                                // ...and standing in a pile that wants this
                                // cargo, the same rule `find_container_for`
                                // routed by. The two must agree.
                                && self
                                    .stockpile_at(here)
                                    .is_some_and(|s| {
                                        self.stockpiles[s].takes(self.items[item].kind)
                                    })
                        })
                        .map(|(c, _)| c);
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
                    self.items[item].state = if let Some(container) = into {
                        ItemState::Inside { container }
                    } else if !taken {
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
                let it = &self.items[item];
                let (ok, kind) = (
                    it.active() && it.pos == self.dwarves[i].pos && it.reserved_by == Some(i),
                    it.kind,
                );
                if !ok {
                    self.abandon_task(i);
                    return;
                }
                // Food goes to the hall to be eaten in company — but a dwarf
                // on the edge of starving does not stand on ceremony, and
                // drink is had where it is found.
                if !drinking
                    && !self.dining.is_empty()
                    && !self.dining_at(self.dwarves[i].pos)
                    && self.dwarves[i].hunger < 85.0
                {
                    let here = self.dwarves[i].pos;
                    let seat = self
                        .dining
                        .iter()
                        .flat_map(|r| r.cells())
                        .filter(|&c| {
                            self.map.walkable(c) && self.regions.id(c) == self.regions.id(here)
                        })
                        .min_by_key(|&c| c.manhattan(here));
                    if let Some(seat) = seat {
                        if let Some(p) = path::astar(&self.map, here, seat, MAX_ASTAR_NODES) {
                            if self.take_item(i, item) {
                                self.dwarves[i].task = Task::DineAt { item, path: p };
                                return;
                            }
                        }
                    }
                }
                // Consume on the spot.
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
            Task::DrinkWell { spot, mut path } => {
                if !path.is_empty() {
                    if self.step_along(i, &mut path) {
                        self.dwarves[i].task = Task::DrinkWell { spot, path };
                    } else {
                        self.abandon_task(i);
                    }
                    return;
                }
                // The well may have been removed while the dwarf walked over.
                if self.building_at(spot).map(|b| b.kind) != Some(BuildingKind::Well) {
                    self.abandon_task(i);
                    return;
                }
                self.dwarves[i].thirst = 0.0;
                self.dwarves[i].dehydrated_since = None;
                self.push_thought(i, ThoughtKind::HadDrink);
                self.dwarves[i].task = Task::Idle { wander_cd: 3 };
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
            Task::Chop { tree, mut path, progress } => {
                if !path.is_empty() {
                    if self.step_along(i, &mut path) {
                        self.dwarves[i].task = Task::Chop { tree, path, progress };
                    } else {
                        self.abandon_task(i);
                    }
                    return;
                }
                // The tree may have been felled or cancelled while walking over.
                let Some(&species) = self.trees.get(&tree) else {
                    self.abandon_task(i);
                    return;
                };
                let speed = 1 + self.dwarves[i].skill_level(Skill::Mining) as u16 / 2;
                let progress = progress + speed;
                if progress < CHOP_WORK {
                    self.dwarves[i].task = Task::Chop { tree, path, progress };
                    return;
                }
                // Timber! The tree falls, leaving a log of its wood where it stood.
                self.trees.remove(&tree);
                self.designations.remove(&tree);
                self.spawn_item(ItemKind::Log, species, tree);
                self.stats.trees_felled += 1;
                self.add_xp(i, Skill::Mining, 25);
                self.push_thought(i, ThoughtKind::HarvestedCrop);
                self.dwarves[i].task = Task::Idle { wander_cd: 3 };
            }
            Task::Gather { shrub, mut path, progress } => {
                if !path.is_empty() {
                    if self.step_along(i, &mut path) {
                        self.dwarves[i].task = Task::Gather { shrub, path, progress };
                    } else {
                        self.abandon_task(i);
                    }
                    return;
                }
                // The shrub may have been foraged or cancelled while walking over.
                if !self.shrubs.contains(&shrub) {
                    self.abandon_task(i);
                    return;
                }
                let speed = 1 + self.dwarves[i].skill_level(Skill::Farming) as u16 / 2;
                let progress = progress + speed;
                if progress < GATHER_WORK {
                    self.dwarves[i].task = Task::Gather { shrub, path, progress };
                    return;
                }
                // The shrub is picked clean, leaving a heap of berries where it
                // stood. (A tended patch reseeds itself over time.)
                self.shrubs.remove(&shrub);
                self.designations.remove(&shrub);
                self.spawn_item(ItemKind::Berry, 0, shrub);
                self.stats.foraged += 1;
                self.add_xp(i, Skill::Farming, 20);
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
                            CraftKind::Stonecraft
                            | CraftKind::BoneCraft
                            | CraftKind::Weave
                            | CraftKind::CutGem
                            | CraftKind::ForgeWeapon
                            | CraftKind::MakeGlass
                            | CraftKind::Smelt
                            | CraftKind::ForgeArmor
                            | CraftKind::ForgeShield
                            | CraftKind::ForgeCrossbow
                            | CraftKind::ForgeBolts
                            | CraftKind::MakeFurniture
                            | CraftKind::SewClothes
                            | CraftKind::MakeBarrel
                            | CraftKind::MakeBin
                            | CraftKind::CarveStatue
                            | CraftKind::MakeInstrument
                            | CraftKind::TanHide
                            | CraftKind::MakeWoodBed
                            | CraftKind::CarveWoodStatue => Skill::Crafting,
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
                        // The maker's skill (0..=6) becomes the good's quality
                        // tier (0..=5): a master turns out finer, dearer work.
                        let q = (self.dwarves[i].skill_level(skill) as u8).min(5);
                        match kind {
                            CraftKind::Brew => {
                                // The drink goes home in a barrel, as it always
                                // does — never onto the floor. The barrel was
                                // there when the job was taken; if it filled or
                                // left in the meantime the brewing still stands
                                // (the plant is spent either way) and the
                                // hauler will find the drink a home.
                                let cask = self.empty_barrel(shop, self.regions.id(shop));
                                self.stats.drinks_brewed += BATCH as u32;
                                for _ in 0..BATCH {
                                    match cask {
                                        Some(c) => {
                                            let at = self.items[c].pos;
                                            self.spawn_item(ItemKind::Drink, stuff, at);
                                            let d = self.items.len() - 1;
                                            self.items[d].state =
                                                ItemState::Inside { container: c };
                                        }
                                        None => {
                                            self.spawn_item(ItemKind::Drink, stuff, shop);
                                        }
                                    }
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
                                    self.spawn_quality_item(ItemKind::Craft, stuff, shop, q);
                                }
                                self.push_thought(i, ThoughtKind::CookedMeal); // a job well done
                            }
                            CraftKind::BoneCraft => {
                                // A bone is smaller stock than a boulder: one
                                // trinket, not two. `stuff` was the rot stage,
                                // meaningless on the trinket, so it carries none.
                                self.stats.crafts_made += 1;
                                self.spawn_quality_item(ItemKind::BoneCraft, 0, shop, q);
                                self.push_thought(i, ThoughtKind::CookedMeal);
                            }
                            CraftKind::Weave => {
                                self.stats.cloth_woven += 1;
                                self.spawn_quality_item(ItemKind::Cloth, 0, shop, q);
                                self.push_thought(i, ThoughtKind::CookedMeal);
                            }
                            CraftKind::CutGem => {
                                // The gem's variety carries through the cut.
                                self.stats.gems_cut += 1;
                                self.spawn_quality_item(ItemKind::CutGem, stuff, shop, q);
                                self.push_thought(i, ThoughtKind::CookedMeal);
                            }
                            CraftKind::MakeGlass => {
                                self.stats.glass_blown += 1;
                                self.spawn_quality_item(ItemKind::Glass, stuff, shop, q);
                                self.push_thought(i, ThoughtKind::CookedMeal);
                            }
                            CraftKind::ForgeWeapon => {
                                // The bar's metal carries into the blade, and the
                                // smith works the fort a mix of arms rather than
                                // a rack of identical swords — a spear for reach,
                                // a hammer for armoured foes. Crossbows are their
                                // own job, so this cycles the five melee kinds.
                                self.stats.weapons_forged += 1;
                                let melee = raws.weapons.melee_indices();
                                let variant =
                                    melee[self.stats.weapons_forged as usize % melee.len()];
                                self.spawn_quality_item(ItemKind::Weapon, stuff, shop, q);
                                self.set_last_weapon(variant);
                                self.push_thought(i, ThoughtKind::CookedMeal);
                            }
                            CraftKind::ForgeCrossbow => {
                                // A crossbow for a marksdwarf — a Weapon item like
                                // any other, but of the ranged kind.
                                self.stats.crossbows_forged += 1;
                                self.spawn_quality_item(ItemKind::Weapon, stuff, shop, q);
                                self.set_last_weapon(raws.weapons.index_of("crossbow").unwrap_or(0));
                                self.push_thought(i, ThoughtKind::CookedMeal);
                            }
                            CraftKind::ForgeBolts => {
                                // A quiver's worth of bolts joins the fort's
                                // shared ammo stock.
                                self.stats.bolts_forged += QUIVER;
                                self.bolts += QUIVER;
                                self.push_thought(i, ThoughtKind::CookedMeal);
                            }
                            CraftKind::Smelt => {
                                // The boulder's material carries into the bar —
                                // unless it's iron ore and a flux stone is on
                                // hand, in which case the two are cooked down
                                // together into steel, the finest war-metal short
                                // of adamantine.
                                self.stats.bars_smelted += 1;
                                let mat = raws.materials.get(stuff);
                                let is_iron = mat.category == MaterialCategory::Ore
                                    && mat.combat.hardness >= 90.0;
                                let mut out = stuff;
                                if is_iron {
                                    let region = self.regions.id(shop);
                                    if let (Some(steel), Some(flux)) = (
                                        raws.materials.index_of("steel"),
                                        self.nearest_flux_boulder(raws, shop, region),
                                    ) {
                                        self.items[flux].consumed = true;
                                        self.items[flux].reserved_by = None;
                                        out = steel as u16;
                                        let name = self.dwarves[i].name.clone();
                                        self.log_event(format!(
                                            "{name} smelts iron and flux into steel."
                                        ));
                                    }
                                }
                                self.spawn_quality_item(ItemKind::Bar, out, shop, q);
                                self.push_thought(i, ThoughtKind::CookedMeal);
                            }
                            CraftKind::ForgeArmor => {
                                // The bar's metal carries into the plate.
                                self.stats.armor_forged += 1;
                                self.spawn_quality_item(ItemKind::Armor, stuff, shop, q);
                                self.push_thought(i, ThoughtKind::CookedMeal);
                            }
                            CraftKind::ForgeShield => {
                                self.stats.shields_forged += 1;
                                self.spawn_quality_item(ItemKind::Shield, stuff, shop, q);
                                self.push_thought(i, ThoughtKind::CookedMeal);
                            }
                            CraftKind::MakeFurniture => {
                                // The boulder's stone carries into the bed.
                                self.stats.furniture_made += 1;
                                self.spawn_quality_item(ItemKind::Bed, stuff, shop, q);
                                self.push_thought(i, ThoughtKind::CookedMeal);
                            }
                            CraftKind::SewClothes => {
                                // Cloth carries no material index; clothes are
                                // valued as fine goods on their own.
                                self.stats.clothes_sewn += 1;
                                self.spawn_quality_item(ItemKind::Clothes, 0, shop, q);
                                self.push_thought(i, ThoughtKind::CookedMeal);
                            }
                            CraftKind::MakeBarrel => {
                                // A log carries no material index; the barrel is
                                // valued as a fine wooden good on its own.
                                self.stats.barrels_made += 1;
                                self.spawn_quality_item(ItemKind::Barrel, 0, shop, q);
                                self.push_thought(i, ThoughtKind::CookedMeal);
                            }
                            CraftKind::MakeBin => {
                                // A log carries no material index; the bin is
                                // valued as a plain wooden good on its own.
                                self.stats.bins_made += 1;
                                self.spawn_quality_item(ItemKind::Bin, 0, shop, q);
                                self.push_thought(i, ThoughtKind::CookedMeal);
                            }
                            CraftKind::CarveStatue => {
                                // The boulder's stone carries into the statue.
                                self.stats.statues_carved += 1;
                                self.spawn_quality_item(ItemKind::Statue, stuff, shop, q);
                                self.push_thought(i, ThoughtKind::CookedMeal);
                            }
                            CraftKind::MakeInstrument => {
                                // A log carries no material index; the instrument
                                // is valued as a fine crafted good on its own.
                                self.stats.instruments_made += 1;
                                self.spawn_quality_item(ItemKind::Instrument, 0, shop, q);
                                self.push_thought(i, ThoughtKind::CookedMeal);
                            }
                            CraftKind::TanHide => {
                                // A hide carries no material index; leather is
                                // valued as a fine good on its own.
                                self.stats.leather_tanned += 1;
                                self.spawn_quality_item(ItemKind::Leather, 0, shop, q);
                                self.push_thought(i, ThoughtKind::CookedMeal);
                            }
                            CraftKind::MakeWoodBed => {
                                // The log's wood carries into the bed.
                                self.stats.furniture_made += 1;
                                self.spawn_quality_item(ItemKind::Bed, stuff, shop, q);
                                self.push_thought(i, ThoughtKind::CookedMeal);
                            }
                            CraftKind::CarveWoodStatue => {
                                // The log's wood carries into the statue.
                                self.stats.statues_carved += 1;
                                self.spawn_quality_item(ItemKind::Statue, stuff, shop, q);
                                self.push_thought(i, ThoughtKind::CookedMeal);
                            }
                        }
                        self.add_xp(i, skill, 30);
                        self.dwarves[i].task = Task::Idle { wander_cd: 3 };
                    }
                }
            }
            Task::Build { site, input, mut path, stage, progress } => {
                if !path.is_empty() {
                    if self.step_along(i, &mut path) {
                        self.dwarves[i].task = Task::Build { site, input, path, stage, progress };
                    } else {
                        self.abandon_task(i);
                    }
                    return;
                }
                // The plan may have been cancelled or the boulder lost.
                let valid = self.constructions.contains_key(&site)
                    && self.items.get(input).is_some_and(|it| it.active());
                if !valid {
                    self.abandon_task(i);
                    return;
                }
                match stage {
                    FetchStage::ToInput => {
                        if !self.take_item(i, input) {
                            self.abandon_task(i);
                            return;
                        }
                        // Head for a tile beside the site — never onto it, or
                        // the wall would rise around the builder.
                        let here = self.dwarves[i].pos;
                        let dest = self.build_work_position(site, here);
                        match dest.and_then(|d| path::astar(&self.map, here, d, MAX_ASTAR_NODES)) {
                            Some(p) => {
                                self.dwarves[i].task = Task::Build {
                                    site,
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
                        // If we've drifted from the site, walk back beside it.
                        let here = self.dwarves[i].pos;
                        if here.x.abs_diff(site.x) + here.y.abs_diff(site.y) != 1
                            || here.z != site.z
                        {
                            let dest = self.build_work_position(site, here);
                            match dest.and_then(|d| path::astar(&self.map, here, d, MAX_ASTAR_NODES))
                            {
                                Some(p) if !p.is_empty() => {
                                    self.dwarves[i].task =
                                        Task::Build { site, input, path: p, stage, progress };
                                }
                                _ => self.abandon_task(i),
                            }
                            return;
                        }
                        let progress = progress + 1;
                        if progress < BUILD_WORK {
                            self.dwarves[i].task =
                                Task::Build { site, input, path, stage, progress };
                            return;
                        }
                        // Never wall in a creature standing on the site: wait
                        // for it to clear — but give up after a while so a
                        // permanently blocked plan never locks the mason and
                        // their stone forever.
                        let occupied = self.dwarves.iter().any(|d| d.alive && d.pos == site)
                            || self.animals.iter().any(|a| a.alive && a.pos == site);
                        if occupied {
                            if progress > BUILD_WORK + 600 {
                                self.abandon_task(i);
                            } else {
                                self.dwarves[i].task =
                                    Task::Build { site, input, path, stage, progress };
                            }
                            return;
                        }
                        // Raise the wall from the carried stone.
                        let mat = self.items[input].stuff;
                        self.items[input].consumed = true;
                        self.items[input].reserved_by = None;
                        let tile = self.map.tile_at(site).unwrap();
                        self.map.set_at(
                            site,
                            Tile { material: mat, shape: TileShape::Solid, water: 0, magma: 0 },
                        );
                        self.displace_water(site, tile.water);
                        self.constructions.remove(&site);
                        // A shrub or tree can't reach a planned tile (guarded
                        // both ways), but never leave one sealed in the new wall.
                        self.shrubs.remove(&site);
                        self.trees.remove(&site);
                        self.regions.dirty = true;
                        self.map_changed = true;
                        self.water.wake(site);
                        self.magma.wake(site);
                        self.add_xp(i, Skill::Mining, 15);
                        let name = self.dwarves[i].name.clone();
                        self.log_event(format!("{name} raises a wall."));
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
                // Once a tanner stands, the hide is saved to be tanned. Gated on
                // the building so a fort without one butchers exactly as before.
                if self.buildings.iter().any(|b| b.kind == BuildingKind::Tanner) {
                    self.spawn_item(ItemKind::Hide, 0, apos);
                }
                self.stats.animals_butchered += 1;
                self.add_xp(i, Skill::Cooking, 15);
                self.log_event(format!(
                    "A {} is butchered — {meat} servings of meat.",
                    kind.name()
                ));
                self.dwarves[i].task = Task::Idle { wander_cd: 3 };
            }
            Task::Train { animal, mut path, progress } => {
                // The pupil may have died, been un-marked, or wandered off.
                let valid = self
                    .animals
                    .get(animal)
                    .is_some_and(|a| a.alive && a.war_marked && !a.war && a.reserved_by == Some(i));
                if !valid {
                    self.abandon_task(i);
                    return;
                }
                if !path.is_empty() {
                    if self.step_along(i, &mut path) {
                        self.dwarves[i].task = Task::Train { animal, path, progress };
                    } else {
                        self.abandon_task(i);
                    }
                    return;
                }
                // Arrived where the dog was. If it drifted, follow it.
                let here = self.dwarves[i].pos;
                let apos = self.animals[animal].pos;
                if here.manhattan(apos) > 1 {
                    match path::astar(&self.map, here, apos, MAX_ASTAR_NODES) {
                        Some(p) if !p.is_empty() => {
                            self.dwarves[i].task = Task::Train { animal, path: p, progress };
                        }
                        _ => self.abandon_task(i),
                    }
                    return;
                }
                let progress = progress + 1;
                if progress < TRAIN_WORK {
                    self.dwarves[i].task = Task::Train { animal, path, progress };
                    return;
                }
                // The dog is a trained guardian now.
                self.animals[animal].war = true;
                self.animals[animal].war_marked = false;
                self.animals[animal].reserved_by = None;
                self.animals[animal].hp = self.animals[animal].kind.war_hp();
                self.add_xp(i, Skill::Fighting, 10);
                self.log_event("A dog is trained for war — it will guard the fort.".to_string());
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
                    // A mug within reach of THIS tavern seat — not one across
                    // the map in some other tavern.
                    let drink = self.items.iter().position(|it| {
                        it.kind == ItemKind::Drink
                            && self.item_takeable(it)
                            && self.tavern_at(it.pos)
                            && it.pos.z == here.z
                            && it.pos.manhattan(here) <= 4
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
            Task::Pray { spot, mut path, remaining } => {
                if !path.is_empty() {
                    if self.step_along(i, &mut path) {
                        self.dwarves[i].task = Task::Pray { spot, path, remaining };
                    } else if self.temple_at(self.dwarves[i].pos) {
                        self.dwarves[i].task = Task::Pray { spot, path: Vec::new(), remaining };
                    } else {
                        self.dwarves[i].task = Task::Idle { wander_cd: 5 };
                    }
                    return;
                }
                // In the temple: worship quiets the heart.
                self.dwarves[i].stress = (self.dwarves[i].stress - 0.08).max(0.0);
                if remaining == 0 {
                    self.dwarves[i].last_prayer = self.clock.tick;
                    self.push_thought(i, ThoughtKind::PrayedAtTemple);
                    self.dwarves[i].task = Task::Idle { wander_cd: 10 };
                } else {
                    self.dwarves[i].task = Task::Pray { spot, path, remaining: remaining - 1 };
                }
            }
            Task::Recover { spot, mut path, remaining } => {
                if !path.is_empty() {
                    if self.step_along(i, &mut path) {
                        self.dwarves[i].task = Task::Recover { spot, path, remaining };
                    } else if self.hospital_at(self.dwarves[i].pos) {
                        self.dwarves[i].task = Task::Recover { spot, path: Vec::new(), remaining };
                    } else {
                        self.dwarves[i].task = Task::Idle { wander_cd: 5 };
                    }
                    return;
                }
                // Resting in the ward: healing is handled in tick_vitals, which
                // mends faster for anyone standing in a hospital. Leave once
                // whole again, or when the stay is up.
                if remaining == 0 || !self.dwarves[i].is_wounded() {
                    self.dwarves[i].task = Task::Idle { wander_cd: 10 };
                } else {
                    self.dwarves[i].task = Task::Recover { spot, path, remaining: remaining - 1 };
                }
            }
            Task::Spar { spot, mut path, remaining } => {
                if !path.is_empty() {
                    if self.step_along(i, &mut path) {
                        self.dwarves[i].task = Task::Spar { spot, path, remaining };
                    } else if self.barracks_at(self.dwarves[i].pos) {
                        self.dwarves[i].task = Task::Spar { spot, path: Vec::new(), remaining };
                    } else {
                        self.dwarves[i].task = Task::Idle { wander_cd: 5 };
                    }
                    return;
                }
                // Drilling: every session hones the soldier's prowess.
                if remaining == 0 {
                    self.add_xp(i, Skill::Fighting, 20);
                    self.dwarves[i].task = Task::Idle { wander_cd: 10 };
                } else {
                    self.dwarves[i].task = Task::Spar { spot, path, remaining: remaining - 1 };
                }
            }
            Task::Shelter { spot, mut path } => {
                if !path.is_empty() {
                    if self.step_along(i, &mut path) {
                        self.dwarves[i].task = Task::Shelter { spot, path };
                    } else if self.burrow_at(self.dwarves[i].pos) {
                        self.dwarves[i].task = Task::Shelter { spot, path: Vec::new() };
                    } else {
                        self.dwarves[i].task = Task::Idle { wander_cd: 5 };
                    }
                    return;
                }
                // Huddled safe in the burrow: hold until the alarm is lifted
                // (update_dwarf releases us back to Idle then).
                self.dwarves[i].task = Task::Shelter { spot, path };
            }
            Task::Station { spot, mut path } => {
                // March to the post, then hold. update_dwarf redirects to a
                // fight the moment a hostile strays within reach.
                if !path.is_empty() && self.step_along(i, &mut path) {
                    self.dwarves[i].task = Task::Station { spot, path };
                } else {
                    self.dwarves[i].task = Task::Station { spot, path: Vec::new() };
                }
            }
            Task::Patrol { a, b, toward_b, mut path } => {
                // Walk the beat. update_dwarf charts the route and flips the
                // ends; this just advances along it (and drops a spent route so
                // the next tick recharts).
                if !path.is_empty() && self.step_along(i, &mut path) {
                    self.dwarves[i].task = Task::Patrol { a, b, toward_b, path };
                } else {
                    self.dwarves[i].task = Task::Patrol { a, b, toward_b, path: Vec::new() };
                }
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
                        // An artifact is a masterwork by its very nature.
                        if let Some(it) = self.items.last_mut() {
                            it.quality = 5;
                        }
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
    /// A recruited companion in adventure mode: cut down any adjacent
    /// enemy, otherwise shadow the hero, keeping a step or two behind.
    fn follow_hero(&mut self, i: usize, raws: &Raws) {
        self.tick_vitals(i);
        if !self.dwarves[i].alive {
            return;
        }
        // Strike first if an enemy is in reach.
        if let Some(enemy) = self.adjacent_enemy(i) {
            self.melee(i, enemy, raws);
            return;
        }
        let Some(hero) = self.player else {
            self.dwarves[i].task = Task::Idle { wander_cd: 50 };
            return;
        };
        let my_pos = self.dwarves[i].pos;
        let hero_pos = self.dwarves[hero].pos;
        // Close ranks, but don't jostle the hero when already at their heel.
        if my_pos.manhattan(hero_pos) <= 2 {
            self.dwarves[i].task = Task::Idle { wander_cd: 0 };
            if self.dwarves[i].move_cd > 0 {
                self.dwarves[i].move_cd -= 1;
            }
            return;
        }
        let (mut path, mut repath_cd) = match self.dwarves[i].task.clone() {
            Task::Fight { target, path, repath_cd } if target == hero => (path, repath_cd),
            _ => (Vec::new(), 0),
        };
        if repath_cd == 0 && path.is_empty() {
            path = path::astar(&self.map, my_pos, hero_pos, 20_000).unwrap_or_default();
            repath_cd = 30; // repath often — the hero moves each turn
        }
        repath_cd = repath_cd.saturating_sub(1);
        if !path.is_empty() {
            if !self.step_along(i, &mut path) {
                path.clear();
            }
        } else if self.dwarves[i].move_cd == 0 {
            // No route: press greedily toward the hero.
            let mut opts = Vec::with_capacity(8);
            path::neighbors(&self.map, my_pos, &mut opts);
            if let Some(&next) = opts.iter().min_by_key(|q| q.manhattan(hero_pos)) {
                if next.manhattan(hero_pos) < my_pos.manhattan(hero_pos) {
                    self.dwarves[i].pos = next;
                    self.dwarves[i].move_cd = WALK_COOLDOWN;
                }
            }
        } else {
            self.dwarves[i].move_cd -= 1;
        }
        self.dwarves[i].task = Task::Fight { target: hero, path, repath_cd };
    }

    fn update_hostile(&mut self, i: usize, raws: &Raws) {
        self.tick_vitals(i);
        if !self.dwarves[i].alive {
            return;
        }
        if let Some(enemy) = self.adjacent_enemy(i) {
            self.melee(i, enemy, raws);
            return;
        }
        // A war dog barring the way is dealt with first.
        if let Some(dog) = self.adjacent_war_dog(i) {
            self.maul_dog(i, dog);
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

        // Stepping onto a weapon trap springs it — hidden blades bite deep.
        let now = self.dwarves[i].pos;
        if now != my_pos && self.trap_at(now) {
            self.spring_trap(i);
        }
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
    fn melee(&mut self, attacker: usize, defender: usize, raws: &Raws) {
        if self.dwarves[attacker].attack_cd > 0 {
            self.dwarves[attacker].attack_cd -= 1;
            return;
        }
        self.dwarves[attacker].attack_cd = ATTACK_COOLDOWN;

        // The defender's chance to turn the blow before it ever bites — dodge,
        // block, or parry. A skilled fighter with a shield lives where a raw
        // recruit is cut down.
        if let Some(defence) = self.try_defend(defender, raws) {
            // The attacker still learns from the exchange.
            self.add_xp(attacker, Skill::Fighting, 2);
            let att = self.dwarves[attacker].name.clone();
            let def = self.dwarves[defender].name.clone();
            self.log_event(format!("{def} {defence} from {att}."));
            return;
        }

        // Torso is the biggest target; head the deadliest. Rolled here, BEFORE
        // the force, to keep the rng draw order combat has always had.
        let part_kind = self.roll_part();
        // The force behind the swing: a beast's monstrous strength, or a
        // dwarf's arm sharpened by training.
        let base = if self.dwarves[attacker].beast {
            self.rng.gen_range(25..=55) as f32
        } else {
            self.rng.gen_range(8..=20) as f32
        };
        let force = base + fighting_bonus(self.dwarves[attacker].skill_level(Skill::Fighting)) as f32;

        // The weapon in hand: its kind (a sword cuts, a hammer crushes) and the
        // metal it is forged from. A soldier draws from the armoury; an
        // adventurer wields whatever they carry; a bare-handed brawler has
        // only fists.
        let weapon = self
            .wielded_weapon(attacker, raws)
            .and_then(|w| {
                let it = &self.items[w];
                let v = it.weapon_variant()?;
                Some((
                    raws.weapons.damage_type(v),
                    raws.weapons.heft(v),
                    raws.materials.get(it.stuff).combat,
                ))
            })
            .or_else(|| {
                // A beast fights with claw and bulk, not a fist. Its natural
                // weapon is a rending, full-weight blow — otherwise the
                // feeble-fist fallback would rob a monster of its menace.
                self.dwarves[attacker].beast.then_some((
                    DamageType::Edge,
                    1.4,
                    CombatStats { sharpness: 1.5, density: 7.8, hardness: 120.0 },
                ))
            });
        let verb = self
            .wielded_weapon(attacker, raws)
            .and_then(|w| self.items[w].weapon_variant())
            .map(|v| raws.weapons.verb(v))
            .unwrap_or("strikes");

        self.land_hit(attacker, defender, force, weapon, part_kind, verb, raws);
    }

    /// Pick which body part a blow strikes — torso most often, head the
    /// deadliest. One rng draw; kept as a helper so melee and ranged fire roll
    /// it the same way.
    fn roll_part(&mut self) -> PartKind {
        match self.rng.gen_range(0..8usize) {
            0 => PartKind::Head,
            1 | 2 | 3 => PartKind::Torso,
            4 => PartKind::LeftArm,
            5 => PartKind::RightArm,
            6 => PartKind::LeftLeg,
            _ => PartKind::RightLeg,
        }
    }

    /// Resolve one landed attack against `defender`: work the wound out against
    /// whatever armour they wear, narrate it with `verb`, and handle death,
    /// kill-stats, and a werebeast's curse. Shared by a melee swing and a
    /// marksdwarf's bolt — the caller supplies the force, weapon (a bolt is just
    /// a fast, distant Pierce), and the pre-rolled body part. The defender's
    /// chance to dodge or block is spent by the caller BEFORE this, so a hit
    /// here lands.
    fn land_hit(
        &mut self,
        attacker: usize,
        defender: usize,
        force: f32,
        weapon: Option<(DamageType, f32, CombatStats)>,
        part_kind: PartKind,
        verb: &str,
        raws: &Raws,
    ) {
        // The armour the defender wears, if the fort issued them any.
        let armor = self
            .worn_armor(defender)
            .map(|a| raws.materials.get(self.items[a].stuff).combat);

        let blow = resolve_blow(force, weapon, armor);
        let (dmg, bleed) = (blow.damage, blow.bleed);
        // Drawing blood teaches the trade: every landed blow hones prowess.
        self.add_xp(attacker, Skill::Fighting, 6);

        let att_name = self.dwarves[attacker].name.clone();
        let def_name = self.dwarves[defender].name.clone();
        let d = &mut self.dwarves[defender];
        let Some(part) = d.body.iter_mut().find(|pt| pt.kind == part_kind) else { return };
        part.hp -= dmg;
        part.bleeding = part.bleeding.saturating_add(bleed);
        let destroyed = part.hp <= 0;
        let just_destroyed = destroyed && part.hp + dmg > 0;
        let vital = part.kind.vital();
        self.log_event(format!(
            "{att_name} {verb} {def_name} in the {}!",
            part_kind.name()
        ));
        // A limb hewn off maims; a head hewn off decapitates on top of death.
        if just_destroyed && (!vital || part_kind == PartKind::Head) {
            self.sever_part(defender, part_kind);
        }
        if destroyed && vital {
            self.log_event(format!("{def_name} falls dead!"));
            // Merchants killed by raiders are a tragedy, not your crime.
            if self.dwarves[defender].faction == Faction::Visitor
                && self.dwarves[attacker].faction == Faction::Hostile
            {
                self.trader_lost_to_raiders = true;
            }
            let was_beast = self.dwarves[defender].beast;
            let was_hostile = self.dwarves[defender].faction == Faction::Hostile;
            self.kill_dwarf(defender);
            if was_beast {
                self.stats.beasts_slain += 1;
            } else if was_hostile {
                self.stats.raiders_slain += 1;
            }
        }
        // A werebeast's bite passes the curse to a fort-mate who survives it.
        // The && short-circuits, so no rng is drawn unless a beast is attacking.
        if self.dwarves[attacker].were_form
            && self.dwarves[defender].alive
            && self.dwarves[defender].faction == Faction::Fort
            && !self.dwarves[defender].werebeast
            && self.rng.gen_ratio(1, 4)
        {
            self.dwarves[defender].werebeast = true;
            let name = self.dwarves[defender].name.clone();
            self.log_event(format!("{name} is savaged by the beast -- the curse takes root."));
        }
    }

    /// A marksdwarf looses a bolt at a foe at range: consume a bolt from the
    /// fort's quiver, let the target try to dodge or block, and on a hit resolve
    /// it as a fast Pierce through their armour. Returns false (drawing no rng)
    /// if there is no bolt to fire, so the caller falls back to closing in.
    fn fire_bolt(&mut self, attacker: usize, defender: usize, raws: &Raws) -> bool {
        if self.dwarves[attacker].attack_cd > 0 {
            self.dwarves[attacker].attack_cd -= 1;
            return true;
        }
        if self.bolts == 0 {
            return false; // out of ammo — fall back to the crossbow's bash
        }
        self.dwarves[attacker].attack_cd = ATTACK_COOLDOWN;
        self.bolts -= 1;
        self.stats.bolts_fired += 1;

        // A bolt can be dodged or turned by a shield, but there is no parrying
        // an arrow out of the air.
        if let Some(defence) = self.try_defend(defender, raws) {
            let att = self.dwarves[attacker].name.clone();
            let def = self.dwarves[defender].name.clone();
            self.log_event(format!("{def} {defence} from {att}'s bolt."));
            return true;
        }

        // The bolt's bite: the crossbow's launch force plus the marksdwarf's
        // aim, driven as a Pierce with the bolt's own metal.
        let part_kind = self.roll_part();
        let force =
            14.0 + fighting_bonus(self.dwarves[attacker].skill_level(Skill::Fighting)) as f32;
        let bolt = (
            DamageType::Pierce,
            0.4,
            CombatStats { sharpness: 1.0, density: 7.8, hardness: 100.0 },
        );
        self.land_hit(attacker, defender, force, Some(bolt), part_kind, "fires a bolt into", raws);
        true
    }

    /// Stain a tile, and lightly the walkable ground around it, with blood.
    /// Deterministic (no rng) so it never perturbs combat. `amount` is the fresh
    /// intensity — a heavy pool for a death, a light drip for a bleeding wound.
    pub fn spatter_blood(&mut self, at: Pos, amount: u16) {
        let e = self.blood.entry(at).or_insert(0);
        *e = (*e).saturating_add(amount).min(BLOOD_MAX);
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let n = Pos::new(at.x + dx, at.y + dy, at.z);
            if self.map.tile_at(n).is_some_and(|t| t.shape.is_walkable()) {
                let e = self.blood.entry(n).or_insert(0);
                *e = (*e).saturating_add(amount / 3).min(BLOOD_MAX);
            }
        }
    }

    /// A part hacked past destruction is struck clean off: it drops to the
    /// ground as gore, the stump gushes blood, and the wound sprays the tile.
    /// A lost limb maims but need not kill (the caller handles a fatal head or
    /// torso). Deterministic (no rng), so combat stays byte-identical.
    pub fn sever_part(&mut self, defender: usize, part_kind: PartKind) {
        let pos = self.dwarves[defender].pos;
        let who = self.dwarves[defender].name.clone();
        self.log_event(format!("{who}'s {} is struck clean off!", part_kind.name()));
        // The severed part lands where the creature stands. `stuff` carries the
        // rot stage (0 = fresh); the name holds the bare part, and the display
        // layer prefixes it ("severed" / "rotting" / "... bones") by stage.
        self.spawn_named_item(ItemKind::BodyPart, 0, pos, Some(part_kind.name().to_string()));
        // The stump gushes: the open wound bleeds hard until it clots or kills.
        if let Some(part) = self.dwarves[defender]
            .body
            .iter_mut()
            .find(|pt| pt.kind == part_kind)
        {
            part.bleeding = part.bleeding.saturating_add(50);
        }
        // Blood sprays across the tile and its surrounds.
        self.spatter_blood(pos, BLOOD_MAX);
    }

    /// Bloody footprints: a creature that treads through a pool carries blood on
    /// its feet and prints it, fainter each step, as it walks away — until its
    /// feet run clean. Called each tick; deterministic (no rng), so combat stays
    /// identical.
    pub fn tick_footprints(&mut self) {
        for i in 0..self.dwarves.len() {
            if !self.dwarves[i].alive {
                continue;
            }
            let cur = self.dwarves[i].pos;
            if cur != self.dwarves[i].last_pos {
                let tracked = self.dwarves[i].blood_tracked;
                if tracked >= 6 {
                    let dep = (tracked / 3).max(6);
                    let e = self.blood.entry(cur).or_insert(0);
                    *e = (*e).saturating_add(dep).min(BLOOD_MAX);
                    self.dwarves[i].blood_tracked = tracked - dep;
                }
                // Only a real pool (not a footprint) reloads the feet.
                if self.blood.get(&cur).copied().unwrap_or(0) > 20 {
                    let t = self.dwarves[i].blood_tracked;
                    self.dwarves[i].blood_tracked = (t + 24).min(60);
                }
            }
            self.dwarves[i].last_pos = cur;
        }
    }

    /// Severed parts rot where they lie: fresh gore sours to carrion within a
    /// day, and is picked clean to bone over a few. The stage rides in the
    /// item's `stuff` field (0 fresh, 1 rotting, 2 skeletal) and only ever
    /// advances. Deterministic (age from the clock, no rng).
    pub fn decay_gore(&mut self) {
        let now = self.clock.tick;
        for it in &mut self.items {
            if it.kind != ItemKind::BodyPart || it.consumed {
                continue;
            }
            let age = now.saturating_sub(it.made_at);
            let stage: u16 = if age >= GORE_SKELETONIZE {
                2
            } else if age >= GORE_ROT {
                1
            } else {
                0
            };
            if it.stuff < stage {
                it.stuff = stage;
            }
        }
    }

    /// Blood dries and fades a little each pass; a tile with none left is
    /// forgotten.
    pub fn dry_blood(&mut self) {
        self.blood.retain(|_, v| {
            *v = v.saturating_sub(BLOOD_DRY);
            *v > 0
        });
    }

    /// Blood, breath, bleeding, and rest-healing — applies to every faction.
    fn tick_vitals(&mut self, i: usize) {
        let tick = self.clock.tick;
        let pos = self.dwarves[i].pos;
        let submerged = self.map.water_at(pos) >= 5;
        // A tended ward mends the fort's own far faster.
        let in_hospital = self.dwarves[i].faction == Faction::Fort && self.hospital_at(pos);
        // Sleeping in a proper bed mends wounds faster than resting on stone
        // (though a hospital ward is faster still).
        let in_bed =
            matches!(self.dwarves[i].task, Task::Sleep { .. }) && self.sleeps_in_bed(i);
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
        // Sleeping, a hospital stay, or huddling in a burrow all count as rest
        // for the purpose of mending (a sheltered dwarf shouldn't be sent into
        // a raid to heal, but they still recover slowly where they hide).
        let resting = matches!(
            d.task,
            Task::Sleep { .. } | Task::Recover { .. } | Task::Shelter { .. }
        );
        // The ward stanches bleeding and knits wounds several times faster; a
        // bed rests between the two.
        let decay_every = if in_hospital {
            40
        } else if in_bed {
            70
        } else if resting {
            100
        } else {
            400
        };
        if tick % decay_every == 0 {
            for p in &mut d.body {
                p.bleeding = p.bleeding.saturating_sub(1);
            }
        }
        let heal_every = if in_hospital {
            60
        } else if in_bed {
            120
        } else {
            200
        };
        if (resting || in_hospital) && tick % heal_every == 0 {
            for p in &mut d.body {
                if p.hp < p.max_hp {
                    p.hp += 1;
                }
            }
        }
        let bled_out = d.blood <= 0.0;

        let hostile = self.dwarves[i].faction == Faction::Hostile;
        let beast = self.dwarves[i].beast;
        let credit_kill = |stats: &mut SimStats| {
            if beast {
                stats.beasts_slain += 1;
            } else if hostile {
                stats.raiders_slain += 1;
            }
        };
        if incinerated {
            credit_kill(&mut self.stats);
            self.log_event(format!("{name} is incinerated by magma!"));
            self.kill_dwarf(i);
        } else if drowned {
            // HUD semantics: drownings counts hostiles killed by floods.
            if hostile {
                self.stats.drownings += 1;
            }
            if beast {
                self.stats.beasts_slain += 1;
            }
            self.log_event(format!("{name} has drowned."));
            self.kill_dwarf(i);
        } else if bled_out {
            credit_kill(&mut self.stats);
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
        // A dwarf dressed in good clothes frets a little less (computed before
        // the mutable borrow; a no-op unless the fort has sewn any clothes).
        let clothed = self.wears_clothes(i);
        // A fort adorned with statues lifts every citizen's spirits a touch —
        // a no-op unless at least one statue has been carved.
        let adorned = self.count_kind(ItemKind::Statue) > 0;
        let d = &mut self.dwarves[i];
        let old_hunger = d.hunger;
        let old_thirst = d.thirst;
        // A vampire takes no food or drink — it feeds only on blood, and so
        // never crosses the hunger/thirst thresholds or dies of its needs.
        if !d.vampire {
            d.hunger = (d.hunger + HUNGER_RATE).min(100.0);
            d.thirst = (d.thirst + THIRST_RATE).min(100.0);
        }
        d.fatigue = (d.fatigue + FATIGUE_RATE).min(100.0);
        // Happiness drifts back toward neutral.
        d.happiness += (50.0 - d.happiness).signum() * 0.0005;

        // Stress slowly drains in calm times; boiling over breaks the mind.
        // Fine clothes and a hall of statues each ease the mind a touch faster.
        let mut calm = 0.001;
        if clothed {
            calm += 0.0005;
        }
        if adorned {
            calm += 0.0005;
        }
        d.stress = (d.stress - calm).max(0.0);
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
        // Death pools blood where the body falls.
        let death_pos = self.dwarves[i].pos;
        self.spatter_blood(death_pos, BLOOD_MAX);
        // A raider's gear leaves with the raider — it is not fort property, and
        // making it lootable would tie the fort's weapon count to its body
        // count and break raider-wealth determinism. Consume it BEFORE
        // `abandon_task`, which would otherwise drop it to the ground first.
        // (Adventure mode still spawns a fresh loot blade below — the hero's
        // spoils are a deliberate, separate thing.)
        if self.dwarves[i].faction == Faction::Hostile {
            for it in &mut self.items {
                if it.state == (ItemState::Carried { by: i }) {
                    it.consumed = true;
                }
            }
        }
        self.abandon_task(i);
        // Release anything still pointing at this dwarf.
        for it in &mut self.items {
            if it.reserved_by == Some(i) {
                it.reserved_by = None;
            }
        }
        // Their bed passes to whoever needs it next; the dead sleep elsewhere.
        self.release_bed(i);
        // A fallen soldier is struck from the muster rolls.
        self.discharge_from_squads(i);
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
        } else if self.player.is_some()
            && self.dwarves[i].faction == Faction::Hostile
            && !self.dwarves[i].beast
        {
            // Spoils of war: in an adventure, a slain raider leaves their blade
            // for the hero to take up. Gated on adventure mode (a live player)
            // so ordinary fortress play — and its determinism — is unchanged.
            let pos = self.dwarves[i].pos;
            self.spawn_quality_item(ItemKind::Weapon, 0, pos, 2);
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

    /// Spawn a crafted good bearing a quality tier (0..=5).
    fn spawn_quality_item(&mut self, kind: ItemKind, stuff: u16, pos: Pos, quality: u8) {
        self.spawn_named_item(kind, stuff, pos, None);
        if let Some(it) = self.items.last_mut() {
            it.quality = quality;
        }
    }

    fn spawn_named_item(&mut self, kind: ItemKind, stuff: u16, pos: Pos, name: Option<String>) {
        let made_at = self.clock.tick;
        self.items.push(Item {
            kind,
            stuff,
            name,
            pos,
            state: ItemState::OnGround,
            reserved_by: None,
            consumed: false,
            quality: 0,
            made_at,
            variant: 0,
        });
    }

    /// Set the kind of the weapon most recently spawned. Weapons come off the
    /// forge through `spawn_quality_item`, which knows nothing of kinds — this
    /// stamps the one just made.
    fn set_last_weapon(&mut self, variant: u16) {
        if let Some(it) = self.items.last_mut() {
            it.variant = variant as u8;
        }
    }

    /// Pick up an item the dwarf is standing on. Returns false if it's gone.
    /// Pick an item up off the tile the dwarf is standing on — including out
    /// of a barrel or bin resting there, which is how a brewer gets at a
    /// packed crop. `item_takeable` decides whether a job may be CLAIMED;
    /// this is the moment the hand closes on the thing, and the two must agree
    /// on what is reachable, or dwarves walk to a barrel and give up in
    /// silence.
    fn take_item(&mut self, i: usize, item: usize) -> bool {
        let it = &self.items[item];
        if !it.active()
            || it.pos != self.dwarves[i].pos
            || it.reserved_by != Some(i)
            || !matches!(
                it.state,
                ItemState::OnGround | ItemState::Stored { .. } | ItemState::Inside { .. }
            )
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

    /// A daily flourish of culture: a poet composes a work at the tavern, and
    /// a scholar sets down a treatise in the library. Deterministic (no RNG, no
    /// dwarf state touched), so it never perturbs the simulation — pure flavor.
    fn tick_culture(&mut self) {
        if self.taverns.is_empty() && self.library.is_empty() {
            return;
        }
        let bards: Vec<usize> = self
            .dwarves
            .iter()
            .enumerate()
            .filter(|(_, d)| d.alive && d.faction == Faction::Fort)
            .map(|(i, _)| i)
            .collect();
        if bards.is_empty() {
            return;
        }
        let day = self.clock.tick / TICKS_PER_DAY;
        // Poetry at the tavern.
        if !self.taverns.is_empty() {
            let poet = bards[(day as usize) % bards.len()];
            let work = self.compose_poem(day, poet);
            let name = self.dwarves[poet].name.clone();
            self.log_event(format!("{name} composes {work} at the tavern."));
            self.poems.push(work);
            if self.poems.len() > 100 {
                self.poems.remove(0);
            }
        }
        // Music at the tavern — but only once the fort has an instrument to play.
        if !self.taverns.is_empty() && self.count_kind(ItemKind::Instrument) > 0 {
            let player = bards[(day as usize + 2) % bards.len()];
            let work = self.compose_song(day, player);
            let name = self.dwarves[player].name.clone();
            self.log_event(format!("{name} plays {work} at the tavern."));
            self.songs.push(work);
            if self.songs.len() > 100 {
                self.songs.remove(0);
            }
        }
        // Scholarship at the library.
        if !self.library.is_empty() {
            let scholar = bards[(day as usize + 1) % bards.len()];
            let work = self.compose_treatise(day, scholar);
            let name = self.dwarves[scholar].name.clone();
            self.log_event(format!("{name} sets down {work} in the library."));
            self.treatises.push(work);
            if self.treatises.len() > 100 {
                self.treatises.remove(0);
            }
        }
    }

    /// Compose a scholarly work, drawn deterministically from the day, the
    /// scholar, and the world the fort knows.
    fn compose_treatise(&self, day: u64, scholar: usize) -> String {
        const FORMS: [&str; 5] = [
            "a treatise on",
            "a history of",
            "a study of",
            "a discourse on",
            "an inquiry into",
        ];
        let mut subjects: Vec<String> = vec![
            "the working of stone and metal".to_string(),
            "the turning of the seasons".to_string(),
            "the deep places of the world".to_string(),
            "the breeding of beasts".to_string(),
            "the properties of magma and water".to_string(),
        ];
        if let Some(roster) = &self.siege_roster {
            subjects.push(format!("the wars against {}", roster.civ_name));
        }
        if let Some(civ) = &self.trade_partner {
            subjects.push(format!("the customs of {}", civ));
        }
        if self.stats.beasts_slain > 0 {
            subjects.push("the anatomy of forgotten beasts".to_string());
        }
        let d = day as usize;
        let form = FORMS[d % FORMS.len()];
        let subject = &subjects[(d / 2 + scholar) % subjects.len()];
        format!("{form} {subject}")
    }

    /// Compose a titled poetic work, spun deterministically from the day and
    /// the poet, so a given fortress always sings the same songs — and, where
    /// it can, about its own story rather than empty abstraction.
    fn compose_poem(&self, day: u64, poet: usize) -> String {
        const FORMS: [&str; 5] = ["a poem", "a saga", "an ode", "a lament", "a ballad"];
        const ADJS: [&str; 8] = [
            "Silent", "Golden", "Bloodied", "Deep", "Endless", "Iron", "Frozen", "Forgotten",
        ];
        const NOUNS: [&str; 8] = [
            "Depths", "Halls", "Axe", "Mountain", "Winter", "Hearth", "Dark", "Oath",
        ];
        let d = day as usize;
        let form = FORMS[d % FORMS.len()];
        let adj = ADJS[(d + poet) % ADJS.len()];
        let noun = NOUNS[(d / 3 + poet * 2) % NOUNS.len()];

        // Subjects drawn from the fortress's own life, so its verse is its own.
        let mut subjects: Vec<String> = Vec::new();
        for dw in &self.dwarves {
            if !dw.alive && dw.faction == Faction::Fort && !dw.ghost {
                subjects.push(format!("{}, gone to the earth", dw.name));
            }
        }
        if let Some(roster) = &self.siege_roster {
            subjects.push(format!("the long hatred of {}", roster.civ_name));
        }
        if self.stats.beasts_slain > 0 {
            subjects.push("the beast that rose from the deep".to_string());
        }
        if self.stats.raiders_slain > 0 {
            subjects.push("the raiders broken at the gate".to_string());
        }
        subjects.push("stone, and cold ale, and the mountain's heart".to_string());
        let subject = &subjects[(d + poet) % subjects.len()];

        format!("{form}, \"The {adj} {noun}\", of {subject}")
    }

    /// Compose a song, drawn deterministically from the day, the player, and the
    /// fort's own life — like its verse, its music is its own. Draws no rng.
    fn compose_song(&self, day: u64, player: usize) -> String {
        const FORMS: [&str; 5] = ["a reel", "a march", "a dirge", "an air", "a jig"];
        const ADJS: [&str; 8] = [
            "Merry", "Mournful", "Roaring", "Quiet", "Brazen", "Ancient", "Stirring", "Wandering",
        ];
        const NOUNS: [&str; 8] = [
            "Miner", "Anvil", "Tankard", "Gate", "Hearthstone", "Deep", "Homeland", "Vein",
        ];
        let d = day as usize;
        let form = FORMS[(d + player) % FORMS.len()];
        let adj = ADJS[(d / 2 + player) % ADJS.len()];
        let noun = NOUNS[(d + player * 3) % NOUNS.len()];

        let mut subjects: Vec<String> = Vec::new();
        for dw in &self.dwarves {
            if !dw.alive && dw.faction == Faction::Fort && !dw.ghost {
                subjects.push(format!("{}, remembered in song", dw.name));
            }
        }
        if self.stats.raiders_slain > 0 || self.stats.beasts_slain > 0 {
            subjects.push("the fight at the gate".to_string());
        }
        if let Some(partner) = &self.trade_partner {
            subjects.push(format!("the long road to {partner}"));
        }
        subjects.push("the pick, the pint, and the long dark".to_string());
        let subject = &subjects[(d + player) % subjects.len()];

        format!("{form}, \"The {adj} {noun}\", of {subject}")
    }

    /// Compose a scene for an engraving from the fortress's own history —
    /// its fallen, its foes, its triumphs — so its walls remember its story.
    fn compose_engraving(&mut self) -> String {
        let mut subjects: Vec<String> = Vec::new();
        // The fallen are memorialized.
        for d in &self.dwarves {
            if !d.alive && d.faction == Faction::Fort && !d.ghost {
                subjects.push(format!("{}, a dwarf of the fortress, now passed", d.name));
            }
        }
        // Named enemies from the world's history.
        if let Some(roster) = &self.siege_roster {
            if let Some(leader) = roster.leaders.first() {
                subjects.push(format!(
                    "{} of {}, who {}",
                    leader.name, roster.civ_name, leader.grudge
                ));
            }
        }
        // Deeds of an adventuring hero, if any.
        for deed in &self.deeds {
            subjects.push(deed.clone());
        }
        // Triumphs recorded in the fort's tallies.
        if self.stats.beasts_slain > 0 {
            subjects.push("the slaying of a forgotten beast in the deeps".to_string());
        }
        if self.stats.raiders_slain > 0 {
            subjects.push("the fortress guard driving raiders from the gates".to_string());
        }
        if self.dwarves.iter().any(|d| d.artifacts_made > 0) {
            subjects.push("a masterwork born of a fey mood".to_string());
        }
        if self.stats.caravans_arrived > 0 {
            subjects.push("a merchant caravan come to trade".to_string());
        }
        // There is always the founding to remember.
        subjects.push("the founding of the fortress".to_string());

        let pick = self.rng.gen_range(0..subjects.len());
        format!("an engraving of {}", subjects[pick])
    }

    fn complete_mine(&mut self, i: usize, target: Pos, raws: &Raws) {
        let Some(des) = self.designations.remove(&target) else {
            self.dwarves[i].task = Task::Idle { wander_cd: 5 };
            return;
        };
        let tile = self.map.tile_at(target).expect("designated tile in bounds");
        // Smoothing leaves the wall standing but carves a scene into its face.
        if des.kind == DesignationKind::Smooth {
            // The wall may have been dug away before the mason arrived; never
            // engrave open space.
            if !tile.is_solid() {
                self.dwarves[i].task = Task::Idle { wander_cd: 2 };
                return;
            }
            let scene = self.compose_engraving();
            self.engravings.insert(target, scene.clone());
            self.add_xp(i, Skill::Crafting, 15);
            let name = self.dwarves[i].name.clone();
            self.log_event(format!("{name} engraves a wall: {scene}"));
            self.dwarves[i].task = Task::Idle { wander_cd: 2 };
            return;
        }
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
            DesignationKind::Smooth => unreachable!("smoothing handled above"),
            DesignationKind::Chop => unreachable!("chopping handled in Task::Chop"),
            DesignationKind::Gather => unreachable!("foraging handled in Task::Gather"),
        }
        // Digging the wall away destroys any scene engraved on it.
        self.engravings.remove(&target);
        self.regions.dirty = true;
        self.map_changed = true;
        // Breach an aquifer and the opened tile weeps water without end — the
        // fort must wall it off or pump it out. The tile becomes a spring the
        // water automaton keeps brimming; a wall raised over it shuts it (a
        // solid tile holds no water, so the spring falls dormant).
        if self.aquifers.remove(&target) {
            self.water.springs.insert(target);
            let name = self.dwarves[i].name.clone();
            self.log_event(format!("{name} breaches an aquifer — water floods in!"));
        }
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
            // A glint in the stone: rarely, the pick strikes a gem. The
            // gen_ratio roll is taken first and unchanged (so the rng stream is
            // identical to before gems were data), then guarded against an empty
            // gem set — data-driving turns the old const `.len()` into a possible
            // `gen_range(0..0)` panic if a mod ships no gems.
            if self.rng.gen_ratio(1, 22) && !raws.gems.is_empty() {
                let gem = self.rng.gen_range(0..raws.gems.len()) as u16;
                self.spawn_item(ItemKind::RoughGem, gem, drop_at);
                self.stats.gems_found += 1;
                let name = self.dwarves[i].name.clone();
                self.log_event(format!("{name} strikes a rough {}!", raws.gems.name(gem)));
            }
        }
        // Adamantine — the deep wonder-metal. And if this cap held back the
        // abyss, digging it out looses the underworld.
        if raws.materials.get(boulder_from.material).category == MaterialCategory::Adamantine {
            let name = self.dwarves[i].name.clone();
            self.log_event(format!("{name} strikes ADAMANTINE! Praise the deep."));
        }
        if self.adamantine_breaches.remove(&target) {
            self.breach_underworld(target, raws);
        }
        self.add_xp(i, Skill::Mining, 20);
        self.dwarves[i].task = Task::Idle { wander_cd: 2 };
    }

    /// Centralized cleanup: release every reservation the current task holds,
    /// drop anything carried, and go idle. Safe to call in any state.
    fn abandon_task(&mut self, i: usize) {
        match self.dwarves[i].task.clone() {
            Task::Mine { target, .. }
            | Task::Chop { tree: target, .. }
            | Task::Gather { shrub: target, .. } => {
                if let Some(des) = self.designations.get_mut(&target) {
                    des.assigned = false;
                    des.retry_at = self.clock.tick + RETRY_DELAY;
                }
            }
            Task::Haul { item, .. }
            | Task::Eat { item, .. }
            | Task::Drink { item, .. }
            | Task::DineAt { item, .. } => {
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
            Task::Build { site, input, .. } => {
                if self.items[input].reserved_by == Some(i) {
                    self.items[input].reserved_by = None;
                }
                // Free the plan for another builder.
                if let Some(assigned) = self.constructions.get_mut(&site) {
                    *assigned = false;
                }
            }
            Task::Butcher { animal, .. } | Task::Train { animal, .. } => {
                if let Some(a) = self.animals.get_mut(animal) {
                    if a.reserved_by == Some(i) {
                        a.reserved_by = None;
                    }
                }
            }
            Task::Idle { .. }
            | Task::Sleep { .. }
            // A bed is owned, not reserved per-job: nothing to release.
            | Task::GoToBed { .. }
            | Task::Fight { .. }
            | Task::Tantrum { .. }
            | Task::Sulk { .. }
            | Task::Relax { .. }
            | Task::Fish { .. }
            | Task::Pray { .. }
            | Task::Recover { .. }
            | Task::Spar { .. }
            | Task::Station { .. }
            | Task::Patrol { .. }
            | Task::Shelter { .. }
            | Task::DrinkWell { .. } => {}
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

/// The lowest walkable surface tile of a map, where a natural spring rises.
/// Deterministic in the map, so every entry into a region — embark or
/// adventure travel — gives it the same spring.
fn natural_spring(map: &Map) -> Option<Pos> {
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
    lowest.map(|(z, x, y)| Pos::new(x, y, z as i32))
}

fn new_dwarf(rng: &mut ChaCha8Rng, pos: Pos, faction: Faction, raws: &Raws) -> Dwarf {
    Dwarf {
        name: names::dwarf_name(rng),
        pos,
        alive: true,
        faction,
        bed: None,
        hunger: rng.gen_range(0.0..20.0),
        thirst: rng.gen_range(0.0..20.0),
        fatigue: rng.gen_range(0.0..30.0),
        happiness: 50.0,
        blood: 100.0,
        blood_tracked: 0,
        last_pos: pos,
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
        beast: false,
        soldier: false,
        last_prayer: 0,
        follower: false,
        vampire: false,
        last_fed: 0,
        werebeast: false,
        were_form: false,
        necromancer: false,
    }
}

// ------------------------------------------------------- the world outside

/// How many lines of foreign news the fort will hear in a single year, so a
/// busy century abroad can't drown out the log of what happened at home. A
/// backstop, not a filter: a year rarely makes even this much news about the
/// two civs a given fort knows.
const NEWS_PER_YEAR: usize = 3;

/// Keep the world's calendar in step with the fortress's: for every year the
/// fort has lived, the world outside lives one too. Cheap to call every tick
/// — it does nothing until a year actually turns.
///
/// This is the one glue point between a fort and its world (the app calls it
/// each tick; tests call it directly). Sieges and caravans were wired once at
/// embark from a world that then stood still; now that the world keeps
/// turning, they are rewired from it as it changes. A civ can be ground into
/// ruins by wars you never see and simply stop coming — the caravan that
/// never arrives is a story the world told without you.
pub fn sync_world(sim: &mut Sim, world: &mut dk_history::World) {
    if sim.embark_world_year == 0 {
        return; // a sim with no world behind it (tests, dummy maps)
    }
    // The fort's clock starts at year 1, so its first year adds nothing.
    let target = sim.embark_world_year + sim.clock.year() as u32 - 1;
    if world.years_simulated >= target {
        return; // the common case: no year has turned since the last tick
    }
    // A fort only hears news of the peoples it actually knows: the neighbors
    // who trade with it and the enemies who hate it. The rest of the world's
    // noise never reaches the mountainhome.
    let known: Vec<String> = sim
        .trade_partner
        .iter()
        .cloned()
        .chain(sim.siege_roster.iter().map(|r| r.civ_name.clone()))
        .collect();
    while world.years_simulated < target {
        let news = world.advance_year();
        for line in news
            .iter()
            .filter(|l| known.iter().any(|c| l.contains(c.as_str())))
            .take(NEWS_PER_YEAR)
        {
            sim.log_event(format!("Word arrives from afar: {line}"));
        }
    }

    // A trade partner whose every site lies in ruins sends no more wagons.
    if let Some(partner) = sim.trade_partner.clone() {
        if world.civ_fallen(&partner) {
            sim.log_event(format!(
                "{partner} has been destroyed. No caravan will ever come from them again."
            ));
            sim.trade_partner = None;
        }
    }

    // Likewise the enemy: a horde razed out of existence besieges no one.
    // Otherwise refresh the roster — the warlord who swore to burn your gates
    // may have died abroad, and new grudges make new enemies.
    if let (Some(roster), Some(region)) = (sim.siege_roster.clone(), sim.home_region) {
        if world.civ_fallen(&roster.civ_name) {
            sim.log_event(format!(
                "{} has been wiped from the world. Their sieges end here.",
                roster.civ_name
            ));
            sim.siege_roster = None;
        } else if let Some((civ_name, leaders)) = world.siege_pack(region.0, region.1) {
            let leaders: Vec<SiegeLeader> = leaders
                .into_iter()
                .map(|(name, grudge)| SiegeLeader { name, grudge })
                .collect();
            // Never leave the fort without a named enemy: an empty refresh
            // (every grudge-bearer dead) keeps the roster we already had.
            if !leaders.is_empty() {
                sim.siege_roster = Some(SiegeRoster { civ_name, leaders });
            }
        }
    }
}

// ------------------------------------------------------------------- saves

const SAVE_MAGIC: u32 = 0x444B_5331; // "DKS1"
// v70: broadened the stone list (new sedimentary/igneous/metamorphic rocks +
// obsidian), which shifts material indices, so a pre-v70 fort save would read
// the wrong stone — reject it rather than misinterpret.
// v71: added the "steel" alloy material (and the is_flux flag), shifting indices
// again — same reason to reject older saves.
// v72: forts gained an `aquifers` set (water-bearing rock tiles).
// v73: forts gained a `cavern_floors` set (the deep cavern layer).
// v74: forts gained a `blood` map (spatter on the ground).
// v75: creatures gained blood_tracked/last_pos for bloody footprints.
// v76: new ItemKind::BodyPart (severed limbs) shifts the item-kind enum.
// v77: ItemKind::BoneCraft + CraftKind::BoneCraft (bones as trade goods).
const SAVE_VERSION: u32 = 83;

#[derive(Serialize)]
struct SaveOut<'a> {
    magic: u32,
    version: u32,
    material_ids: Vec<String>,
    plant_ids: Vec<String>,
    gem_ids: Vec<String>,
    weapon_ids: Vec<String>,
    /// The mods (id, version) that made this fort, so a load can tell whether
    /// they are present before it tries to resolve their content.
    mods: Vec<(String, String)>,
    sim: &'a Sim,
}

// Read as header-then-body so version mismatches produce a clear error
// instead of a bincode failure mid-struct. bincode serializes struct fields
// sequentially, so this matches SaveOut's layout exactly.
type SaveBody = (Vec<String>, Vec<String>, Vec<String>, Vec<String>, Vec<(String, String)>, Sim);

/// Render a mod stamp for a player-facing message: "cool_mod v1.0.0, other v2.1"
/// or "none".
fn fmt_mod_stamp(mods: &[(String, String)]) -> String {
    if mods.is_empty() {
        "none".to_string()
    } else {
        mods.iter()
            .map(|(id, v)| format!("{id} v{v}"))
            .collect::<Vec<_>>()
            .join(", ")
    }
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
        gem_ids: raws.gems.id_manifest(),
        weapon_ids: raws.weapons.id_manifest(),
        mods: raws.mod_stamp(),
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
    let (material_ids, plant_ids, gem_ids, weapon_ids, saved_mods, sim): SaveBody =
        bincode::deserialize_from(&mut reader)
            .with_context(|| format!("deserializing {}", path.display()))?;
    // Refuse a fort whose mods aren't the ones now loaded, with a clear message —
    // friendlier than the cryptic "material 'X' missing" the remap would raise if
    // a mod that added content is gone. Compared as a set (order-independent): the
    // id-manifest remap already tolerates a load-order change. `mod_stamp` carries
    // version too, so a different version of a mod is treated as a different mod.
    {
        let mut have = raws.mod_stamp();
        have.sort();
        let mut need = saved_mods.clone();
        need.sort();
        anyhow::ensure!(
            have == need,
            "this fortress was made with mods [{}] but you have [{}] — load the matching mods to reclaim it",
            fmt_mod_stamp(&saved_mods),
            fmt_mod_stamp(&raws.mod_stamp()),
        );
    }
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
    // Gems are data now (dk_raws::GemRegistry), stored by index in a RoughGem/
    // CutGem's `stuff` — remap them like materials/plants so a reordered or
    // mod-extended gem list survives a save. (A missing gem should already be
    // caught by the mod-set check above, but name it clearly just in case.)
    let gem_remap: Vec<u16> = gem_ids
        .iter()
        .map(|id| {
            raws.gems
                .index_of(id)
                .with_context(|| format!("save uses gem '{id}' missing from current raws"))
        })
        .collect::<Result<_>>()?;
    // Weapons are data too (dk_raws::WeaponRegistry); a Weapon's kind is stored
    // by index in `variant`, remapped like gems so a reordered/mod-extended
    // weapon list survives a save.
    let weapon_remap: Vec<u16> = weapon_ids
        .iter()
        .map(|id| {
            raws.weapons
                .index_of(id)
                .with_context(|| format!("save uses weapon '{id}' missing from current raws"))
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
            ItemKind::Boulder
            | ItemKind::Artifact
            | ItemKind::Craft
            | ItemKind::Weapon
            | ItemKind::Glass
            | ItemKind::Bar
            | ItemKind::Armor
            | ItemKind::Shield
            | ItemKind::Bed
            // A log and everything worked from it (see also Barrel/Instrument
            // below stay flat) carries a wood-material index.
            | ItemKind::Log
            | ItemKind::Statue => remap_one(&mat_remap, item.stuff, "material")?,
            // A rough or cut gem stores its gem index in `stuff`.
            ItemKind::RoughGem | ItemKind::CutGem => remap_one(&gem_remap, item.stuff, "gem")?,
            // These carry no raws index (dwarf index, or nothing).
            ItemKind::Corpse
            | ItemKind::Wool
            | ItemKind::Cloth
            | ItemKind::Clothes
            | ItemKind::Barrel
            | ItemKind::Bin
            | ItemKind::Instrument
            | ItemKind::Hide
            | ItemKind::Leather => item.stuff,
            _ => remap_one(&plant_remap, item.stuff, "plant")?,
        };
        // A weapon's `stuff` is its metal (remapped above); its `variant` is its
        // kind, an index into the weapon registry — remap that too.
        if item.kind == ItemKind::Weapon {
            item.variant = remap_one(&weapon_remap, item.variant as u16, "weapon")? as u8;
        }
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
    // A tree's stored species is a wood-material index — remap it like any other.
    let old_trees = std::mem::take(&mut sim.trees);
    for (pos, species) in old_trees {
        sim.trees.insert(pos, remap_one(&mat_remap, species, "material")?);
    }
    // Each dwarf remembers a favorite material and crop by raws index — remap
    // those too, so a registry that was reordered (or the wood species added)
    // between save and load doesn't leave a dwarf fond of the wrong thing (or,
    // if the registry shrank, index out of bounds when their tale is told).
    for d in &mut sim.dwarves {
        d.favorite_material = remap_one(&mat_remap, d.favorite_material, "material")?;
        d.favorite_crop = remap_one(&plant_remap, d.favorite_crop, "plant")?;
    }
    sim.rebuild_caches();
    Ok(sim)
}
