//! Data-driven content ("raws"). The engine knows mechanisms; these files
//! supply everything else. Phase 0: materials only.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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
pub struct Raws {
    pub materials: MaterialRegistry,
    pub plants: PlantRegistry,
    pub tileset: Option<TilesetDef>,
}

impl Raws {
    pub fn load(data_dir: &Path) -> Result<Self> {
        let tileset_path = data_dir.join("tileset.ron");
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
        Ok(Raws {
            materials: MaterialRegistry::load_dir(&data_dir.join("materials"))?,
            plants: PlantRegistry::load_dir(&data_dir.join("plants"))?,
            tileset,
        })
    }
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
