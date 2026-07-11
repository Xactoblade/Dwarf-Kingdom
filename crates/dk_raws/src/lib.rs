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
        Ok(Self { materials, by_id })
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

    pub fn get(&self, index: u16) -> &MaterialDef {
        &self.materials[index as usize]
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
