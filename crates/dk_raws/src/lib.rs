//! Data-driven content ("raws"). The engine knows mechanisms; these files
//! supply everything else. Phase 0: materials only.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MaterialCategory {
    Soil,
    Sedimentary,
    Igneous,
    Metamorphic,
    Ore,
    /// Wood species. Never placed in the ground by mapgen (which only draws
    /// from the geological categories) — these exist purely as the material of
    /// logs and the wooden goods worked from them.
    Wood,
    /// Adamantine — the deep, precious metal. Never placed by ordinary mapgen;
    /// seeded only in deep spires that, dug too greedily, breach the underworld.
    Adamantine,
    /// A refined alloy — steel and its like. Never in the ground; made at the
    /// smelter from ore and flux, so mapgen never queries this category.
    Alloy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaterialDef {
    pub id: String,
    pub name: String,
    pub category: MaterialCategory,
    /// Display color, sRGB 0-255.
    pub color: [u8; 3],
    /// Relative trade value multiplier.
    pub value: u32,
    /// How this material fights and defends. Defaulted so existing raws that
    /// predate combat still load — a material with no stats is treated as a
    /// dull, middling stone, fine for a wall and useless for a blade.
    #[serde(default)]
    pub combat: CombatStats,
    /// A flux stone (limestone, marble, dolomite, chalk): consumed with iron at
    /// the smelter to make steel. Defaulted false for ordinary rock.
    #[serde(default)]
    pub is_flux: bool,
}

/// A material's mechanical properties, as they matter in a fight. A pared-down
/// stand-in for Dwarf Fortress's material science: DF drives combat off shear
/// and impact yield/fracture, density, and a per-material sharpness multiplier
/// (1x for metals, 2x obsidian, 10x adamantine). These three axes capture the
/// same shape — a keen edge, a heavy head, and metal that beats lesser metal —
/// without the fragile version-specific formulas.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct CombatStats {
    /// Edge multiplier for cutting and stabbing. Dwarf Fortress's own numbers:
    /// 1.0 for metal, 1.5 glass, 2.0 obsidian, 10.0 adamantine — and near-zero
    /// for wood and stone, which hold no edge at all.
    pub sharpness: f32,
    /// Grams per cubic centimetre. The mass behind a blunt blow, and the weight
    /// that helps a suit of armour turn one aside.
    pub density: f32,
    /// Resistance to being cut through or dented — DF's shear and impact yield,
    /// rolled into one. Steel beats iron beats bronze beats copper beats bone
    /// beats wood, and this is the number that says so.
    pub hardness: f32,
}

impl Default for CombatStats {
    fn default() -> Self {
        // Dull stone: no edge, heavy, middling hardness.
        CombatStats { sharpness: 0.1, density: 2.6, hardness: 20.0 }
    }
}

/// How a weapon hurts. Dwarf Fortress's three practical melee classes: an edge
/// cuts, a point punches through, a blunt head crushes. Each meets armour and
/// flesh differently, which is the whole reason the class matters. Lives here so
/// a data-defined `WeaponDef` can name it; the sim re-exports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DamageType {
    /// Slashing. Cuts through flesh with the material's edge; a keener blade
    /// bites deeper, and a lesser metal is turned by a better armour.
    Edge,
    /// Stabbing. An edge attack, but narrow — it punches a small deep wound,
    /// which is how a spear finds an organ a slash would only score.
    Pierce,
    /// Crushing. Ignores the edge and drives force through the armour into the
    /// flesh, breaking bone the armour never stopped. Beaten by weight, not
    /// sharpness.
    Blunt,
}

/// A crop that can be farmed, then eaten raw-ish (cooked) or brewed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlantDef {
    pub id: String,
    pub name: String,
    /// Display color, sRGB 0-255.
    pub color: [u8; 3],
    /// In-game days from planting to harvestable.
    pub grow_days: u32,
    /// Season indices it grows in (0 spring, 1 summer, 2 autumn, 3 winter).
    pub seasons: Vec<u8>,
    /// Can a still turn it into drink?
    pub brewable: bool,
}

