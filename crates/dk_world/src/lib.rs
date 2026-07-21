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

    /// Does this tile stop a line of sight (and a bolt)? Walls and shut gates
    /// do; open floor, stairs, and ramps do not.
    pub fn blocks_sight(&self) -> bool {
        matches!(self.shape, TileShape::Solid | TileShape::Gate)
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

    /// Is there a clear shot from `from` to `to` — an unobstructed straight
    /// line a bolt (or an eye) could travel? Only same-level shots count; a
    /// wall or shut gate anywhere between the two endpoints blocks it. The
    /// endpoints themselves are not tested (the shooter and target stand on
    /// walkable tiles). Uses an integer Bresenham walk so it is exact and
    /// draws no rng.
    pub fn clear_shot(&self, from: Pos, to: Pos) -> bool {
        if from.z != to.z {
            return false;
        }
        let (mut x, mut y) = (from.x, from.y);
        let (dx, dy) = ((to.x - x).abs(), (to.y - y).abs());
        let (sx, sy) = ((to.x - x).signum(), (to.y - y).signum());
        let mut err = dx - dy;
        loop {
            if (x, y) == (to.x, to.y) {
                return true;
            }
            // Advance one step along the line, then test the tile we land on
            // (skipping the shooter's own tile; the loop exits on reaching the
            // target before testing it).
            let e2 = 2 * err;
            if e2 > -dy {
                err -= dy;
                x += sx;
            }
            if e2 < dx {
                err += dx;
                y += sy;
            }
            if (x, y) == (to.x, to.y) {
                return true;
            }
            if self
                .tile_at(Pos::new(x, y, from.z))
                .is_none_or(|t| t.blocks_sight())
            {
                return false;
            }
        }
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

    /// The highest z-level that has anything in it at all — the roof of the
    /// world, wherever the mountains or the fort's own towers reach.
    ///
    /// A view above this sees nothing: the renderer looks a few levels down
    /// from the current z for something to draw, and above the roof there is
    /// nothing within reach, so the map renders as a black void. Clamping the
    /// view here keeps the player over the world instead of lost in the sky.
    pub fn highest_solid_z(&self) -> usize {
        for z in (0..self.depth).rev() {
            for y in 0..self.height {
                for x in 0..self.width {
                    if self.get(x, y, z).shape != TileShape::Empty {
                        return z;
                    }
                }
            }
        }
        0
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

/// How dramatic a region's surface relief is, driven by its overworld biome.
/// Scales the heightfield AFTER the noise is drawn (so the RNG stream is
/// untouched) and, for mountains, bares the upper rock. `Rolling` reproduces
/// the original terrain exactly, keeping every existing caller/test identical.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relief {
    /// Near-flat plains and grassland.
    Flat,
    /// The classic gentle roll (the original, byte-identical terrain).
    Rolling,
    /// Moderate, hummocky ground — hills.
    Hilly,
    /// Tall, steep ground that climbs many levels and sheds soil to bare rock
    /// on its heights — mountains.
    Mountainous,
}

impl Relief {
    /// Multiplier on the heightfield amplitude. `Rolling` is 1.0 so the terrain
    /// is unchanged from before reliefs existed.
    fn amplitude(self) -> f32 {
        match self {
            Relief::Flat => 0.4,
            Relief::Rolling => 1.0,
            Relief::Hilly => 1.8,
            Relief::Mountainous => 2.8,
        }
    }

    /// Depth of soil atop a column of the given surface height. Only mountains
    /// differ: their higher, steeper ground wears down to bare stone.
    fn soil_depth(self, surface: usize, base: usize) -> usize {
        const FULL: usize = 3;
        match self {
            Relief::Mountainous => {
                if surface >= base + 6 {
                    0
                } else if surface >= base + 3 {
                    1
                } else {
                    FULL
                }
            }
            _ => FULL,
        }
    }
}

/// Generate a region with the default (mixed) surface and gentle rolling
/// relief. Kept as the stable entry point so every existing caller and test is
/// byte-for-byte unchanged.
pub fn generate(reg: &MaterialRegistry, rng: &mut ChaCha8Rng, width: usize, height: usize, depth: usize, seed: u64) -> Map {
    generate_terrain(reg, rng, width, height, depth, seed, SurfaceStyle::Default, Relief::Rolling)
}

/// Smooth 2-D value noise in `[0, 1)` at frequency `f` (region size ~= 1/f),
/// lattice-hashed from `seed`+`salt` and smoothstep-interpolated. Purely a
/// function of position and seed — it draws NO rng, so it varies the map's look
/// without perturbing the deterministic simulation stream. Used to lay down
/// soil and stone in organic, blobby regions with curved boundaries instead of
/// the hard rectangular grid that `(x/7 + y/9)` produced.
fn value_noise(x: usize, y: usize, seed: u64, salt: u64, f: f32) -> f32 {
    let hash = |ix: i64, iy: i64| -> f32 {
        let mut h = seed ^ salt.wrapping_mul(0x9E3779B97F4A7C15);
        h = h.wrapping_add(ix as u64).wrapping_mul(0xBF58476D1CE4E5B9);
        h ^= h >> 27;
        h = h.wrapping_add(iy as u64).wrapping_mul(0x94D049BB133111EB);
        h ^= h >> 31;
        (h & 0xFFFF) as f32 / 65536.0
    };
    let (fx, fy) = (x as f32 * f, y as f32 * f);
    let (x0, y0) = (fx.floor() as i64, fy.floor() as i64);
    let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
    let (sx, sy) = (tx * tx * (3.0 - 2.0 * tx), ty * ty * (3.0 - 2.0 * ty));
    let n0 = hash(x0, y0) + (hash(x0 + 1, y0) - hash(x0, y0)) * sx;
    let n1 = hash(x0, y0 + 1) + (hash(x0 + 1, y0 + 1) - hash(x0, y0 + 1)) * sx;
    n0 + (n1 - n0) * sy
}

/// Pick an index into `n` items from a noise value in `[0, 1)`.
fn noise_pick(v: f32, n: usize) -> usize {
    ((v * n as f32) as usize).min(n.saturating_sub(1))
}

/// Generate with a chosen surface soil style and gentle rolling relief.
pub fn generate_styled(reg: &MaterialRegistry, rng: &mut ChaCha8Rng, width: usize, height: usize, depth: usize, seed: u64, surface: SurfaceStyle) -> Map {
    generate_terrain(reg, rng, width, height, depth, seed, surface, Relief::Rolling)
}

/// Cut a winding river across an already-generated map: a flat-bottomed
/// channel filled with water, sunk just below the surrounding ground so its
/// banks contain it. A pure map mutation deterministic in `seed` — it draws no
/// RNG, so callers that don't want a river are unaffected. The water sim keeps
/// the (already-level) river settled.
/// Sink a set of tiles into a single flat water body: a stone bed one level
/// below the LOWEST solid surface over the body and its banks (so every bank
/// tile stays solid at the bed level and water can't leak sideways), cleared
/// open above and brimming with water. Shared by rivers, lakes and ponds.
fn carve_water_body(map: &mut Map, tiles: &std::collections::BTreeSet<(usize, usize)>) {
    let (w, h) = (map.width, map.height);
    let mut min_surface = usize::MAX;
    for &(x, y) in tiles {
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
    for &(x, y) in tiles {
        let Some(st) = map.surface_z(x, y) else { continue };
        let mat = map.get(x, y, bed_z).material;
        let top = (st + 1).min(map.depth - 1);
        for z in (bed_z + 1)..=top {
            map.set_at(
                Pos::new(x as i32, y as i32, z as i32),
                Tile { material: NO_MATERIAL, shape: TileShape::Empty, water: 0, magma: 0 },
            );
        }
        map.set_at(
            Pos::new(x as i32, y as i32, bed_z as i32),
            Tile { material: mat, shape: TileShape::Floor, water: 7, magma: 0 },
        );
    }
}

/// Cut a river that meanders from `from` to `to` (tile coords, usually on
/// opposite map edges matching the overworld's flow direction), wandering off
/// the straight line with seed-varied harmonics and breathing in width, so no
/// two rivers look alike. A pure map mutation, deterministic in `seed`.
pub fn carve_river(map: &mut Map, seed: u64, from: (usize, usize), to: (usize, usize)) {
    use std::f32::consts::{PI, TAU};
    let (w, h) = (map.width, map.height);
    let (fx, fy) = (from.0 as f32, from.1 as f32);
    let (tx, ty) = (to.0 as f32, to.1 as f32);
    let (dx, dy) = (tx - fx, ty - fy);
    let len = (dx * dx + dy * dy).sqrt().max(1.0);
    let (px, py) = (-dy / len, dx / len); // unit perpendicular
    let ph1 = (seed % 1000) as f32 / 1000.0 * TAU;
    let ph2 = ((seed / 1000) % 1000) as f32 / 1000.0 * TAU;
    let a1 = (len * 0.16).clamp(2.0, 10.0);
    let a2 = (len * 0.07).clamp(1.0, 5.0);
    let steps = (len * 1.6) as usize + 2;
    let mut tiles: std::collections::BTreeSet<(usize, usize)> = std::collections::BTreeSet::new();
    for i in 0..=steps {
        let f = i as f32 / steps as f32;
        // Meander, tapering to zero at both ends so it meets the edges cleanly.
        let taper = (f * PI).sin();
        let off = taper * (a1 * (f * TAU * 1.3 + ph1).sin() + a2 * (f * TAU * 3.1 + ph2).sin());
        let cx = fx + dx * f + px * off;
        let cy = fy + dy * f + py * off;
        // Width breathes between a brook and a broad river.
        let r = (1.0 + 1.0 * (0.5 + 0.5 * (f * TAU * 2.4 + ph2).sin())).round() as i32;
        for oy in -r..=r {
            for ox in -r..=r {
                if ox * ox + oy * oy <= r * r {
                    let (nx, ny) = (cx as i32 + ox, cy as i32 + oy);
                    // Never flood the fort's spawn clearing at map centre: the
                    // river fords dry around it, so dwarves begin on solid, safe
                    // ground rather than in the water. The keep-out is a touch
                    // wider than the clearing's flat core so no deep water sits
                    // right at the settlers' feet.
                    let (dcx, dcy) = (nx - (w / 2) as i32, ny - (h / 2) as i32);
                    if dcx * dcx + dcy * dcy <= FORT_KEEPOUT * FORT_KEEPOUT {
                        continue;
                    }
                    if nx >= 1 && ny >= 1 && (nx as usize) < w - 1 && (ny as usize) < h - 1 {
                        tiles.insert((nx as usize, ny as usize));
                    }
                }
            }
        }
    }
    carve_water_body(map, &tiles);
}

/// Radius (tiles) around map centre kept dry of river water, so the embark
/// clearing and the dwarves' spawn are never underwater. Wider than the
/// clearing's flat core (8) plus a bank of margin.
const FORT_KEEPOUT: i32 = 12;

/// Fill a basin with a lake: an irregular blob of water centred at `(cx, cy)`,
/// its shore wobbled by seed so it reads as a natural pond or lake, not a disc.
pub fn carve_lake(map: &mut Map, seed: u64, cx: usize, cy: usize, radius: f32) {
    use std::f32::consts::TAU;
    let (w, h) = (map.width, map.height);
    let ph = (seed % 997) as f32 / 997.0 * TAU;
    let ph2 = ((seed / 997) % 997) as f32 / 997.0 * TAU;
    let mut tiles: std::collections::BTreeSet<(usize, usize)> = std::collections::BTreeSet::new();
    let bound = radius.ceil() as i32 + 2;
    for oy in -bound..=bound {
        for ox in -bound..=bound {
            let ang = (oy as f32).atan2(ox as f32);
            let wobble = 1.0 + 0.34 * (ang * 3.0 + ph).sin() + 0.16 * (ang * 5.0 + ph2).sin();
            let rr = radius * wobble;
            if (ox * ox + oy * oy) as f32 <= rr * rr {
                let (nx, ny) = (cx as i32 + ox, cy as i32 + oy);
                if nx >= 1 && ny >= 1 && (nx as usize) < w - 1 && (ny as usize) < h - 1 {
                    tiles.insert((nx as usize, ny as usize));
                }
            }
        }
    }
    carve_water_body(map, &tiles);
}

/// Scatter `count` small ponds across the map's lower ground — the sort of
/// still water a swamp or wet forest holds. Deterministic in `seed`.
pub fn carve_ponds(map: &mut Map, seed: u64, count: usize) {
    use rand::{Rng, SeedableRng};
    let (w, h) = (map.width, map.height);
    // The median surface height — ponds prefer the low ground below it.
    let mut heights: Vec<usize> = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            if let Some(s) = map.surface_z(x, y) {
                heights.push(s);
            }
        }
    }
    if heights.is_empty() {
        return;
    }
    heights.sort_unstable();
    let median = heights[heights.len() / 2];
    let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(seed ^ 0x9E37_79B9);
    let mut placed = 0;
    for _ in 0..(count * 30) {
        if placed >= count {
            break;
        }
        let x = rng.gen_range(4..w.saturating_sub(4).max(5));
        let y = rng.gen_range(4..h.saturating_sub(4).max(5));
        // Low, land, and dry.
        match map.surface_z(x, y) {
            Some(s) if s <= median && map.water_at(Pos::new(x as i32, y as i32, s as i32 + 1)) == 0 => {}
            _ => continue,
        }
        let radius = 2.0 + rng.gen_range(0.0f32..2.5);
        carve_lake(map, seed ^ ((x as u64) << 20) ^ (y as u64), x, y, radius);
        placed += 1;
    }
}

/// Seed rare adamantine deep in the stone: a few short vertical spires near the
/// bottom of the map. The deepest one or two are hollow-cored — mining their
/// bottom cap breaches the underworld. Post-gen and deterministic in `seed`,
/// applied app-side (kept out of `generate()` so headless forts stay
/// byte-identical). Returns the breach tile positions.
pub fn place_adamantine(map: &mut Map, seed: u64, adamantine: u16) -> Vec<Pos> {
    use rand::{Rng, SeedableRng};
    let mut rng = ChaCha8Rng::seed_from_u64(seed ^ 0x0ADA_0ADA_0ADA);
    let (w, h, d) = (map.width, map.height, map.depth);
    let mut breaches = Vec::new();
    if w < 12 || h < 12 || d < 8 {
        return breaches;
    }
    let count = 3 + rng.gen_range(0..3);
    for i in 0..count {
        let cx = rng.gen_range(5..w - 5) as i32;
        let cy = rng.gen_range(5..h - 5) as i32;
        let top = 2 + rng.gen_range(2..(d / 5).max(3));
        for z in 2..=top.min(d - 1) {
            for (dx, dy) in [(0i32, 0i32), (1, 0), (0, 1), (-1, 0), (0, -1)] {
                let (nx, ny) = (cx + dx, cy + dy);
                if nx >= 0 && ny >= 0 && (nx as usize) < w && (ny as usize) < h {
                    let t = map.get(nx as usize, ny as usize, z);
                    if t.is_solid() {
                        map.set_at(Pos::new(nx, ny, z as i32), Tile::solid(adamantine));
                    }
                }
            }
        }
        // The deepest one or two spires cap the abyss.
        if i < 2 {
            breaches.push(Pos::new(cx, cy, 2));
        }
    }
    breaches
}

/// Identify a shallow water-bearing layer: the solid soil and sedimentary tiles
/// a couple of levels below the surface. Mining one floods the dig — the caller
/// (the sim) flags these tiles and turns each into a spring when it is opened.
/// This does NOT change the terrain (the stone keeps its material); it only
/// reports which tiles are wet. Deterministic; the caller decides whether a
/// given embark has an aquifer at all.
pub fn place_aquifer(map: &Map, reg: &MaterialRegistry) -> Vec<Pos> {
    let mut tiles = Vec::new();
    for y in 0..map.height {
        for x in 0..map.width {
            let Some(top) = map.surface_z(x, y) else { continue };
            // The two solid layers just beneath the surface — deep enough that a
            // fort digging down soon meets the water.
            for dz in 2..=3usize {
                if top < dz {
                    continue;
                }
                let z = top - dz;
                let t = map.get(x, y, z);
                if t.is_solid()
                    && matches!(
                        reg.get(t.material).category,
                        MaterialCategory::Soil | MaterialCategory::Sedimentary
                    )
                {
                    tiles.push(Pos::new(x as i32, y as i32, z as i32));
                }
            }
        }
    }
    tiles
}

/// Hollow out a great cavern layer in the middle depths — an open cave system
/// with natural rock pillars, a walkable floor, and a few still pools. Dig down
/// far enough and a fort breaks into it, exactly as in Dwarf Fortress. Carves
/// the terrain and returns the cavern-floor tiles (the caller flags them so they
/// can be lit with a fungal glow and, later, grow cave life). Deterministic in
/// `seed`; app-embark-only (never carved into small test maps).
pub fn carve_caverns(map: &mut Map, seed: u64) -> Vec<Pos> {
    use rand::{Rng, SeedableRng};
    let mut rng = ChaCha8Rng::seed_from_u64(seed ^ 0xCA7E_0CA7_0CA7_0CA7);
    let (w, h, d) = (map.width, map.height, map.depth);
    let mut floors = Vec::new();
    if w < 48 || h < 48 || d < 20 {
        return floors;
    }
    // One cavern layer in the middle depths, a few levels tall, safely above the
    // adamantine spires at the very bottom.
    let floor_z = (d / 4).max(4); // e.g. 8 of 32
    let ceil_z = (d * 3 / 8).max(floor_z + 3); // e.g. 12 of 32
    // A coarse, interpolated open/solid field: high => open cave, low => a
    // natural pillar of rock left standing floor-to-ceiling.
    let coarse = 6usize;
    let (gw, gh) = (w / coarse + 2, h / coarse + 2);
    let grid: Vec<f32> = (0..gw * gh).map(|_| rng.gen_range(0.0f32..1.0)).collect();
    let noise = |x: usize, y: usize| -> f32 {
        let (fx, fy) = (x as f32 / coarse as f32, y as f32 / coarse as f32);
        let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
        let (tx, ty) = (fx.fract(), fy.fract());
        let g = |gx: usize, gy: usize| grid[gy * gw + gx];
        let top = g(x0, y0) * (1.0 - tx) + g(x0 + 1, y0) * tx;
        let bot = g(x0, y0 + 1) * (1.0 - tx) + g(x0 + 1, y0 + 1) * tx;
        top * (1.0 - ty) + bot * ty
    };
    for y in 0..h {
        for x in 0..w {
            // Keep a solid rim around the edges so the cavern is enclosed.
            let edge = x < 2 || y < 2 || x + 2 >= w || y + 2 >= h;
            if edge || noise(x, y) <= 0.34 {
                continue; // a pillar / wall of rock stands here
            }
            let fmat = map.get(x, y, floor_z).material;
            if !map.get(x, y, floor_z).is_solid() {
                continue;
            }
            // Hollow the band above the floor to open air.
            for z in (floor_z + 1)..=ceil_z.min(d - 1) {
                map.set(x, y, z, Tile::AIR);
            }
            map.set(x, y, floor_z, Tile::floor(fmat));
            floors.push(Pos::new(x as i32, y as i32, floor_z as i32));
        }
    }
    // A handful of still pools on the cavern floor — clustered, not a flood.
    if !floors.is_empty() {
        let pool_seeds = 3 + rng.gen_range(0..3);
        for _ in 0..pool_seeds {
            let c = floors[rng.gen_range(0..floors.len())];
            let r = 2 + rng.gen_range(0..3) as i32;
            for &p in &floors {
                if (p.x - c.x).abs() <= r && (p.y - c.y).abs() <= r {
                    map.set_water(p, MAX_WATER);
                }
            }
        }
    }
    floors
}

/// Flood the deepest reaches with a magma sea: an open layer just above the
/// adamantine spires, its floor a lake of molten rock. Dig down to it for a
/// magma forge — or breach it and burn. Carves the terrain and fills the floor
/// with magma (a saved tile property, so no new save field). Deterministic in
/// `seed`; app-embark-only and only on full-depth maps.
pub fn carve_magma_sea(map: &mut Map, seed: u64) {
    use rand::{Rng, SeedableRng};
    let mut rng = ChaCha8Rng::seed_from_u64(seed ^ 0x0AA6_0AA6_0AA6_0AA6);
    let (w, h, d) = (map.width, map.height, map.depth);
    if w < 48 || h < 48 || d < 24 {
        return;
    }
    // Just above the adamantine at z2, well below the cavern layer.
    let floor_z = 3usize;
    let ceil_z = 5usize;
    let coarse = 7usize;
    let (gw, gh) = (w / coarse + 2, h / coarse + 2);
    let grid: Vec<f32> = (0..gw * gh).map(|_| rng.gen_range(0.0f32..1.0)).collect();
    let noise = |x: usize, y: usize| -> f32 {
        let (fx, fy) = (x as f32 / coarse as f32, y as f32 / coarse as f32);
        let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
        let (tx, ty) = (fx.fract(), fy.fract());
        let g = |gx: usize, gy: usize| grid[gy * gw + gx];
        let top = g(x0, y0) * (1.0 - tx) + g(x0 + 1, y0) * tx;
        let bot = g(x0, y0 + 1) * (1.0 - tx) + g(x0 + 1, y0 + 1) * tx;
        top * (1.0 - ty) + bot * ty
    };
    for y in 0..h {
        for x in 0..w {
            let edge = x < 2 || y < 2 || x + 2 >= w || y + 2 >= h;
            if edge || noise(x, y) <= 0.30 {
                continue; // a rock island / rim stands in the sea
            }
            if !map.get(x, y, floor_z).is_solid() {
                continue;
            }
            let fmat = map.get(x, y, floor_z).material;
            for z in (floor_z + 1)..=ceil_z.min(d - 1) {
                map.set(x, y, z, Tile::AIR);
            }
            map.set(x, y, floor_z, Tile::floor(fmat));
            map.set_magma(Pos::new(x as i32, y as i32, floor_z as i32), MAX_WATER);
        }
    }
}

pub fn generate_terrain(reg: &MaterialRegistry, rng: &mut ChaCha8Rng, width: usize, height: usize, depth: usize, seed: u64, surface: SurfaceStyle, relief: Relief) -> Map {
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
            // Scale AFTER the noise is drawn — the RNG stream is unchanged, so
            // Rolling (amplitude 1.0) reproduces the original heights exactly.
            let off = (top * (1.0 - ty) + bot * ty) * relief.amplitude();
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

    // --- Embark clearing: a fort should begin in the open, not in a random dip
    // ringed by higher ground (which reads as being walled in by rock) nor on a
    // sheer peak. Rather than stamp a flat box on the map, ease the land around
    // the centre toward the local median height with a round, smooth falloff —
    // so the fort sits in a gentle open basin that follows the lie of the land
    // and blends seamlessly into the natural hills, no hard edges or terraces.
    // Only on full-size fortress maps — small maps (unit tests) stay untouched.
    if width >= 64 && height >= 64 {
        let (cx, cy) = (width as f32 / 2.0, height as f32 / 2.0);
        let r_flat = 8.0f32; // an open, level core to build on
        let r_edge = 30.0f32; // fades smoothly into the natural land by here
        // Target level: the median surface over the central area, so the
        // clearing rests at the natural lie of the land, neither pit nor mesa.
        let win = 22i64;
        let mut window: Vec<usize> = Vec::new();
        let (icx, icy) = (cx as i64, cy as i64);
        for y in (icy - win).max(0)..=(icy + win).min(height as i64 - 1) {
            for x in (icx - win).max(0)..=(icx + win).min(width as i64 - 1) {
                window.push(heights[y as usize * width + x as usize]);
            }
        }
        window.sort_unstable();
        let target = window[window.len() / 2] as f32;
        for y in 0..height {
            for x in 0..width {
                let dx = x as f32 + 0.5 - cx;
                let dy = y as f32 + 0.5 - cy;
                let dist = (dx * dx + dy * dy).sqrt();
                // Round Euclidean falloff, smoothstepped: full weight inside the
                // core, easing to zero by the edge — no square rings, no X.
                let t = ((r_edge - dist) / (r_edge - r_flat)).clamp(0.0, 1.0);
                let w = t * t * (3.0 - 2.0 * t);
                let nat = heights[y * width + x] as f32;
                let blended = nat * (1.0 - w) + target * w;
                heights[y * width + x] =
                    blended.round().clamp(4.0, depth as f32 - 2.0) as usize;
            }
        }
        // Re-relax so the blend never leaves a step taller than a ramp can climb.
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
            // Organic patches of soil and stone: the material follows a smooth
            // noise field, so types meet along curved, natural boundaries rather
            // than the old rectangular grid cells.
            let soil_mat = fixed_soil
                .unwrap_or_else(|| soils[noise_pick(value_noise(x, y, seed, 0x50117, 0.10), soils.len())]);
            let sed_mat =
                sedimentary[noise_pick(value_noise(x, y, seed, 0x5ED, 0.075), sedimentary.len())];
            // Mountains bare their heights to rock; other reliefs keep full soil.
            let soil_depth = relief.soil_depth(surface, base);
            let mut top_mat = soil_mat;
            for z in 0..=surface {
                let below_surface = surface - z;
                let mat = if below_surface < soil_depth {
                    soil_mat
                } else if below_surface < soil_depth + SEDIMENTARY_DEPTH {
                    sed_mat
                } else if !metamorphic.is_empty() && z % 9 == 0 {
                    metamorphic[z / 9 % metamorphic.len()]
                } else {
                    igneous[(z / 5) % igneous.len()]
                };
                map.set(x, y, z, Tile::solid(mat));
                if z == surface {
                    top_mat = mat;
                }
            }
            // The walkable ground reads as its topmost material — soil where
            // there's soil, bare rock on a stripped mountainside (so no grass
            // grows there).
            map.set(x, y, surface + 1, Tile::floor(top_mat));
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
                    combat: Default::default(),
                    is_flux: false,
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
            MaterialDef { id: "dirt".into(), name: "dirt".into(), category: MaterialCategory::Soil, color: [0; 3], value: 1, combat: Default::default(), is_flux: false },
            MaterialDef { id: "sed".into(), name: "sed".into(), category: MaterialCategory::Sedimentary, color: [0; 3], value: 1, combat: Default::default(), is_flux: false },
            MaterialDef { id: "ign".into(), name: "ign".into(), category: MaterialCategory::Igneous, color: [0; 3], value: 1, combat: Default::default(), is_flux: false },
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
            MaterialDef { id: "loam".into(), name: "loam".into(), category: MaterialCategory::Soil, color: [1; 3], value: 1, combat: Default::default(), is_flux: false },
            MaterialDef { id: "clay".into(), name: "clay".into(), category: MaterialCategory::Soil, color: [2; 3], value: 1, combat: Default::default(), is_flux: false },
            MaterialDef { id: "sand".into(), name: "sand".into(), category: MaterialCategory::Soil, color: [3; 3], value: 1, combat: Default::default(), is_flux: false },
            MaterialDef { id: "sed".into(), name: "sed".into(), category: MaterialCategory::Sedimentary, color: [0; 3], value: 1, combat: Default::default(), is_flux: false },
            MaterialDef { id: "ign".into(), name: "ign".into(), category: MaterialCategory::Igneous, color: [0; 3], value: 1, combat: Default::default(), is_flux: false },
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
    fn mountainous_relief_climbs_higher_and_bares_rock() {
        use rand::SeedableRng;
        let reg = strata_reg();
        let spread = |relief: Relief| {
            let mut rng = ChaCha8Rng::seed_from_u64(42);
            let m = generate_terrain(&reg, &mut rng, 48, 48, 32, 42, SurfaceStyle::Default, relief);
            let (mut lo, mut hi, mut bare_rock) = (usize::MAX, 0usize, 0u32);
            for y in 0..48 {
                for x in 0..48 {
                    let wz = m.walk_surface_z(x, y).unwrap();
                    lo = lo.min(wz);
                    hi = hi.max(wz);
                    if reg.get(m.get(x, y, wz - 1).material).category != MaterialCategory::Soil {
                        bare_rock += 1;
                    }
                }
            }
            (hi - lo, bare_rock)
        };
        let (roll_spread, roll_rock) = spread(Relief::Rolling);
        let (mtn_spread, mtn_rock) = spread(Relief::Mountainous);
        assert!(
            mtn_spread > roll_spread + 3,
            "mountains climb far higher: rolling {roll_spread} vs mountainous {mtn_spread}"
        );
        assert_eq!(roll_rock, 0, "rolling ground is soil all over");
        assert!(mtn_rock > 0, "mountain heights are bared to rock");
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
