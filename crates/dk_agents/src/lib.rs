//! The living simulation: dwarves, needs, designations, jobs, farming,
//! workshops, hauling, happiness.
//!
//! Engine-agnostic and fully deterministic — `Sim::step()` advances one fixed
//! tick, so the whole game loop can run (and be tested) headlessly.

use anyhow::{Context, Result};
use dk_core::{Calendar, Season, DAYS_PER_SEASON, SEASONS_PER_YEAR, TICKS_PER_DAY};
use dk_raws::{MaterialCategory, Raws};
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
/// How close a raider must be before a war dog charges it.
pub const WAR_DOG_ENGAGE: u32 = 18;
/// Extra damage a soldier deals when wielding a forged weapon.
pub const WEAPON_DAMAGE: i16 = 10;
/// Damage a suit of armor turns aside from each blow that lands on its wearer
/// (a struck blow always does at least 1, so armor never fully negates a hit).
pub const ARMOR_REDUCTION: i16 = 7;
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
/// Days in the lunar cycle; a werebeast transforms during the first few nights.
pub const WERE_MOON_CYCLE: u64 = 28;
/// Nights of each cycle the moon is full (a cursed dwarf is a beast).
pub const WERE_MOON_NIGHTS: u64 = 2;
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
    /// A bed built at the mason's workshop. `stuff` = material index. A bulky
    /// trade good, and its owner sleeps better than a dwarf on the bare stone.
    Bed,
    /// Sewn clothes — the fort's woven cloth made into something to wear.
    /// `stuff` unused. A fine trade good, and a well-dressed dwarf is content.
    Clothes,
    /// A felled log. `stuff` unused. The carpenter's raw stock, and a modest
    /// trade good.
    Log,
    /// A barrel worked from a log at the carpenter's shop. `stuff` unused. A
    /// fine wooden trade good.
    Barrel,
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

/// Gem varieties, indexed by `Item::stuff` for RoughGem/CutGem.
pub const GEM_KINDS: [(&str, [u8; 3]); 6] = [
    ("ruby", [200, 40, 60]),
    ("emerald", [40, 190, 90]),
    ("sapphire", [50, 90, 210]),
    ("amethyst", [160, 80, 200]),
    ("topaz", [220, 180, 60]),
    ("opal", [210, 220, 230]),
];

pub fn gem_name(idx: u16) -> &'static str {
    GEM_KINDS.get(idx as usize).map(|(n, _)| *n).unwrap_or("gem")
}

pub fn gem_color(idx: u16) -> [u8; 3] {
    GEM_KINDS.get(idx as usize).map(|(_, c)| *c).unwrap_or([180, 180, 200])
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
    /// Craftsdwarfship: 0 = ordinary, up to 5 = a masterwork. Set from the
    /// maker's skill; raises the item's worth.
    pub quality: u8,
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
    /// A working dog: not raised for meat, but can be trained to guard the
    /// fort and fight off raiders.
    Dog,
}