impl PlantDef {
    pub fn grows_in(&self, season_index: u8) -> bool {
        self.seasons.contains(&season_index)
    }
}

/// All loaded plants, indexed by a stable u16 handle items/farms store.
pub struct PlantRegistry {
    plants: Vec<PlantDef>,
    by_id: HashMap<String, u16>,
}

impl PlantRegistry {
    pub fn from_defs(plants: Vec<PlantDef>) -> Result<Self> {
        anyhow::ensure!(!plants.is_empty(), "no plants defined");
        anyhow::ensure!(plants.len() < u16::MAX as usize, "too many plants");
        let mut by_id = HashMap::new();
        for (i, p) in plants.iter().enumerate() {
            if by_id.insert(p.id.clone(), i as u16).is_some() {
                anyhow::bail!("duplicate plant id: {}", p.id);
            }
        }
        Ok(Self { plants, by_id })
    }

    pub fn load_dir(dir: &Path) -> Result<Self> {
        let defs: Vec<PlantDef> = load_ron_dir(dir)?;
        Self::from_defs(defs).with_context(|| format!("loading plants from {}", dir.display()))
    }

    pub fn get(&self, index: u16) -> &PlantDef {
        &self.plants[index as usize]
    }

    pub fn index_of(&self, id: &str) -> Option<u16> {
        self.by_id.get(id).copied()
    }

    pub fn id_manifest(&self) -> Vec<String> {
        self.plants.iter().map(|p| p.id.clone()).collect()
    }

    pub fn len(&self) -> usize {
        self.plants.len()
    }

    pub fn is_empty(&self) -> bool {
        self.plants.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (u16, &PlantDef)> {
        self.plants.iter().enumerate().map(|(i, p)| (i as u16, p))
    }
}

/// The price of one kind of item, in a common coin, as
/// `material.value * mat_coeff + flat`. A worked craft is worth several times
/// its raw stone (`mat_coeff` high), a meal is a flat few coins (`mat_coeff`
/// zero). Quality is layered on top by the sim, not here.
///
/// The `kind` string matches an item-kind name the sim knows (`ItemKind::key`);
/// keeping it a bare string keeps this crate engine-agnostic — dk_raws prices
/// goods without knowing what an `ItemKind` is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KindPrice {
    pub kind: String,
    /// Multiplier on the item's material value. Zero for goods whose worth does
    /// not depend on their stuff (a meal, a bolt of cloth).
    pub mat_coeff: u32,
    /// Flat worth added on top, regardless of material.
    pub flat: u32,
}

/// The fort's price list and trade terms (data/economy/prices.ron). Lifts the
/// old hardcoded `item_value` constants and `TRADE_MARGIN` out of the sim so
/// they can be tuned and modded from data without a recompile.
///
/// Gems are the one deliberate exception: a rough/cut gem is priced by its
/// rarity tier in code (`gem_value`), not a material multiplier, so no
/// `KindPrice` row governs them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EconomyConfig {
    /// A caravan buys at a margin: an offer must beat the asked value by this
    /// ratio (they came a long way).
    pub trade_margin: f32,
    /// One row per priced item kind.
    pub kind_prices: Vec<KindPrice>,
}

impl EconomyConfig {
    /// The price row for a kind, by its `ItemKind::key` name.
    pub fn price(&self, kind: &str) -> Option<&KindPrice> {
        self.kind_prices.iter().find(|p| p.kind == kind)
    }
}

