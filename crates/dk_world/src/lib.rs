//! Tile world: the 3D local map, its generation, and persistence.

use anyhow::{Context, Result};
use dk_raws::{MaterialCategory, MaterialRegistry};
use rand::Rng;
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Sentinel for "no material" (air).
pub const NO_MATERIAL: u16 = u16::MAX;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TileShape {
    Empty,
    Solid,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Tile {
    pub material: u16,
    pub shape: TileShape,
}

impl Tile {
    pub const AIR: Tile = Tile {
        material: NO_MATERIAL,
        shape: TileShape::Empty,
    };

    pub fn solid(material: u16) -> Self {
        Tile {
            material,
            shape: TileShape::Solid,
        }
    }

    pub fn is_solid(&self) -> bool {
        self.shape == TileShape::Solid
    }
}

/// Dense 3D tile map. Phase 0 keeps a flat vec; chunking arrives with
/// dirty-region tracking in Phase 1.
#[derive(Serialize, Deserialize)]
pub struct Map {
    pub width: usize,
    pub height: usize,
    pub depth: usize,
    pub seed: u64,
    tiles: Vec<Tile>,
}

impl Map {
    pub fn new_air(width: usize, height: usize, depth: usize, seed: u64) -> Self {
        Map {
            width,
            height,
            depth,
            seed,
            tiles: vec![Tile::AIR; width * height * depth],
        }
    }

    #[inline]
    fn idx(&self, x: usize, y: usize, z: usize) -> usize {
        (z * self.height + y) * self.width + x
    }

    pub fn in_bounds(&self, x: i64, y: i64, z: i64) -> bool {
        x >= 0
            && y >= 0
            && z >= 0
            && (x as usize) < self.width
            && (y as usize) < self.height
            && (z as usize) < self.depth
    }

    pub fn get(&self, x: usize, y: usize, z: usize) -> Tile {
        self.tiles[self.idx(x, y, z)]
    }

    pub fn set(&mut self, x: usize, y: usize, z: usize, t: Tile) {
        let i = self.idx(x, y, z);
        self.tiles[i] = t;
    }

    /// Highest solid z at a column, if any.
    pub fn surface_z(&self, x: usize, y: usize) -> Option<usize> {
        (0..self.depth).rev().find(|&z| self.get(x, y, z).is_solid())
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let file = std::fs::File::create(path)
            .with_context(|| format!("creating {}", path.display()))?;
        bincode::serialize_into(std::io::BufWriter::new(file), self)?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self> {
        let file = std::fs::File::open(path)
            .with_context(|| format!("opening {}", path.display()))?;
        let map = bincode::deserialize_from(std::io::BufReader::new(file))?;
        Ok(map)
    }
}

/// Generate a Phase 0 local map: rolling surface, soil cover, sedimentary
/// over igneous strata, ore veins scattered through the stone.
pub fn generate(reg: &MaterialRegistry, rng: &mut ChaCha8Rng, width: usize, height: usize, depth: usize, seed: u64) -> Map {
    let mut map = Map::new_air(width, height, depth, seed);

    let soils = reg.indices_in_category(MaterialCategory::Soil);
    let sedimentary = reg.indices_in_category(MaterialCategory::Sedimentary);
    let igneous = reg.indices_in_category(MaterialCategory::Igneous);
    let metamorphic = reg.indices_in_category(MaterialCategory::Metamorphic);
    let ores = reg.indices_in_category(MaterialCategory::Ore);
    assert!(
        !soils.is_empty() && !sedimentary.is_empty() && !igneous.is_empty(),
        "raws must define soil, sedimentary and igneous materials"
    );

    // --- Surface heightfield: coarse random grid, bilinearly interpolated.
    let base = (depth * 2) / 3;
    let coarse = 8usize;
    let gw = width / coarse + 2;
    let gh = height / coarse + 2;
    let grid: Vec<f32> = (0..gw * gh).map(|_| rng.gen_range(-3.5f32..3.5)).collect();
    let height_at = |x: usize, y: usize| -> usize {
        let fx = x as f32 / coarse as f32;
        let fy = y as f32 / coarse as f32;
        let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
        let (tx, ty) = (fx.fract(), fy.fract());
        let g = |gx: usize, gy: usize| grid[gy * gw + gx];
        let top = g(x0, y0) * (1.0 - tx) + g(x0 + 1, y0) * tx;
        let bot = g(x0, y0 + 1) * (1.0 - tx) + g(x0 + 1, y0 + 1) * tx;
        let off = top * (1.0 - ty) + bot * ty;
        ((base as f32 + off).round() as i64).clamp(4, depth as i64 - 2) as usize
    };

    // --- Column fill: soil on top, then sedimentary, then igneous with
    // occasional metamorphic bands.
    const SOIL_DEPTH: usize = 3;
    const SEDIMENTARY_DEPTH: usize = 8;
    for y in 0..height {
        for x in 0..width {
            let surface = height_at(x, y);
            let soil_mat = soils[(x / 7 + y / 9) % soils.len()];
            let sed_mat = sedimentary[(x / 11 + y / 6) % sedimentary.len()];
            for z in 0..=surface {
                let below_surface = surface - z;
                let mat = if below_surface < SOIL_DEPTH {
                    soil_mat
                } else if below_surface < SOIL_DEPTH + SEDIMENTARY_DEPTH {
                    sed_mat
                } else if !metamorphic.is_empty() && z % 9 == 0 {
                    metamorphic[z / 9 % metamorphic.len()]
                } else {
                    igneous[(z / 5) % igneous.len()]
                };
                map.set(x, y, z, Tile::solid(mat));
            }
        }
    }

    // --- Ore veins: random 3D walks through solid stone.
    if !ores.is_empty() {
        let vein_count = (width * height) / 300;
        for _ in 0..vein_count {
            let ore = ores[rng.gen_range(0..ores.len())];
            let mut x = rng.gen_range(0..width) as i64;
            let mut y = rng.gen_range(0..height) as i64;
            let mut z = rng.gen_range(2..base.saturating_sub(SOIL_DEPTH)) as i64;
            for _ in 0..rng.gen_range(12..30) {
                if map.in_bounds(x, y, z) {
                    let t = map.get(x as usize, y as usize, z as usize);
                    if t.is_solid() {
                        map.set(x as usize, y as usize, z as usize, Tile::solid(ore));
                    }
                }
                x += rng.gen_range(-1..=1);
                y += rng.gen_range(-1..=1);
                // Veins wander mostly horizontally.
                if rng.gen_ratio(1, 4) {
                    z += rng.gen_range(-1..=1);
                }
            }
        }
    }

    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_save_load() {
        let mut m = Map::new_air(4, 4, 4, 7);
        m.set(1, 2, 3, Tile::solid(5));
        let dir = std::env::temp_dir().join("dk_world_test");
        let path = dir.join("map.bin");
        m.save(&path).unwrap();
        let loaded = Map::load(&path).unwrap();
        assert_eq!(loaded.get(1, 2, 3).material, 5);
        assert!(loaded.get(0, 0, 0).shape == TileShape::Empty);
    }
}
