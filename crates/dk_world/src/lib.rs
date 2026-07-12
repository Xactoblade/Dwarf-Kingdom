//! Tile world: the 3D local map, its generation, pathfinding, and material
//! remapping for saves.

use anyhow::{Context, Result};
use dk_raws::{MaterialCategory, MaterialRegistry};
use rand::Rng;
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};

pub mod path;

pub use path::Pos;

/// Sentinel for "no material" (air).
pub const NO_MATERIAL: u16 = u16::MAX;

/// Water this deep (or deeper) is impassable and drowns non-swimmers.
pub const DEEP_WATER: u8 = 4;
/// Maximum water per tile.
pub const MAX_WATER: u8 = 7;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TileShape {
    /// Open space. Not walkable (nothing to stand on).
    Empty,
    /// Solid rock/soil wall. Not passable.
    Solid,
    /// Walkable surface (natural ground or a mined-out tile).
    Floor,
    /// Walkable; connects vertically to stairs directly above/below.
    Stairs,
    /// Walkable slope; connects to walkable tiles one level up alongside.
    Ramp,
    /// A closed floodgate: blocks walkers and water. Opens back into Floor.
    Gate,
}

impl TileShape {
    pub fn is_walkable(self) -> bool {
        matches!(self, TileShape::Floor | TileShape::Stairs | TileShape::Ramp)
    }

    pub fn name(self) -> &'static str {
        match self {
            TileShape::Empty => "open air",
            TileShape::Solid => "wall",
            TileShape::Floor => "floor",
            TileShape::Stairs => "stairs",
            TileShape::Ramp => "ramp",
            TileShape::Gate => "closed floodgate",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Tile {
    pub material: u16,
    pub shape: TileShape,
    /// Water depth 0-7 (see dk_sim). Lives on the tile so pathfinding and
    /// rendering see it without a parallel grid.
    pub water: u8,
    /// Magma depth 0-7. Any amount is impassable and lethal.
    pub magma: u8,
}

impl Tile {
    pub const AIR: Tile = Tile {
        material: NO_MATERIAL,
        shape: TileShape::Empty,
        water: 0,
        magma: 0,
    };

    pub fn solid(material: u16) -> Self {
        Tile { material, shape: TileShape::Solid, water: 0, magma: 0 }
    }

    pub fn floor(material: u16) -> Self {
        Tile { material, shape: TileShape::Floor, water: 0, magma: 0 }
    }

    /// Can water occupy this tile?
    pub fn holds_water(&self) -> bool {
        !matches!(self.shape, TileShape::Solid | TileShape::Gate)
    }

    pub fn is_solid(&self) -> bool {
        self.shape == TileShape::Solid
    }
}

/// Dense 3D tile map. Phase 0/1 keeps a flat vec; chunking arrives with
/// dirty-region tracking when maps grow.
#[derive(Debug, Serialize, Deserialize)]
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

    /// Bounds-checked accessor used by pathfinding and the sim.
    pub fn tile_at(&self, p: Pos) -> Option<Tile> {
        if self.in_bounds(p.x as i64, p.y as i64, p.z as i64) {
            Some(self.get(p.x as usize, p.y as usize, p.z as usize))
        } else {
            None
        }
    }

    pub fn set_at(&mut self, p: Pos, t: Tile) {
        assert!(self.in_bounds(p.x as i64, p.y as i64, p.z as i64));
        self.set(p.x as usize, p.y as usize, p.z as usize, t);
    }

    /// Walkable, not dangerously deep water, and free of magma (any depth
    /// of magma is death — nobody paths through it).
    pub fn walkable(&self, p: Pos) -> bool {
        self.tile_at(p)
            .is_some_and(|t| t.shape.is_walkable() && t.water < DEEP_WATER && t.magma == 0)
    }

    pub fn magma_at(&self, p: Pos) -> u8 {
        self.tile_at(p).map_or(0, |t| t.magma)
    }

    pub fn set_magma(&mut self, p: Pos, m: u8) {
        if self.in_bounds(p.x as i64, p.y as i64, p.z as i64) {
            let mut t = self.get(p.x as usize, p.y as usize, p.z as usize);
            t.magma = m;
            self.set(p.x as usize, p.y as usize, p.z as usize, t);
        }
    }

    pub fn water_at(&self, p: Pos) -> u8 {
        self.tile_at(p).map_or(0, |t| t.water)
    }

    pub fn set_water(&mut self, p: Pos, w: u8) {
        if self.in_bounds(p.x as i64, p.y as i64, p.z as i64) {
            let mut t = self.get(p.x as usize, p.y as usize, p.z as usize);
            t.water = w;
            self.set(p.x as usize, p.y as usize, p.z as usize, t);
        }
    }

    /// Highest solid z at a column, if any.
    pub fn surface_z(&self, x: usize, y: usize) -> Option<usize> {
        (0..self.depth).rev().find(|&z| self.get(x, y, z).is_solid())
    }

    /// Highest walkable z at a column, if any.
    pub fn walk_surface_z(&self, x: usize, y: usize) -> Option<usize> {
        (0..self.depth)
            .rev()
            .find(|&z| self.get(x, y, z).shape.is_walkable())
    }

    /// Structural sanity checks for freshly deserialized maps.
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            self.width > 0 && self.height > 0 && self.depth > 0,
            "corrupt map: zero dimension"
        );
        anyhow::ensure!(
            self.tiles.len() == self.width * self.height * self.depth,
            "corrupt map: tile count {} does not match {}x{}x{}",
            self.tiles.len(),
            self.width,
            self.height,
            self.depth
        );
        Ok(())
    }

    /// Remap material indices recorded under `old_ids` (a save's manifest)
    /// to the currently loaded registry. Errors if a material vanished.
    pub fn remap_materials(&mut self, old_ids: &[String], reg: &MaterialRegistry) -> Result<()> {
        let remap = build_remap(old_ids, reg)?;
        for tile in &mut self.tiles {
            if tile.material != NO_MATERIAL {
                let old = tile.material as usize;
                anyhow::ensure!(old < remap.len(), "corrupt map: material index {old} out of range");
                tile.material = remap[old];
            }
        }
        Ok(())
    }
}