impl Default for EconomyConfig {
    /// The canonical shipped economy, in code. `data/economy/prices.ron` mirrors
    /// these exact numbers for the real game; tests and tools that build `Raws`
    /// without touching disk use this, and a parity test keeps the two in step.
    fn default() -> Self {
        // (kind, mat_coeff, flat) — see the old item_value match this replaced.
        // Order mirrors data/economy/prices.ron so the two compare equal (the
        // parity test relies on it): material-derived goods first, then flat.
        let rows: &[(&str, u32, u32)] = &[
            // material-derived
            ("Boulder", 3, 0),
            ("Craft", 12, 4),
            ("Weapon", 10, 20),
            ("Bar", 8, 10),
            ("Armor", 10, 30),
            ("Shield", 6, 20),
            ("Bed", 6, 20),
            ("Log", 2, 6),
            ("Statue", 15, 40),
            ("Artifact", 5, 50),
            // flat-priced
            ("Seed", 0, 3),
            ("Crop", 0, 5),
            ("Meal", 0, 8),
            ("Drink", 0, 8),
            ("Corpse", 0, 0),
            ("BodyPart", 0, 0),
            ("BoneCraft", 0, 10),
            ("Wool", 0, 4),
            ("Cloth", 0, 18),
            ("Glass", 0, 85),
            ("Clothes", 0, 40),
            ("Barrel", 0, 45),
            ("Bin", 0, 30),
            ("Instrument", 0, 55),
            ("Hide", 0, 6),
            ("Leather", 0, 30),
            ("Berry", 0, 4),
        ];
        EconomyConfig {
            trade_margin: 1.2,
            kind_prices: rows
                .iter()
                .map(|&(kind, mat_coeff, flat)| KindPrice { kind: kind.into(), mat_coeff, flat })
                .collect(),
        }
    }
}

/// Sprite-sheet configuration (data/tileset.ron). Optional: when absent
/// the renderer falls back to flat colored squares.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TilesetDef {
    /// Image path relative to the assets/ directory.
    pub image: String,
    pub tile_px: u32,
    pub columns: u32,
    pub rows: u32,
    /// Glyph name -> cell index (row-major from 0).
    pub glyphs: HashMap<String, usize>,
    /// Glyphs drawn in grayscale, to be tinted by material color at runtime.
    pub tinted: Vec<String>,
}

/// Everything loaded from `data/`. Passed into the simulation.
/// A gem variety — struck while mining, cut into a premium trade good. Priced by
/// its rarity `value_tier` (1 ornamental .. 6 the rarest), not a material
/// multiplier. Gems were a Rust const array; they are the first former-enum
/// content axis moved into data, chosen because the sim never matches on a gem
/// exhaustively — access is only these name/color/value lookups.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GemDef {
    pub id: String,
    pub name: String,
    /// Display color, sRGB 0-255.
    pub color: [u8; 3],
    /// Rarity tier 1 (ornamental) .. 6 (the rarest); scales the cut stone's worth.
    pub value_tier: u32,
}

/// All loaded gems, indexed by a stable u16 handle that `RoughGem`/`CutGem`
/// items store in `stuff`. Modelled on `MaterialRegistry`.
pub struct GemRegistry {
    gems: Vec<GemDef>,
    by_id: HashMap<String, u16>,
    /// Neutral stand-in for an out-of-range index, so a lookup never panics —
    /// the same graceful fallback the old `gem_*` helpers gave with `unwrap_or`.
    unknown: GemDef,
}

impl GemRegistry {
    pub fn from_defs(gems: Vec<GemDef>) -> Result<Self> {
        anyhow::ensure!(!gems.is_empty(), "no gems defined");
        anyhow::ensure!(gems.len() < u16::MAX as usize, "too many gems");
        let mut by_id = HashMap::new();
        for (i, g) in gems.iter().enumerate() {
            if by_id.insert(g.id.clone(), i as u16).is_some() {
                anyhow::bail!("duplicate gem id: {}", g.id);
            }
        }
        Ok(Self {
            gems,
            by_id,
            unknown: GemDef { id: "gem".into(), name: "gem".into(), color: [180, 180, 200], value_tier: 2 },
        })
    }

    pub fn get(&self, idx: u16) -> &GemDef {
        self.gems.get(idx as usize).unwrap_or(&self.unknown)
    }
    pub fn name(&self, idx: u16) -> &str {
        &self.get(idx).name
    }
    pub fn color(&self, idx: u16) -> [u8; 3] {
        self.get(idx).color
    }
    pub fn value_tier(&self, idx: u16) -> u32 {
        self.get(idx).value_tier
    }
    pub fn index_of(&self, id: &str) -> Option<u16> {
        self.by_id.get(id).copied()
    }
    pub fn id_manifest(&self) -> Vec<String> {
        self.gems.iter().map(|g| g.id.clone()).collect()
    }
    pub fn len(&self) -> usize {
        self.gems.len()
    }
    pub fn is_empty(&self) -> bool {
        self.gems.is_empty()
    }
}

