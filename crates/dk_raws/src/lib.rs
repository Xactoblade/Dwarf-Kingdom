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
        let mut materials: Vec<MaterialDef> = Vec::new();
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
            let mut defs: Vec<MaterialDef> = ron::from_str(&text)
                .with_context(|| format!("parsing {}", path.display()))?;
            materials.append(&mut defs);
        }
        Self::from_defs(materials)
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