/// Old index -> current registry index, by material id.
pub fn build_remap(old_ids: &[String], reg: &MaterialRegistry) -> Result<Vec<u16>> {
    old_ids
        .iter()
        .map(|id| {
            reg.index_of(id)
                .with_context(|| format!("save uses material '{id}' missing from current raws"))
        })
        .collect()
}

/// Generate a Phase 1 local map: rolling surface with walkable ground,
/// soil cover, sedimentary over igneous strata, ore veins in the stone.
/// How a region's surface soil reads, driven by its overworld biome. Purely
/// cosmetic — it only recolors the top soil layer (all are Soil-category, so
/// digging and value are unchanged) and never alters the RNG stream, so a
/// region's structure is identical across styles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceStyle {
    /// The classic mixed palette (loam/clay/sand by position).
    Default,
    /// Pale, dry sand — deserts.
    Sandy,
    /// Reddish clay — swamps and marshes.
    Clayey,
    /// Rich brown loam — grasslands and forests.
    Loamy,
}

/// Generate a region with the default (mixed) surface. Kept as the stable
/// entry point so every existing caller and test is byte-for-byte unchanged.
pub fn generate(reg: &MaterialRegistry, rng: &mut ChaCha8Rng, width: usize, height: usize, depth: usize, seed: u64) -> Map {
    generate_styled(reg, rng, width, height, depth, seed, SurfaceStyle::Default)
}

/// Cut a winding river across an already-generated map: a flat-bottomed
/// channel filled with water, sunk just below the surrounding ground so its
/// banks contain it. A pure map mutation deterministic in `seed` — it draws no
/// RNG, so callers that don't want a river are unaffected. The water sim keeps
/// the (already-level) river settled.
pub fn carve_river(map: &mut Map, seed: u64) {
    use std::f32::consts::TAU;
    let (w, h) = (map.width, map.height);
    // A gently meandering course from the west edge to the east, two tiles wide.
    let phase = (seed % 997) as f32 / 997.0 * TAU;
    let amp = (h as f32 / 6.0).max(2.0);
    let mid = h as f32 / 2.0;
    let mut path: Vec<(usize, usize)> = Vec::new();
    for x in 0..w {
        let fy = mid + amp * ((x as f32 / w.max(1) as f32 * TAU * 1.5) + phase).sin();
        let cy = (fy.round() as i64).clamp(1, h as i64 - 3) as usize;
        path.push((x, cy));
        path.push((x, cy + 1));
    }
    // The flat bed sits one below the LOWEST solid surface over the channel AND
    // its banks, so every bank tile is solid at the bed level (water can't
    // leak sideways into lower ground beside the river).
    let mut min_surface = usize::MAX;
    for &(x, y) in &path {
        for (nx, ny) in [(x, y), (x.saturating_sub(1), y), (x + 1, y), (x, y.saturating_sub(1)), (x, y + 1)] {
            if nx < w && ny < h {
                if let Some(s) = map.surface_z(nx, ny) {
                    min_surface = min_surface.min(s);
                }
            }
        }
    }
    if min_surface == usize::MAX || min_surface < 2 {
        return;
    }
    let bed_z = min_surface - 1;
    for &(x, y) in &path {
        let Some(st) = map.surface_z(x, y) else { continue };
        let mat = map.get(x, y, bed_z).material;
        // Open the channel: clear everything from just above the bed up to the
        // old ground floor.
        let top = (st + 1).min(map.depth - 1);
        for z in (bed_z + 1)..=top {
            map.set_at(
                Pos::new(x as i32, y as i32, z as i32),
                Tile { material: NO_MATERIAL, shape: TileShape::Empty, water: 0, magma: 0 },
            );
        }
        // The bed: a stone floor brimming with water.
        map.set_at(
            Pos::new(x as i32, y as i32, bed_z as i32),
            Tile { material: mat, shape: TileShape::Floor, water: 7, magma: 0 },
        );
    }
}