/// The base game's 18 gems, in the order the old `GEM_KINDS` const defined them.
/// `data/gems/gems.ron` mirrors this exactly (a parity test guards them), and
/// keeping the order preserves every existing world seed's gem strikes. Test
/// fixtures that don't touch disk build a registry from this.
pub fn canonical_gems() -> Vec<GemDef> {
    let g = |id: &str, name: &str, color: [u8; 3], value_tier: u32| GemDef {
        id: id.into(),
        name: name.into(),
        color,
        value_tier,
    };
    vec![
        g("ruby", "ruby", [200, 40, 60], 5),
        g("emerald", "emerald", [40, 190, 90], 5),
        g("sapphire", "sapphire", [50, 90, 210], 5),
        g("amethyst", "amethyst", [160, 80, 200], 3),
        g("topaz", "topaz", [220, 180, 60], 3),
        g("opal", "opal", [210, 220, 230], 3),
        g("diamond", "diamond", [235, 240, 250], 6),
        g("garnet", "garnet", [150, 30, 45], 3),
        g("aquamarine", "aquamarine", [130, 210, 210], 3),
        g("citrine", "citrine", [232, 196, 92], 2),
        g("jade", "jade", [86, 176, 128], 3),
        g("onyx", "onyx", [44, 44, 52], 2),
        g("turquoise", "turquoise", [72, 200, 190], 2),
        g("lapis_lazuli", "lapis lazuli", [46, 76, 178], 3),
        g("malachite", "malachite", [34, 150, 92], 2),
        g("jasper", "jasper", [172, 84, 60], 2),
        g("agate", "agate", [192, 156, 126], 1),
        g("peridot", "peridot", [172, 210, 84], 2),
    ]
}

/// A weapon a fort can forge (or a raider can carry). Its `damage_type`, `heft`
/// (mass, driving blunt force), combat `verb`, and whether it is `ranged` were a
/// Rust enum's match arms; they are data now, so a mod can add a weapon. The
/// wielding material is separate — a weapon stores its metal in `Item::stuff` and
/// its kind (an index into this registry) in `Item::variant`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WeaponDef {
    pub id: String,
    pub name: String,
    pub damage_type: DamageType,
    /// A rough stand-in for DF's SIZE — heavier heads hit harder with blunt force.
    pub heft: f32,
    /// The combat-log verb: a sword slashes, a hammer smashes.
    pub verb: String,
    /// A ranged weapon (a crossbow) fires at a distance rather than closing to
    /// strike; up close it bashes. The forge makes ranged arms only for
    /// marksdwarves, and a general weaponsmithing job draws only from melee arms.
    #[serde(default)]
    pub ranged: bool,
}

/// All loaded weapons, indexed by a stable u16 handle a `Weapon` item stores in
/// `variant`. Modelled on `MaterialRegistry`.
pub struct WeaponRegistry {
    weapons: Vec<WeaponDef>,
    by_id: HashMap<String, u16>,
    unknown: WeaponDef,
}

impl WeaponRegistry {
    pub fn from_defs(weapons: Vec<WeaponDef>) -> Result<Self> {
        anyhow::ensure!(!weapons.is_empty(), "no weapons defined");
        anyhow::ensure!(weapons.len() < u16::MAX as usize, "too many weapons");
        let mut by_id = HashMap::new();
        for (i, w) in weapons.iter().enumerate() {
            if by_id.insert(w.id.clone(), i as u16).is_some() {
                anyhow::bail!("duplicate weapon id: {}", w.id);
            }
        }
        Ok(Self {
            weapons,
            by_id,
            unknown: WeaponDef {
                id: "fist".into(),
                name: "fist".into(),
                damage_type: DamageType::Blunt,
                heft: 0.5,
                verb: "strikes".into(),
                ranged: false,
            },
        })
    }