impl AnimalKind {
    pub fn name(self) -> &'static str {
        match self {
            AnimalKind::Cow => "cow",
            AnimalKind::Sheep => "sheep",
            AnimalKind::Dog => "dog",
        }
    }

    /// Meals yielded when butchered as an adult.
    pub fn meat_yield(self) -> usize {
        match self {
            AnimalKind::Cow => 5,
            AnimalKind::Sheep => 3,
            AnimalKind::Dog => 1,
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
    /// Cut a rough gem into a brilliant one.
    CutGem,
    /// Forge a metal bar into a weapon.
    ForgeWeapon,
    /// Smelt a stone/ore boulder down into a refined metal bar.
    Smelt,
    /// Forge a metal bar into a suit of armor.
    ForgeArmor,
    /// Work a stone boulder into a piece of furniture (a bed).
    MakeFurniture,
    /// Sew a bolt of cloth into clothes.
    SewClothes,
    /// Work a log into a barrel at the carpenter's shop.
    MakeBarrel,
    /// Carve a stone boulder into a statue at the mason's workshop.
    CarveStatue,
    /// Work a log into a musical instrument at the carpenter's shop.
    MakeInstrument,
    /// Tan a raw hide into leather at the tanner's shop.
    TanHide,
    /// Melt a boulder into a piece of blown glass.
    MakeGlass,
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
    /// Shelter in a burrow while the alarm sounds.
    Shelter { spot: Pos, path: Vec<Pos> },
    /// Walk to a marked tree and fell it for a log.
    Chop { tree: Pos, path: Vec<Pos>, progress: u16 },
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
            Task::Craft { kind: CraftKind::CutGem, .. } => "cutting gems",
            Task::Craft { kind: CraftKind::ForgeWeapon, .. } => "forging a weapon",
            Task::Craft { kind: CraftKind::Smelt, .. } => "smelting",
            Task::Craft { kind: CraftKind::ForgeArmor, .. } => "forging armor",
            Task::Craft { kind: CraftKind::MakeFurniture, .. } => "building furniture",
            Task::Craft { kind: CraftKind::SewClothes, .. } => "sewing clothes",
            Task::Craft { kind: CraftKind::MakeBarrel, .. } => "making a barrel",
            Task::Craft { kind: CraftKind::CarveStatue, .. } => "carving a statue",
            Task::Craft { kind: CraftKind::MakeInstrument, .. } => "making an instrument",
            Task::Craft { kind: CraftKind::TanHide, .. } => "tanning leather",
            Task::Chop { .. } => "chopping wood",
            Task::DrinkWell { .. } => "drawing water",
            Task::Craft { kind: CraftKind::MakeGlass, .. } => "blowing glass",
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
    /// Pieces of furniture (beds) built at the mason's workshop.
    pub furniture_made: u32,
    /// Sets of clothes sewn at the clothier's shop.
    pub clothes_sewn: u32,
    /// Trees felled by woodcutters.
    pub trees_felled: u32,
    /// Barrels worked from logs at the carpenter's shop.
    pub barrels_made: u32,
    /// Statues carved at the mason's workshop.
    pub statues_carved: u32,
    /// Instruments crafted at the carpenter's shop.
    pub instruments_made: u32,
    /// Hides tanned into leather at the tanner's shop.
    pub leather_tanned: u32,
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
    weapons: bool,
    glass: bool,
    bars: bool,
    armor: bool,
    furniture: bool,
    clothes: bool,
    barrels: bool,
    statues: bool,
    instruments: bool,
    leather: bool,
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
    let base = match item.kind {
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
        ItemKind::RoughGem => 12,
        // A cut gem is the fort's finest legitimate trade good.
        ItemKind::CutGem => 70,
        // A forged weapon: worth several times its metal, and it arms a soldier.
        ItemKind::Weapon => raws.materials.get(item.stuff).value * 10 + 20,
        // Blown glass: the fort's finest ordinary trade good.
        ItemKind::Glass => 85,
        // A metal bar: refined stock, worth several times its raw ore.
        ItemKind::Bar => raws.materials.get(item.stuff).value * 8 + 10,
        // A suit of armor: costly plate, dearer than a weapon of the same metal.
        ItemKind::Armor => raws.materials.get(item.stuff).value * 10 + 30,
        // A bed: bulky furniture, a solid trade good in its own right.
        ItemKind::Bed => raws.materials.get(item.stuff).value * 6 + 20,
        // Sewn clothes: worth well more than the bolt of cloth they're made of.
        ItemKind::Clothes => 40,
        // A felled log: cheap raw wood.
        ItemKind::Log => 10,
        // A barrel: a fine wooden good, worth several logs.
        ItemKind::Barrel => 45,
        // A statue: a precious work of art, the fort's finest furnishing.
        ItemKind::Statue => raws.materials.get(item.stuff).value * 15 + 40,
        // A musical instrument: a fine crafted good.
        ItemKind::Instrument => 55,
        // A raw hide: cheap until it's tanned.
        ItemKind::Hide => 6,
        // Tanned leather: a fine, renewable trade good.
        ItemKind::Leather => 30,
    };
    // Craftsdwarfship raises the worth: a masterwork (tier 5) is worth 3.5x.
    base + base * item.quality as u32 / 2
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
    pub temples: Vec<Rect>,
    /// Hospital zones: wounded dwarves rest here and mend faster.
    pub hospitals: Vec<Rect>,
    /// Barracks zones: enlisted soldiers spar here to hone their fighting.
    pub barracks: Vec<Rect>,
    /// Burrow zones: safe rooms civilians retreat to when the alarm sounds.
    pub burrows: Vec<Rect>,
    /// Whether the civilian alarm is sounded (retreat to the burrows).
    pub alarm: bool,
    /// Library zones: with one, the fort's scholars set down treatises.
    pub library: Vec<Rect>,
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
    /// Trees standing on the surface. Passable, but a woodcutter can fell one
    /// (designate Chop) for a log. Empty unless a map is planted with them.
    pub trees: BTreeSet<Pos>,
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
        if let Some(spring) = natural_spring(&map) {
            water.springs.insert(spring);
        }

        Sim {
            map,
            dwarves,
            items: Vec::new(),
            stockpiles: Vec::new(),
            pastures: Vec::new(),
            taverns: Vec::new(),
            temples: Vec::new(),
            hospitals: Vec::new(),
            barracks: Vec::new(),
            burrows: Vec::new(),
            alarm: false,
            library: Vec::new(),
            treatises: Vec::new(),
            fisheries: Vec::new(),
            animals: Vec::new(),
            buildings: Vec::new(),
            farms: BTreeMap::new(),
            designations: BTreeMap::new(),
            trees: BTreeSet::new(),
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
                    // Engrave a solid wall that isn't already engraved.
                    DesignationKind::Smooth => {
                        tile.is_solid() && !self.engravings.contains_key(&p)
                    }
                    // Fell a tree standing on this tile.
                    DesignationKind::Chop => self.trees.contains(&p),
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
        self.log_event(if now {
            format!("{name} takes up arms as a soldier.")
        } else {
            format!("{name} lays down their arms.")
        });
        Some(now)
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
            // Dogs are companions, not livestock — never marked for the block.
            .filter(|(_, a)| a.alive && !a.marked && a.kind != AnimalKind::Dog)
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
                            quality: 0,
                        });
                        placed += 1;
                    }
                }
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

    /// Is there a tree standing on this tile?
    pub fn tree_at(&self, p: Pos) -> bool {
        self.trees.contains(&p)
    }

    /// Scatter `count` trees across walkable surface tiles. Deterministic given
    /// the sim's rng; called at embark (like the starting dogs) so headless
    /// tests that don't ask for a forest stay byte-identical.
    pub fn plant_trees(&mut self, count: usize) {
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
            // Keep trees off occupied ground so they never block a workshop,
            // farm, stockpile tile, or another tree.
            if self.trees.contains(&p)
                || self.building_at(p).is_some()
                || self.farms.contains_key(&p)
                || self.map.water_at(p) > 0
            {
                continue;
            }
            self.trees.insert(p);
            placed += 1;
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
        // Drop whatever the hero was doing (releasing any reservation / carried
        // item), since as the player their task never advances to completion.
        self.abandon_task(hero);
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
        // Companions journey on with the hero; nobody else does.
        let mut companions: Vec<Dwarf> = self
            .dwarves
            .iter()
            .enumerate()
            .filter(|&(j, d)| j != hero && d.alive && d.follower)
            .map(|(_, d)| d.clone())
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
        self.burrows.clear();
        self.alarm = false;
        self.library.clear();
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
            if self.dwarves[i].follower && self.player.is_some() {
                self.follow_hero(i);
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
        if self.fallen_at.is_none() && self.player.is_none() && self.alive_dwarves() == 0 {
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
        let vital = part.kind.vital();
        self.log_event(format!("A war {kind} savages {def_name}'s {}!", part_kind.name()));
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
        let vital = part.kind.vital();
        self.log_event(format!("A weapon trap tears into {name}!"));
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
                    quality: 0,
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
        let mut pending_weapons = 0usize;
        let mut pending_glass = 0usize;
        let mut pending_bars = 0usize;
        let mut pending_armor = 0usize;
        let mut pending_furniture = 0usize;
        let mut pending_clothes = 0usize;
        let mut pending_barrels = 0usize;
        let mut pending_statues = 0usize;
        let mut pending_instruments = 0usize;
        let mut pending_leather = 0usize;
        for d in &self.dwarves {
            if d.alive {
                match d.task {
                    Task::Craft { kind: CraftKind::Brew, .. } => pending_brews += 1,
                    Task::Craft { kind: CraftKind::Cook, .. } => pending_cooks += 1,
                    Task::Craft { kind: CraftKind::Stonecraft, .. } => pending_crafts += 1,
                    Task::Craft { kind: CraftKind::ForgeWeapon, .. } => pending_weapons += 1,
                    Task::Craft { kind: CraftKind::MakeGlass, .. } => pending_glass += 1,
                    Task::Craft { kind: CraftKind::Smelt, .. } => pending_bars += 1,
                    Task::Craft { kind: CraftKind::ForgeArmor, .. } => pending_armor += 1,
                    Task::Craft { kind: CraftKind::MakeFurniture, .. } => pending_furniture += 1,
                    Task::Craft { kind: CraftKind::SewClothes, .. } => pending_clothes += 1,
                    Task::Craft { kind: CraftKind::MakeBarrel, .. } => pending_barrels += 1,
                    Task::Craft { kind: CraftKind::CarveStatue, .. } => pending_statues += 1,
                    Task::Craft { kind: CraftKind::MakeInstrument, .. } => pending_instruments += 1,
                    Task::Craft { kind: CraftKind::TanHide, .. } => pending_leather += 1,
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
        let weapons_on_hand = self
            .items
            .iter()
            .filter(|it| it.active() && it.kind == ItemKind::Weapon)
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
                // Arm and armor the soldiers: the forge works smelted bars into
                // weapons and plate until every enlistee has both. Bars feed both
                // lines, so each checks there's a bar free of the other's claims.
                let bars = self.count_kind(ItemKind::Bar);
                let armor_on_hand = self.count_kind(ItemKind::Armor);
                let claimed_bars = pending_weapons + pending_armor;
                let want_weapons = has_forge
                    && bars > claimed_bars
                    && weapons_on_hand + pending_weapons < soldiers;
                let want_armor = has_forge
                    && bars > claimed_bars
                    && armor_on_hand + pending_armor < soldiers;
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
                // one for every citizen (plus a couple over to trade).
                let beds = self.count_kind(ItemKind::Bed);
                let want_furniture = has_mason
                    && boulders > 4 + pending_crafts + pending_furniture
                    && beds + pending_furniture < alive + 2;
                // Adorn the halls: carve a few statues from surplus stone. A
                // handful beautifies the whole fort, so the target is small.
                let statues = self.count_kind(ItemKind::Statue);
                let want_statues = has_mason
                    && boulders > 4 + pending_crafts + pending_furniture + pending_statues
                    && statues + pending_statues < 3;
                // Sew clothes from any cloth on hand until the fort is dressed
                // (plus a couple of sets over to trade).
                let cloth = self.count_kind(ItemKind::Cloth);
                let clothes = self.count_kind(ItemKind::Clothes);
                let want_clothes = has_clothier
                    && cloth > pending_clothes
                    && clothes + pending_clothes < alive + 2;
                // Work logs into barrels while there's spare wood on hand.
                let logs = self.count_kind(ItemKind::Log);
                let barrels = self.count_kind(ItemKind::Barrel);
                let want_barrels = has_carpenter
                    && logs > pending_barrels + pending_instruments
                    && barrels + pending_barrels < alive + 2;
                // Craft an instrument or two so the fort can make music.
                let instruments = self.count_kind(ItemKind::Instrument);
                let want_instruments = has_carpenter
                    && logs > pending_barrels + pending_instruments
                    && instruments + pending_instruments < 2;
                // Tan any hides on hand into leather at the tanner's shop.
                let hides = self.count_kind(ItemKind::Hide);
                let want_leather = has_tanner && hides > pending_leather;
                let wants = Wants {
                    drinks: want_drinks,
                    meals: want_meals,
                    crafts: want_crafts,
                    weapons: want_weapons,
                    glass: want_glass,
                    bars: want_bars,
                    armor: want_armor,
                    furniture: want_furniture,
                    clothes: want_clothes,
                    barrels: want_barrels,
                    statues: want_statues,
                    instruments: want_instruments,
                    leather: want_leather,
                };
                match self.assign_one(i, raws, wants) {
                    Some(CraftKind::Brew) => pending_brews += 1,
                    Some(CraftKind::Cook) => pending_cooks += 1,
                    Some(CraftKind::Stonecraft) => pending_crafts += 1,
                    Some(CraftKind::ForgeWeapon) => pending_weapons += 1,
                    Some(CraftKind::MakeGlass) => pending_glass += 1,
                    Some(CraftKind::Smelt) => pending_bars += 1,
                    Some(CraftKind::ForgeArmor) => pending_armor += 1,
                    Some(CraftKind::MakeFurniture) => pending_furniture += 1,
                    Some(CraftKind::SewClothes) => pending_clothes += 1,
                    Some(CraftKind::MakeBarrel) => pending_barrels += 1,
                    Some(CraftKind::CarveStatue) => pending_statues += 1,
                    Some(CraftKind::MakeInstrument) => pending_instruments += 1,
                    Some(CraftKind::TanHide) => pending_leather += 1,
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
            // Chopping: the woodcutter stands on the (passable) tree tile
            // itself, rather than working a wall from an adjacent square.
            if des.kind == DesignationKind::Chop {
                if self.regions.id(target) == my_region {
                    consider(target.manhattan(dwarf_pos), Cand::Chop { tree: target }, &mut best);
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
        if w.drinks {
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
            if let Some((shop, input)) = self.craft_forge_pair(dwarf_pos, my_region) {
                let d = self.items[input].pos.manhattan(dwarf_pos);
                consider(d, Cand::Craft { shop, input, kind: CraftKind::ForgeWeapon }, &mut best);
            }
        }
        // Armoring: forge a metal bar into plate to protect the soldiers. Shares
        // the forge and its bar stock with weaponsmithing.
        if w.armor {
            if let Some((shop, input)) = self.craft_forge_pair(dwarf_pos, my_region) {
                let d = self.items[input].pos.manhattan(dwarf_pos);
                consider(d, Cand::Craft { shop, input, kind: CraftKind::ForgeArmor }, &mut best);
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
        // Instrument-making: work a log into an instrument at the carpenter's shop.
        if w.instruments {
            if let Some((shop, input)) = self.craft_carpenter_pair(dwarf_pos, my_region) {
                let d = self.items[input].pos.manhattan(dwarf_pos);
                consider(d, Cand::Craft { shop, input, kind: CraftKind::MakeInstrument }, &mut best);
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
    /// forge's stock now — raw stone must be smelted at a smelter first.
    fn craft_forge_pair(&self, near: Pos, region: u32) -> Option<(Pos, usize)> {
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
            .min_by_key(|(_, it)| it.pos.manhattan(near))
            .map(|(i, _)| i)?;
        Some((shop.pos, input))
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

    /// Whether soldier `i` is drawing one of the fort's forged weapons: the
    /// armory arms enlistees in index order, up to the number of weapons.
    fn is_armed(&self, i: usize) -> bool {
        if !self.dwarves[i].soldier || !self.dwarves[i].alive {
            return false;
        }
        let weapons = self
            .items
            .iter()
            .filter(|it| it.active() && it.kind == ItemKind::Weapon)
            .count();
        if weapons == 0 {
            return false;
        }
        // Rank among living soldiers by index; armed if within the weapon count.
        let rank = self
            .dwarves
            .iter()
            .take(i)
            .filter(|d| d.alive && d.faction == Faction::Fort && d.soldier)
            .count();
        rank < weapons
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

    /// Whether soldier `i` is wearing one of the fort's forged suits of armor.
    fn is_armored(&self, i: usize) -> bool {
        if !self.dwarves[i].soldier || !self.dwarves[i].alive {
            return false;
        }
        let armor = self
            .items
            .iter()
            .filter(|it| it.active() && it.kind == ItemKind::Armor)
            .count();
        if armor == 0 {
            return false;
        }
        let rank = self
            .dwarves
            .iter()
            .take(i)
            .filter(|d| d.alive && d.faction == Faction::Fort && d.soldier)
            .count();
        rank < armor
    }

    /// Is this creature carrying a forged weapon in hand? (Used for the lone
    /// adventurer, who wields looted blades rather than the fortress armory.)
    fn carries_weapon(&self, i: usize) -> bool {
        self.items.iter().any(|it| {
            it.active() && it.kind == ItemKind::Weapon && it.state == ItemState::Carried { by: i }
        })
    }

    /// Whether citizen `i` has a bed to sleep in: the fort's beds are claimed by
    /// its citizens in index order, one each, just as the armory issues weapons
    /// and armor. A dwarf with a bed rests more soundly than one on bare stone.
    fn sleeps_in_bed(&self, i: usize) -> bool {
        if self.dwarves[i].faction != Faction::Fort || !self.dwarves[i].alive {
            return false;
        }
        let beds = self
            .items
            .iter()
            .filter(|it| it.active() && it.kind == ItemKind::Bed)
            .count();
        if beds == 0 {
            return false;
        }
        let rank = self
            .dwarves
            .iter()
            .take(i)
            .filter(|d| d.alive && d.faction == Faction::Fort)
            .count();
        rank < beds
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
            self.melee(i, enemy);
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

        // Soldiers proactively hunt: march on the nearest reachable hostile
        // instead of standing at their post.
        if self.dwarves[i].soldier {
            let my_pos = self.dwarves[i].pos;
            let my_region = self.regions.id(my_pos);
            let quarry = self
                .dwarves
                .iter()
                .enumerate()
                .filter(|(_, d)| {
                    d.alive
                        && d.faction == Faction::Hostile
                        && self.regions.id(d.pos) == my_region
                })
                .min_by_key(|(_, d)| d.pos.manhattan(my_pos))
                .map(|(j, _)| j);
            if let Some(q) = quarry {
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
                    // A dwarf with a proper bed is refreshed sooner than one
                    // dropping to sleep on the bare stone floor.
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
                if !self.trees.contains(&tree) {
                    self.abandon_task(i);
                    return;
                }
                let speed = 1 + self.dwarves[i].skill_level(Skill::Mining) as u16 / 2;
                let progress = progress + speed;
                if progress < CHOP_WORK {
                    self.dwarves[i].task = Task::Chop { tree, path, progress };
                    return;
                }
                // Timber! The tree falls, leaving a log where it stood.
                self.trees.remove(&tree);
                self.designations.remove(&tree);
                self.spawn_item(ItemKind::Log, 0, tree);
                self.stats.trees_felled += 1;
                self.add_xp(i, Skill::Mining, 25);
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
                            | CraftKind::Weave
                            | CraftKind::CutGem
                            | CraftKind::ForgeWeapon
                            | CraftKind::MakeGlass
                            | CraftKind::Smelt
                            | CraftKind::ForgeArmor
                            | CraftKind::MakeFurniture
                            | CraftKind::SewClothes
                            | CraftKind::MakeBarrel
                            | CraftKind::CarveStatue
                            | CraftKind::MakeInstrument
                            | CraftKind::TanHide => Skill::Crafting,
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
                                    self.spawn_quality_item(ItemKind::Craft, stuff, shop, q);
                                }
                                self.push_thought(i, ThoughtKind::CookedMeal); // a job well done
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
                                // The bar's metal carries into the blade.
                                self.stats.weapons_forged += 1;
                                self.spawn_quality_item(ItemKind::Weapon, stuff, shop, q);
                                self.push_thought(i, ThoughtKind::CookedMeal);
                            }
                            CraftKind::Smelt => {
                                // The boulder's material carries into the bar.
                                self.stats.bars_smelted += 1;
                                self.spawn_quality_item(ItemKind::Bar, stuff, shop, q);
                                self.push_thought(i, ThoughtKind::CookedMeal);
                            }
                            CraftKind::ForgeArmor => {
                                // The bar's metal carries into the plate.
                                self.stats.armor_forged += 1;
                                self.spawn_quality_item(ItemKind::Armor, stuff, shop, q);
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
    fn follow_hero(&mut self, i: usize) {
        self.tick_vitals(i);
        if !self.dwarves[i].alive {
            return;
        }
        // Strike first if an enemy is in reach.
        if let Some(enemy) = self.adjacent_enemy(i) {
            self.melee(i, enemy);
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

    fn update_hostile(&mut self, i: usize) {
        self.tick_vitals(i);
        if !self.dwarves[i].alive {
            return;
        }
        if let Some(enemy) = self.adjacent_enemy(i) {
            self.melee(i, enemy);
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
        // A forgotten beast's blow lands with terrible force.
        let base = if self.dwarves[attacker].beast {
            self.rng.gen_range(25..=55) as i16
        } else {
            self.rng.gen_range(8..=20) as i16
        };
        // A trained fighter puts more weight behind the blow; a forged weapon
        // in hand makes it far deadlier than bare fists. A soldier draws from
        // the armory; a lone adventurer wields whatever blade they carry.
        let armed = self.is_armed(attacker)
            || (self.player == Some(attacker) && self.carries_weapon(attacker));
        // A soldier in forged plate turns aside much of the blow — but a hit
        // that lands always draws at least a little blood.
        let armored = self.is_armored(defender);
        let dmg = (base
            + fighting_bonus(self.dwarves[attacker].skill_level(Skill::Fighting))
            + if armed { WEAPON_DAMAGE } else { 0 }
            - if armored { ARMOR_REDUCTION } else { 0 })
        .max(1);
        let bleed = self.rng.gen_range(1..=3) as u8;
        // Drawing blood teaches the trade: every landed blow hones prowess.
        self.add_xp(attacker, Skill::Fighting, 6);

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
        self.items.push(Item {
            kind,
            stuff,
            name,
            pos,
            state: ItemState::OnGround,
            reserved_by: None,
            consumed: false,
            quality: 0,
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
        }
        // Digging the wall away destroys any scene engraved on it.
        self.engravings.remove(&target);
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
            // A glint in the stone: rarely, the pick strikes a gem.
            if self.rng.gen_ratio(1, 22) {
                let gem = self.rng.gen_range(0..GEM_KINDS.len()) as u16;
                self.spawn_item(ItemKind::RoughGem, gem, drop_at);
                self.stats.gems_found += 1;
                let name = self.dwarves[i].name.clone();
                self.log_event(format!("{name} strikes a rough {}!", gem_name(gem)));
            }
        }
        self.add_xp(i, Skill::Mining, 20);
        self.dwarves[i].task = Task::Idle { wander_cd: 2 };
    }

    /// Centralized cleanup: release every reservation the current task holds,
    /// drop anything carried, and go idle. Safe to call in any state.
    fn abandon_task(&mut self, i: usize) {
        match self.dwarves[i].task.clone() {
            Task::Mine { target, .. } | Task::Chop { tree: target, .. } => {
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
            | Task::Fight { .. }
            | Task::Tantrum { .. }
            | Task::Sulk { .. }
            | Task::Relax { .. }
            | Task::Fish { .. }
            | Task::Pray { .. }
            | Task::Recover { .. }
            | Task::Spar { .. }
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
        beast: false,
        soldier: false,
        last_prayer: 0,
        follower: false,
        vampire: false,
        last_fed: 0,
        werebeast: false,
        were_form: false,
    }
}

// ------------------------------------------------------------------- saves

const SAVE_MAGIC: u32 = 0x444B_5331; // "DKS1"
const SAVE_VERSION: u32 = 53;

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
            ItemKind::Boulder
            | ItemKind::Artifact
            | ItemKind::Craft
            | ItemKind::Weapon
            | ItemKind::Glass
            | ItemKind::Bar
            | ItemKind::Armor
            | ItemKind::Bed
            | ItemKind::Statue => remap_one(&mat_remap, item.stuff, "material")?,
            // These carry no raws index (dwarf index, gem type, or nothing).
            ItemKind::Corpse
            | ItemKind::Wool
            | ItemKind::Cloth
            | ItemKind::Clothes
            | ItemKind::Log
            | ItemKind::Barrel
            | ItemKind::Instrument
            | ItemKind::Hide
            | ItemKind::Leather
            | ItemKind::RoughGem
            | ItemKind::CutGem => item.stuff,
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