pub fn generate_styled(reg: &MaterialRegistry, rng: &mut ChaCha8Rng, width: usize, height: usize, depth: usize, seed: u64, surface: SurfaceStyle) -> Map {
    // The strata/heightfield math below assumes room for soil + stone layers.
    assert!(
        width >= 16 && height >= 16 && depth >= 12,
        "generate() requires at least a 16x16x12 map (got {width}x{height}x{depth})"
    );
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

    // --- Surface heightfield: coarse random grid, bilinearly interpolated,
    // then relaxed so adjacent columns differ by at most one z-level. Ramps
    // placed on every slope keep the whole surface one walkable region.
    let base = (depth * 2) / 3;
    let coarse = 8usize;
    let gw = width / coarse + 2;
    let gh = height / coarse + 2;
    let grid: Vec<f32> = (0..gw * gh).map(|_| rng.gen_range(-3.5f32..3.5)).collect();
    let mut heights = vec![0usize; width * height];
    for y in 0..height {
        for x in 0..width {
            let fx = x as f32 / coarse as f32;
            let fy = y as f32 / coarse as f32;
            let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
            let (tx, ty) = (fx.fract(), fy.fract());
            let g = |gx: usize, gy: usize| grid[gy * gw + gx];
            let top = g(x0, y0) * (1.0 - tx) + g(x0 + 1, y0) * tx;
            let bot = g(x0, y0 + 1) * (1.0 - tx) + g(x0 + 1, y0 + 1) * tx;
            let off = top * (1.0 - ty) + bot * ty;
            heights[y * width + x] =
                ((base as f32 + off).round() as i64).clamp(4, depth as i64 - 2) as usize;
        }
    }
    // Relax: no column more than one level above any neighbor.
    loop {
        let mut changed = false;
        for y in 0..height {
            for x in 0..width {
                let h = heights[y * width + x];
                let mut min_n = usize::MAX;
                if x > 0 { min_n = min_n.min(heights[y * width + x - 1]); }
                if x + 1 < width { min_n = min_n.min(heights[y * width + x + 1]); }
                if y > 0 { min_n = min_n.min(heights[(y - 1) * width + x]); }
                if y + 1 < height { min_n = min_n.min(heights[(y + 1) * width + x]); }
                if min_n != usize::MAX && h > min_n + 1 {
                    heights[y * width + x] = min_n + 1;
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    let height_at = |x: usize, y: usize| heights[y * width + x];

    // --- Column fill: soil on top, then sedimentary, then igneous with
    // occasional metamorphic bands. Above the top solid tile sits a walkable
    // Floor tile (natural ground surface).
    const SOIL_DEPTH: usize = 3;
    const SEDIMENTARY_DEPTH: usize = 8;
    // The biome recolors the surface soil. A specific style pins one soil
    // material (falling back to the mixed palette if the raws lack it); this
    // draws no RNG, so only the top layer's color changes.
    let styled_soil = |id: &str| reg.index_of(id).filter(|m| soils.contains(m));
    let fixed_soil = match surface {
        SurfaceStyle::Sandy => styled_soil("sand"),
        SurfaceStyle::Clayey => styled_soil("clay"),
        SurfaceStyle::Loamy => styled_soil("loam"),
        SurfaceStyle::Default => None,
    };
    for y in 0..height {
        for x in 0..width {
            let surface = height_at(x, y);
            let soil_mat = fixed_soil.unwrap_or_else(|| soils[(x / 7 + y / 9) % soils.len()]);
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
            map.set(x, y, surface + 1, Tile::floor(soil_mat));
        }
    }

    // --- Ramps: any surface tile with a one-level-higher neighbor becomes a
    // ramp so walkers can traverse slopes.
    for y in 0..height {
        for x in 0..width {
            let h = height_at(x, y);
            let higher_neighbor = [(1i64, 0i64), (-1, 0), (0, 1), (0, -1)]
                .iter()
                .any(|&(dx, dy)| {
                    let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                    nx >= 0
                        && ny >= 0
                        && (nx as usize) < width
                        && (ny as usize) < height
                        && height_at(nx as usize, ny as usize) == h + 1
                });
            if higher_neighbor {
                let floor_z = h + 1;
                let mat = map.get(x, y, floor_z).material;
                map.set(x, y, floor_z, Tile { material: mat, shape: TileShape::Ramp, water: 0, magma: 0 });
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

    // --- Caverns: blobby open galleries carved deep beneath the surface,
    // and magma pockets pooling at the roots of the world.
    let cav_lo = (depth / 8).max(2);
    let cav_hi = (depth / 4).max(cav_lo + 1);
    let mag_hi = (depth / 10).max(1);
    let coarse2 = 6usize;
    let gw2 = width / coarse2 + 2;
    let gh2 = height / coarse2 + 2;
    let cav_grid: Vec<f32> = (0..gw2 * gh2).map(|_| rng.gen_range(0.0f32..1.0)).collect();
    let mag_grid: Vec<f32> = (0..gw2 * gh2).map(|_| rng.gen_range(0.0f32..1.0)).collect();
    let field_at = |grid: &Vec<f32>, x: usize, y: usize| -> f32 {
        let fx = x as f32 / coarse2 as f32;
        let fy = y as f32 / coarse2 as f32;
        let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
        let (tx, ty) = (fx.fract(), fy.fract());
        let g = |gx: usize, gy: usize| grid[gy * gw2 + gx];
        let top = g(x0, y0) * (1.0 - tx) + g(x0 + 1, y0) * tx;
        let bot = g(x0, y0 + 1) * (1.0 - tx) + g(x0 + 1, y0 + 1) * tx;
        top * (1.0 - ty) + bot * ty
    };
    for y in 0..height {
        for x in 0..width {
            // Cavern galleries: hollow where the noise runs high.
            if field_at(&cav_grid, x, y) > 0.62 {
                for z in cav_lo..=cav_hi {
                    if map.get(x, y, z).is_solid() {
                        map.set(x, y, z, Tile::AIR);
                    }
                }
                // A walkable cavern floor sits on the solid below.
                if map.get(x, y, cav_lo - 1).is_solid() {
                    let mat = map.get(x, y, cav_lo - 1).material;
                    map.set(x, y, cav_lo, Tile::floor(mat));
                }
            }
            // Magma pockets: pooled fire in the deepest stone.
            if field_at(&mag_grid, x, y) > 0.72 {
                for z in 1..=mag_hi {
                    if map.get(x, y, z).is_solid() {
                        let mat = map.get(x, y, z).material;
                        let mut t = Tile::floor(mat);
                        t.magma = 7;
                        map.set(x, y, z, t);
                    }
                }
            }
        }
    }

    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use dk_raws::MaterialDef;

    fn registry(ids: &[&str]) -> MaterialRegistry {
        MaterialRegistry::from_defs(
            ids.iter()
                .map(|id| MaterialDef {
                    id: id.to_string(),
                    name: id.to_string(),
                    category: MaterialCategory::Soil,
                    color: [1, 2, 3],
                    value: 1,
                })
                .collect(),
        )
        .unwrap()
    }

    #[test]
    fn remap_shifts_indices_by_id() {
        let mut m = Map::new_air(4, 4, 4, 7);
        m.set(1, 2, 3, Tile::solid(1)); // "rock" under the old manifest
        let old_ids = vec!["dirt".to_string(), "rock".to_string()];
        let new_reg = registry(&["clay", "dirt", "rock"]);
        m.remap_materials(&old_ids, &new_reg).unwrap();
        assert_eq!(m.get(1, 2, 3).material, new_reg.index_of("rock").unwrap());
    }

    #[test]
    fn remap_fails_when_material_removed() {
        let mut m = Map::new_air(4, 4, 4, 7);
        let old_ids = vec!["dirt".to_string(), "rock".to_string()];
        let err = m
            .remap_materials(&old_ids, &registry(&["dirt"]))
            .unwrap_err()
            .to_string();
        assert!(err.contains("rock"), "unexpected error: {err}");
    }

    #[test]
    fn validate_rejects_bad_tile_count() {
        let mut m = Map::new_air(4, 4, 4, 7);
        m.tiles.pop();
        assert!(m.validate().is_err());
    }

    #[test]
    fn generated_surface_is_walkable() {
        // registry() marks everything Soil; mapgen needs the stone categories.
        let reg = MaterialRegistry::from_defs(vec![
            MaterialDef { id: "dirt".into(), name: "dirt".into(), category: MaterialCategory::Soil, color: [0; 3], value: 1 },
            MaterialDef { id: "sed".into(), name: "sed".into(), category: MaterialCategory::Sedimentary, color: [0; 3], value: 1 },
            MaterialDef { id: "ign".into(), name: "ign".into(), category: MaterialCategory::Igneous, color: [0; 3], value: 1 },
        ])
        .unwrap();
        use rand::SeedableRng;
        let mut rng = ChaCha8Rng::seed_from_u64(1);
        let map = generate(&reg, &mut rng, 32, 32, 16, 1);
        for y in 0..32 {
            for x in 0..32 {
                let wz = map.walk_surface_z(x, y).expect("every column has ground");
                let shape = map.get(x, y, wz).shape;
                assert!(
                    matches!(shape, TileShape::Floor | TileShape::Ramp),
                    "surface at ({x},{y},{wz}) is {shape:?}"
                );
                assert!(map.get(x, y, wz - 1).is_solid());
            }
        }
        // The whole surface must be one connected walkable region (ramps).
        let regions = path::Regions::new(&map);
        let origin = Pos::new(0, 0, map.walk_surface_z(0, 0).unwrap() as i32);
        for y in 0..32 {
            for x in 0..32 {
                let wz = map.walk_surface_z(x, y).unwrap();
                assert!(
                    regions.same_region(origin, Pos::new(x as i32, y as i32, wz as i32)),
                    "surface tile ({x},{y}) is cut off from the rest of the map"
                );
            }
        }
    }

    fn strata_reg() -> MaterialRegistry {
        MaterialRegistry::from_defs(vec![
            MaterialDef { id: "loam".into(), name: "loam".into(), category: MaterialCategory::Soil, color: [1; 3], value: 1 },
            MaterialDef { id: "clay".into(), name: "clay".into(), category: MaterialCategory::Soil, color: [2; 3], value: 1 },
            MaterialDef { id: "sand".into(), name: "sand".into(), category: MaterialCategory::Soil, color: [3; 3], value: 1 },
            MaterialDef { id: "sed".into(), name: "sed".into(), category: MaterialCategory::Sedimentary, color: [0; 3], value: 1 },
            MaterialDef { id: "ign".into(), name: "ign".into(), category: MaterialCategory::Igneous, color: [0; 3], value: 1 },
        ])
        .unwrap()
    }

    #[test]
    fn a_sandy_biome_lays_sand_across_the_surface() {
        use rand::SeedableRng;
        let reg = strata_reg();
        let sand = reg.index_of("sand").unwrap();
        let mut rng = ChaCha8Rng::seed_from_u64(5);
        let map = generate_styled(&reg, &mut rng, 32, 32, 16, 5, SurfaceStyle::Sandy);
        for y in 0..32 {
            for x in 0..32 {
                let wz = map.walk_surface_z(x, y).unwrap();
                // The topmost solid tile (just below the walkable floor) is soil.
                assert_eq!(
                    map.get(x, y, wz - 1).material,
                    sand,
                    "sandy surface at ({x},{y}) should be sand"
                );
            }
        }
    }

    #[test]
    fn styling_the_surface_does_not_change_the_structure() {
        // Only the soil color changes: the height/shape of every tile is
        // identical between Default and a styled surface (same seed, same RNG).
        use rand::SeedableRng;
        let reg = strata_reg();
        let mut r1 = ChaCha8Rng::seed_from_u64(9);
        let a = generate_styled(&reg, &mut r1, 32, 32, 16, 9, SurfaceStyle::Default);
        let mut r2 = ChaCha8Rng::seed_from_u64(9);
        let b = generate_styled(&reg, &mut r2, 32, 32, 16, 9, SurfaceStyle::Clayey);
        for y in 0..32 {
            for x in 0..32 {
                assert_eq!(
                    map_shape(&a, x, y),
                    map_shape(&b, x, y),
                    "surface height/shape differs at ({x},{y})"
                );
            }
        }
    }

    fn map_shape(m: &Map, x: usize, y: usize) -> usize {
        m.walk_surface_z(x, y).unwrap()
    }
}