    pub fn get(&self, idx: u16) -> &WeaponDef {
        self.weapons.get(idx as usize).unwrap_or(&self.unknown)
    }
    pub fn name(&self, idx: u16) -> &str {
        &self.get(idx).name
    }
    pub fn damage_type(&self, idx: u16) -> DamageType {
        self.get(idx).damage_type
    }
    pub fn heft(&self, idx: u16) -> f32 {
        self.get(idx).heft
    }
    pub fn verb(&self, idx: u16) -> &str {
        &self.get(idx).verb
    }
    pub fn is_ranged(&self, idx: u16) -> bool {
        self.get(idx).ranged
    }
    pub fn index_of(&self, id: &str) -> Option<u16> {
        self.by_id.get(id).copied()
    }
    pub fn id_manifest(&self) -> Vec<String> {
        self.weapons.iter().map(|w| w.id.clone()).collect()
    }
    pub fn len(&self) -> usize {
        self.weapons.len()
    }
    pub fn is_empty(&self) -> bool {
        self.weapons.is_empty()
    }
    /// The melee weapon indices, in registry order — everything but the ranged
    /// arms. A general weaponsmithing job cycles through these, so keeping the
    /// base order preserves the forge's rng stream.
    pub fn melee_indices(&self) -> Vec<u16> {
        self.weapons
            .iter()
            .enumerate()
            .filter(|(_, w)| !w.ranged)
            .map(|(i, _)| i as u16)
            .collect()
    }
}

/// The base game's 6 weapons, in the order the old `WeaponKind::ALL` defined them
/// (sword, axe, spear, mace, hammer, crossbow) so a saved weapon's `variant`
/// index still names the same weapon. `data/weapons/weapons.ron` mirrors this.
pub fn canonical_weapons() -> Vec<WeaponDef> {
    let w = |id: &str, name: &str, damage_type: DamageType, heft: f32, verb: &str, ranged: bool| {
        WeaponDef { id: id.into(), name: name.into(), damage_type, heft, verb: verb.into(), ranged }
    };
    vec![
        w("sword", "sword", DamageType::Edge, 1.0, "slashes", false),
        w("axe", "axe", DamageType::Edge, 1.5, "hacks", false),
        w("spear", "spear", DamageType::Pierce, 1.1, "stabs", false),
        w("mace", "mace", DamageType::Blunt, 2.0, "bashes", false),
        w("hammer", "war hammer", DamageType::Blunt, 1.8, "smashes", false),
        w("crossbow", "crossbow", DamageType::Blunt, 1.3, "bashes", true),
    ]
}

/// A mod's identity card — its `mod.ron`. A single RON struct at the root of a
/// mod folder (not a `Vec` like the content files). The `id` is the stable key
/// and de-facto namespace; `version` is stamped into saves so a world knows
/// which mods made it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    /// Which game/content version this mod targets. Advisory for now.
    #[serde(default)]
    pub target_game_version: String,
    /// Mod ids this one expects to load after (a load-order hint). Recorded but
    /// not yet used to reorder — load order is alphabetical by folder this slice.
    #[serde(default)]
    pub load_after: Vec<String>,
    /// Material/plant ids this mod deliberately replaces. A duplicate id that is
    /// NOT declared here is a hard error (an accidental clash between mods), so an
    /// override is always intentional — never a silent last-wins clobber.
    #[serde(default)]
    pub overrides: Vec<String>,
}

/// Everything loaded from `data/` (and any mods). Passed into the simulation.
pub struct Raws {
    pub materials: MaterialRegistry,
    pub plants: PlantRegistry,
    pub gems: GemRegistry,
    pub weapons: WeaponRegistry,
    pub tileset: Option<TilesetDef>,
    pub economy: EconomyConfig,
    /// The active mods, in resolved load order (empty for an unmodded game).
    pub mods: Vec<ModManifest>,
}

impl Raws {
    /// Load the base game only. Convenience for tests and tools; the app uses
    /// `load_with_mods`.
    pub fn load(data_dir: &Path) -> Result<Self> {
        Self::load_with_mods(data_dir, &[])
    }

    /// Load the base `data/` directory, then layer mod folders on top in the
    /// given order. Materials and plants merge across roots by id: a mod ADDS new
    /// ids, and may REPLACE an existing id only if its manifest declares that id
    /// in `overrides`. An undeclared duplicate is a hard error naming both the mod
    /// and the source it clashed with — never a silent last-wins clobber. An
    /// override replaces in place, so the material's registry index (and every
    /// saved fort's reference to it) is unchanged. Tileset and economy are
    /// base-only for now.
    pub fn load_with_mods(base: &Path, mod_roots: &[std::path::PathBuf]) -> Result<Self> {
        let tileset_path = base.join("tileset.ron");
        let tileset = if tileset_path.is_file() {
            let text = std::fs::read_to_string(&tileset_path)
                .with_context(|| format!("reading {}", tileset_path.display()))?;
            Some(
                ron::from_str(&text)
                    .with_context(|| format!("parsing {}", tileset_path.display()))?,
            )
        } else {
            None
        };
        let economy_path = base.join("economy").join("prices.ron");
        let economy_text = std::fs::read_to_string(&economy_path)
            .with_context(|| format!("reading {}", economy_path.display()))?;
        let economy: EconomyConfig = ron::from_str(&economy_text)
            .with_context(|| format!("parsing {}", economy_path.display()))?;

        // Collect each root's defs with its provenance. Base is "core", always
        // first and declaring no overrides (it can't override anyone).
        let no_overrides = HashSet::new();
        let mut mat_sources: Vec<(String, HashSet<String>, Vec<MaterialDef>)> =
            vec![("core".into(), no_overrides.clone(), load_ron_dir(&base.join("materials"))?)];
        let mut plant_sources: Vec<(String, HashSet<String>, Vec<PlantDef>)> =
            vec![("core".into(), no_overrides.clone(), load_ron_dir(&base.join("plants"))?)];
        let mut gem_sources: Vec<(String, HashSet<String>, Vec<GemDef>)> =
            vec![("core".into(), no_overrides.clone(), load_ron_dir(&base.join("gems"))?)];
        let mut weapon_sources: Vec<(String, HashSet<String>, Vec<WeaponDef>)> =
            vec![("core".into(), no_overrides, load_ron_dir(&base.join("weapons"))?)];

        let mut mods = Vec::new();
        for root in mod_roots {
            let manifest_path = root.join("mod.ron");
            let text = std::fs::read_to_string(&manifest_path)
                .with_context(|| format!("reading mod manifest {}", manifest_path.display()))?;
            let manifest: ModManifest = ron::from_str(&text)
                .with_context(|| format!("parsing mod manifest {}", manifest_path.display()))?;
            let overrides: HashSet<String> = manifest.overrides.iter().cloned().collect();
            let mat_dir = root.join("materials");
            let mats = if mat_dir.is_dir() { load_ron_dir::<MaterialDef>(&mat_dir)? } else { Vec::new() };
            let plant_dir = root.join("plants");
            let plants = if plant_dir.is_dir() { load_ron_dir::<PlantDef>(&plant_dir)? } else { Vec::new() };
            let gem_dir = root.join("gems");
            let gems = if gem_dir.is_dir() { load_ron_dir::<GemDef>(&gem_dir)? } else { Vec::new() };
            let weapon_dir = root.join("weapons");
            let weapons = if weapon_dir.is_dir() { load_ron_dir::<WeaponDef>(&weapon_dir)? } else { Vec::new() };
            mat_sources.push((manifest.id.clone(), overrides.clone(), mats));
            plant_sources.push((manifest.id.clone(), overrides.clone(), plants));
            gem_sources.push((manifest.id.clone(), overrides.clone(), gems));
            weapon_sources.push((manifest.id.clone(), overrides, weapons));
            mods.push(manifest);
        }

        let material_defs = merge_by_id(mat_sources, |m: &MaterialDef| m.id.as_str(), "material")?;
        let plant_defs = merge_by_id(plant_sources, |p: &PlantDef| p.id.as_str(), "plant")?;
        let gem_defs = merge_by_id(gem_sources, |g: &GemDef| g.id.as_str(), "gem")?;
        let weapon_defs = merge_by_id(weapon_sources, |w: &WeaponDef| w.id.as_str(), "weapon")?;

        Ok(Raws {
            materials: MaterialRegistry::from_defs(material_defs)
                .context("building the material registry")?,
            plants: PlantRegistry::from_defs(plant_defs)
                .context("building the plant registry")?,
            gems: GemRegistry::from_defs(gem_defs).context("building the gem registry")?,
            weapons: WeaponRegistry::from_defs(weapon_defs)
                .context("building the weapon registry")?,
            tileset,
            economy,
            mods,
        })
    }

    /// The active mods as `(id, version)` pairs in load order — the analogue of
    /// `id_manifest` for save stamping. A fort save records this so it can tell,
    /// on load, whether the mods that made it are present.
    pub fn mod_stamp(&self) -> Vec<(String, String)> {
        self.mods.iter().map(|m| (m.id.clone(), m.version.clone())).collect()
    }

    /// A stable fingerprint of the WORLDGEN-relevant content: material
    /// (id, category) in registry order, then plant ids. Two raws sets with the
    /// same hash generate the same world from the same seed; a different hash
    /// means the seed would diverge, so the app rebuilds the world rather than
    /// loading a mismatched map.
    ///
    /// It hashes what worldgen's RNG actually consumes:
    /// - CATEGORY, not just ids — worldgen draws from `indices_in_category(...)`,
    ///   so re-tagging a stone shifts the stream even with the id unchanged;
    /// - registry ORDER — the draw is `ores[gen_range(0..ores.len())]`, so which
    ///   material sits at which index matters, and adding a material or loading
    ///   mods in a different order changes it.
    ///
    /// It deliberately does NOT hash the mod list, display colours, or trade
    /// values: a mod that only recolours or reprices an existing stone leaves
    /// worldgen identical, so its world must NOT be needlessly rebuilt. Which
    /// mods a save needs is recorded separately (fort-save mod stamping).
    pub fn content_hash(&self) -> u64 {
        // FNV-1a — small, dependency-free, and byte-stable across platforms
        // (unlike the std default hasher), so a save shared between machines
        // agrees on the fingerprint.
        const OFFSET: u64 = 0xcbf29ce484222325;
        const PRIME: u64 = 0x100000001b3;
        let mut h = OFFSET;
        let mut eat = |bytes: &[u8]| {
            for &b in bytes {
                h ^= b as u64;
                h = h.wrapping_mul(PRIME);
            }
        };
        eat(b"materials\0");
        for i in 0..self.materials.len() {
            let m = self.materials.get(i as u16);
            eat(m.id.as_bytes());
            eat(&[0, m.category as u8]);
        }
        eat(b"plants\0");
        for id in self.plants.id_manifest() {
            eat(id.as_bytes());
            eat(&[0]);
        }
        h
    }
}

/// Merge id'd content defs from several sources (base first, then mods in load
/// order) into one list. A later source may replace an id an earlier source
/// defined ONLY if it declared that id in its `overrides` set; an undeclared
/// duplicate is an error naming both sources. An override replaces the def in
/// place, so existing indices (and saved references to them) don't move.
fn merge_by_id<T>(
    sources: Vec<(String, HashSet<String>, Vec<T>)>,
    id_of: impl Fn(&T) -> &str,
    kind: &str,
) -> Result<Vec<T>> {
    let mut out: Vec<T> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut origin: HashMap<String, String> = HashMap::new();
    for (label, overrides, defs) in sources {
        for def in defs {
            let id = id_of(&def).to_string();
            if let Some(&i) = index.get(&id) {
                anyhow::ensure!(
                    overrides.contains(&id),
                    "mod \"{label}\" redefines {kind} \"{id}\" (already defined by \"{}\") \
                     without declaring it in `overrides`",
                    origin[&id]
                );
                out[i] = def;
                origin.insert(id, label.clone());
            } else {
                index.insert(id.clone(), out.len());
                origin.insert(id.clone(), label.clone());
                out.push(def);
            }
        }
    }
    Ok(out)
}

/// Worldgen draws a material from each of these geological categories, so each
/// must have at least one member. A mod can't remove a base material, but an
/// override that re-categorizes the last member of a category would empty it and
/// crash mapgen — this catches that at load, naming the empty category.
pub fn validate_required_categories(raws: &Raws) -> Result<()> {
    for cat in [
        MaterialCategory::Soil,
        MaterialCategory::Sedimentary,
        MaterialCategory::Igneous,
    ] {
        anyhow::ensure!(
            !raws.materials.indices_in_category(cat).is_empty(),
            "no {cat:?} materials remain — worldgen needs at least one \
             (a mod may have re-categorized the last one)"
        );
    }
    Ok(())
}

/// Read every `.ron` file in a directory; each holds a `Vec<T>`.
fn load_ron_dir<T: serde::de::DeserializeOwned>(dir: &Path) -> Result<Vec<T>> {
    let mut out: Vec<T> = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .with_context(|| format!("reading raws dir {}", dir.display()))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "ron"))
        .collect();
    entries.sort(); // deterministic load order
    for path in entries {
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let mut defs: Vec<T> = ron::from_str(&text)
            .with_context(|| format!("parsing {}", path.display()))?;
        out.append(&mut defs);
    }
    Ok(out)
}

/// All loaded materials, indexed by a stable u16 handle that tiles store.
pub struct MaterialRegistry {
    materials: Vec<MaterialDef>,
    by_id: HashMap<String, u16>,
    /// Handed back for out-of-range indices so the renderer can never panic on
    /// one. See `get`.
    unknown: MaterialDef,
}

impl MaterialRegistry {
    /// Build a registry from an in-memory list (used by tests and tools).
    pub fn from_defs(materials: Vec<MaterialDef>) -> Result<Self> {
        anyhow::ensure!(!materials.is_empty(), "no materials defined");
        anyhow::ensure!(materials.len() < u16::MAX as usize, "too many materials");
        let mut by_id = HashMap::new();
        for (i, m) in materials.iter().enumerate() {
            if by_id.insert(m.id.clone(), i as u16).is_some() {
                anyhow::bail!("duplicate material id: {}", m.id);
            }
        }
        Ok(Self {
            materials,
            by_id,
            unknown: MaterialDef {
                id: "unknown".into(),
                name: "unknown".into(),
                category: MaterialCategory::Soil,
                color: [120, 120, 120],
                value: 0,
                combat: CombatStats::default(),
                is_flux: false,
            },
        })
    }

    /// Load every `.ron` file in a directory. Each file holds a `Vec<MaterialDef>`.
    /// NOTE: indices are only stable while the raws are unchanged — anything
    /// persisted must store material *ids* (or a manifest) and remap on load.
    pub fn load_dir(dir: &Path) -> Result<Self> {
        let defs: Vec<MaterialDef> = load_ron_dir(dir)?;
        Self::from_defs(defs)
            .with_context(|| format!("loading materials from {}", dir.display()))
    }

    /// Ordered list of material ids — the manifest embedded in save files.
    pub fn id_manifest(&self) -> Vec<String> {
        self.materials.iter().map(|m| m.id.clone()).collect()
    }

    /// The material at `index`.
    ///
    /// Out-of-range asks — `NO_MATERIAL` (u16::MAX) above all, which every
    /// empty tile carries — yield a neutral stand-in rather than panicking.
    /// This is called from the renderer for every tile of every frame, and a
    /// bare `self.materials[i]` there turns one stray index into a hard crash
    /// of the whole game. A grey square is a bug you can see and report; a
    /// panic is a bug that ends the session.
    pub fn get(&self, index: u16) -> &MaterialDef {
        self.materials.get(index as usize).unwrap_or(&self.unknown)
    }

    /// Is this a real material, or would `get` hand back the stand-in?
    pub fn is_valid(&self, index: u16) -> bool {
        (index as usize) < self.materials.len()
    }

    pub fn index_of(&self, id: &str) -> Option<u16> {
        self.by_id.get(id).copied()
    }

    pub fn indices_in_category(&self, cat: MaterialCategory) -> Vec<u16> {
        self.materials
            .iter()
            .enumerate()
            .filter(|(_, m)| m.category == cat)
            .map(|(i, _)| i as u16)
            .collect()
    }

    pub fn len(&self) -> usize {
        self.materials.len()
    }

    pub fn is_empty(&self) -> bool {
        self.materials.is_empty()
    }
}
