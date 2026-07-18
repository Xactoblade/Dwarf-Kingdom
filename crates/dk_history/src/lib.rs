//! The world outside the fortress: overworld regions, civilizations, and a
//! history simulation whose events feed Legends mode (BLUEPRINT.md §4.6).
//!
//! Everything here is derived purely from a seed — the world is regenerated
//! identically at every launch, so only the fortress sim needs saving.

use rand::Rng;
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};

pub mod names;

/// How many worlds to reject before settling for what we have. Dwarf Fortress
/// retries until it succeeds and shows the player the count; we would rather
/// hand back an odd world than hang.
const MAX_WORLD_ATTEMPTS: usize = 24;

/// How much of a climb it takes to wring rain out of the air, in elevation.
/// Below this the ground is merely rolling and the weather does not notice.
const MIN_OROGRAPHIC_CLIMB: f32 = 25.0;
/// How fast air picks moisture back up crossing land — this is what sets how
/// far a rain shadow reaches inland before the country turns green again.
const SHADOW_RECOVERY: f32 = 0.06;

// ---------------------------------------------------------------- overworld

/// The land a region is. Dwarf Fortress's base biome set, which falls out of
/// elevation first and then a drainage-by-rainfall table (see `classify`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Biome {
    Ocean,
    Lake,
    Mountains,
    /// Frozen wetland: tundra's drowned cousin, above drainage 75.
    Glacier,
    Tundra,
    /// The three deserts, told apart by drainage.
    SandDesert,
    RockyWasteland,
    Badlands,
    Grassland,
    Savanna,
    Shrubland,
    /// The two wetlands: marsh is drier country than swamp.
    Marsh,
    Swamp,
    ConiferForest,
    /// A cold conifer forest.
    Taiga,
    BroadleafForest,
}

impl Biome {
    pub fn name(self) -> &'static str {
        match self {
            Biome::Ocean => "ocean",
            Biome::Lake => "lake",
            Biome::Mountains => "mountains",
            Biome::Glacier => "glacier",
            Biome::Tundra => "tundra",
            Biome::SandDesert => "sand desert",
            Biome::RockyWasteland => "rocky wasteland",
            Biome::Badlands => "badlands",
            Biome::Grassland => "grassland",
            Biome::Savanna => "savanna",
            Biome::Shrubland => "shrubland",
            Biome::Marsh => "marsh",
            Biome::Swamp => "swamp",
            Biome::ConiferForest => "conifer forest",
            Biome::Taiga => "taiga",
            Biome::BroadleafForest => "broadleaf forest",
        }
    }

    /// Can a fortress embark here? Not on open water, and not on a glacier —
    /// there is nothing under the ice to dig a home out of.
    pub fn embarkable(self) -> bool {
        !matches!(self, Biome::Ocean | Biome::Lake | Biome::Glacier)
    }

    /// Woodland of any kind — where the trees are.
    pub fn is_forest(self) -> bool {
        matches!(
            self,
            Biome::ConiferForest | Biome::Taiga | Biome::BroadleafForest
        )
    }

    /// Wet country: standing water, reeds, and clay underfoot.
    pub fn is_wetland(self) -> bool {
        matches!(self, Biome::Marsh | Biome::Swamp)
    }

    /// Dry country: sand and bare rock.
    pub fn is_desert(self) -> bool {
        matches!(
            self,
            Biome::SandDesert | Biome::RockyWasteland | Biome::Badlands
        )
    }

    /// Open country under grass or scrub.
    pub fn is_grassy(self) -> bool {
        matches!(self, Biome::Grassland | Biome::Savanna | Biome::Shrubland)
    }

    /// Display color, sRGB 0-255.
    pub fn color(self) -> [u8; 3] {
        match self {
            Biome::Ocean => [24, 48, 110],
            Biome::Lake => [38, 84, 160],
            Biome::Mountains => [128, 124, 120],
            Biome::Glacier => [222, 236, 244],
            Biome::Tundra => [176, 188, 196],
            Biome::SandDesert => [214, 194, 126],
            Biome::RockyWasteland => [166, 150, 118],
            Biome::Badlands => [172, 120, 78],
            Biome::Grassland => [96, 140, 60],
            Biome::Savanna => [154, 158, 72],
            Biome::Shrubland => [118, 138, 66],
            Biome::Marsh => [96, 122, 84],
            Biome::Swamp => [64, 84, 58],
            Biome::ConiferForest => [34, 82, 52],
            Biome::Taiga => [56, 90, 76],
            Biome::BroadleafForest => [40, 110, 44],
        }
    }
}

/// The kinds of country a named region can be.
///
/// Dwarf Fortress lumps like with like before it names anything: "Wetland =
/// swamp+marsh; Forest = broadleaf+coniferous+taiga; Grassland/Hills =
/// grassland+savanna+shrubland; Desert = badlands+rocky wasteland+sand desert;
/// Lake, Tundra, Glacier, Ocean, Mountains each stand alone." A traveller does
/// not say "I crossed the shrubland and then the savanna" — they say they
/// crossed the plains.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum RegionKind {
    Ocean,
    Lake,
    Mountains,
    Glacier,
    Tundra,
    Desert,
    Grassland,
    Wetland,
    Forest,
}

impl RegionKind {
    pub fn of(biome: Biome) -> RegionKind {
        match biome {
            Biome::Ocean => RegionKind::Ocean,
            Biome::Lake => RegionKind::Lake,
            Biome::Mountains => RegionKind::Mountains,
            Biome::Glacier => RegionKind::Glacier,
            Biome::Tundra => RegionKind::Tundra,
            Biome::SandDesert | Biome::RockyWasteland | Biome::Badlands => RegionKind::Desert,
            Biome::Grassland | Biome::Savanna | Biome::Shrubland => RegionKind::Grassland,
            Biome::Marsh | Biome::Swamp => RegionKind::Wetland,
            Biome::ConiferForest | Biome::Taiga | Biome::BroadleafForest => RegionKind::Forest,
        }
    }

    /// The noun in the name: "the Forest of Whispering".
    pub fn noun(self) -> &'static str {
        match self {
            RegionKind::Ocean => "Ocean",
            RegionKind::Lake => "Lake",
            RegionKind::Mountains => "Mountains",
            RegionKind::Glacier => "Glacier",
            RegionKind::Tundra => "Tundra",
            RegionKind::Desert => "Desert",
            RegionKind::Grassland => "Plains",
            RegionKind::Wetland => "Marshes",
            RegionKind::Forest => "Forest",
        }
    }
}

/// A named stretch of country: contiguous land of one kind that shares one
/// nature. Dwarf Fortress's subregion, and the thing a player actually names
/// when they talk about where they settled.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamedRegion {
    pub name: String,
    pub kind: RegionKind,
    pub alignment: Alignment,
    /// How many overworld tiles it covers. DF's size classes: <=24 small,
    /// 25-99 medium, 100+ large.
    pub tiles: usize,
}

impl NamedRegion {
    pub fn size_class(&self) -> &'static str {
        match self.tiles {
            0..=24 => "small",
            25..=99 => "medium",
            _ => "large",
        }
    }
}

/// How wild the land is — Dwarf Fortress's savagery, in its own words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Savagery {
    Calm,
    Wilderness,
    Savage,
}

/// Whether the land itself is kindly, indifferent, or hates you. Not a mesh
/// field in Dwarf Fortress — it is painted onto whole regions late in
/// generation, so a named region is uniformly one thing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Alignment {
    Good,
    Neutral,
    Evil,
}

impl Region {
    pub fn savagery_class(&self) -> Savagery {
        match self.savagery {
            0..=32 => Savagery::Calm,
            33..=65 => Savagery::Wilderness,
            _ => Savagery::Savage,
        }
    }

    /// Dwarf Fortress's "surroundings": savagery crossed with alignment, and
    /// the words a player actually reads on the embark screen. The matrix is
    /// theirs — Serene through Terrifying.
    pub fn surroundings(&self) -> &'static str {
        match (self.alignment, self.savagery_class()) {
            (Alignment::Good, Savagery::Calm) => "Serene",
            (Alignment::Good, Savagery::Wilderness) => "Mirthful",
            (Alignment::Good, Savagery::Savage) => "Joyous Wilds",
            (Alignment::Neutral, Savagery::Calm) => "Calm",
            (Alignment::Neutral, Savagery::Wilderness) => "Wilderness",
            (Alignment::Neutral, Savagery::Savage) => "Untamed Wilds",
            (Alignment::Evil, Savagery::Calm) => "Sinister",
            (Alignment::Evil, Savagery::Wilderness) => "Haunted",
            (Alignment::Evil, Savagery::Savage) => "Terrifying",
        }
    }

    /// Hills are drainage's doing: past the halfway mark the water sinks away
    /// and the land rumples. It is why Dwarf Fortress's biome chart splits
    /// grassland, savanna and shrubland into flat and hilly at drainage 50.
    pub fn hilly(&self) -> bool {
        self.drainage >= 50 && self.biome.is_grassy()
    }

    /// Elevation as a 0..1 fraction, for shading and relief.
    pub fn elevation_frac(&self) -> f32 {
        self.elevation as f32 / Overworld::MAX_ELEVATION as f32
    }
}

/// A cardinal direction on the overworld / a local map edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Dir {
    N,
    E,
    S,
    W,
}

impl Dir {
    pub const ALL: [Dir; 4] = [Dir::N, Dir::E, Dir::S, Dir::W];
    pub fn delta(self) -> (i32, i32) {
        match self {
            Dir::N => (0, -1),
            Dir::E => (1, 0),
            Dir::S => (0, 1),
            Dir::W => (-1, 0),
        }
    }
    pub fn opposite(self) -> Dir {
        match self {
            Dir::N => Dir::S,
            Dir::E => Dir::W,
            Dir::S => Dir::N,
            Dir::W => Dir::E,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Region {
    /// 0..400 on Dwarf Fortress's own scale: below 100 is ocean, 300 and up is
    /// mountain, and everything between is decided by drainage and rainfall.
    pub elevation: u16,
    /// 0..100. Where the rain falls.
    pub rainfall: u8,
    /// 0..100. Whether it drains away or lies there — the field that decides
    /// swamp from forest, and flat from hilly.
    pub drainage: u8,
    /// Degrees on a region scale (roughly Celsius). Below -5 the land freezes;
    /// around 85 it turns tropical.
    pub temperature: i16,
    /// 0..100. Fire in the rock. Only a 100 makes a volcano.
    pub volcanism: u8,
    /// 0..100. How wild the beasts are. Not part of the biome table — this is
    /// what the land does to you, not what grows on it.
    pub savagery: u8,
    /// Whether the land is kindly, indifferent, or hates you.
    pub alignment: Alignment,
    /// Index into `Overworld::named` — the stretch of country this tile is
    /// part of, and the name it goes by.
    pub subregion: usize,
    pub biome: Biome,
    /// A river flows through this region (part of the downhill river network).
    pub river: bool,
    /// The edge the river enters from (upstream), if any.
    pub river_in: Option<Dir>,
    /// The edge the river exits toward (downhill), if any.
    pub river_out: Option<Dir>,
    /// This region sits in a basin and holds a lake.
    pub lake: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Overworld {
    pub width: usize,
    pub height: usize,
    pub regions: Vec<Region>,
    /// Every named stretch of country. `Region::subregion` indexes this.
    pub named: Vec<NamedRegion>,
    /// Where the mountains are on fire.
    pub volcanoes: Vec<(usize, usize)>,
}

impl Overworld {
    /// Dwarf Fortress's elevation scale: 0..400, with the sea at 100 and the
    /// mountains from 300 up.
    pub const MAX_ELEVATION: u16 = 400;
    pub const SEA_LEVEL: u16 = 100;
    pub const MOUNTAIN_LEVEL: u16 = 300;

    pub fn get(&self, x: usize, y: usize) -> &Region {
        &self.regions[y * self.width + x]
    }

    /// What land this is. Dwarf Fortress's rule, in its order:
    ///
    /// > "When determining the biome, elevation comes first; any terrain with
    /// > an elevation of 0-99 is ocean, while any terrain with an elevation of
    /// > 300-400 is mountain. All other biomes lie in between these two
    /// > extremes. The remaining base biomes are determined by the combination
    /// > of drainage and rainfall."
    ///
    /// The drainage-by-rainfall grid below is theirs, read off the wiki's
    /// biome distribution chart. Temperature is then laid over the top: cold
    /// enough and anything becomes tundra, or glacier where the water lies.
    ///
    /// A caveat worth keeping: the wiki presents these thresholds on a current
    /// page but sources them from a 40d-era analysis, so they are a strong
    /// default rather than a measured fact about the modern game.
    pub fn classify(elevation: u16, rainfall: u8, drainage: u8, temperature: i16) -> Biome {
        if elevation < Self::SEA_LEVEL {
            return Biome::Ocean;
        }
        if elevation >= Self::MOUNTAIN_LEVEL {
            return Biome::Mountains;
        }
        // Frozen: "at or below -5, all base biomes with drainage <75 become
        // Tundra, and biomes with drainage 75+ become Glaciers".
        if temperature <= -5 {
            return if drainage >= 75 { Biome::Glacier } else { Biome::Tundra };
        }
        let base = match rainfall {
            0..=9 => match drainage {
                0..=32 => Biome::SandDesert,
                33..=65 => Biome::RockyWasteland,
                _ => Biome::Badlands,
            },
            10..=19 => Biome::Grassland,
            20..=32 => Biome::Savanna,
            33..=65 => {
                if drainage <= 32 {
                    Biome::Marsh
                } else {
                    Biome::Shrubland
                }
            }
            66..=74 => {
                if drainage <= 32 {
                    Biome::Swamp
                } else {
                    Biome::ConiferForest
                }
            }
            _ => {
                if drainage <= 32 {
                    Biome::Swamp
                } else {
                    Biome::BroadleafForest
                }
            }
        };
        // "Between -4 and 9 inclusive, Conifer Forests become Taiga."
        if base == Biome::ConiferForest && temperature <= 9 {
            return Biome::Taiga;
        }
        base
    }

    fn generate(rng: &mut ChaCha8Rng, width: usize, height: usize) -> Self {
        // Coarse random grids, bilinearly interpolated — same trick as the
        // local map, at world scale.
        // `spread` pushes values away from the middle before they are used.
        //
        // Interpolating between random grid points averages the extremes away:
        // measured, drainage came out a bell curve with 8 tiles of 2304 in its
        // driest tenth and 15 in its wettest. A world like that has no deserts
        // and no rainforest — it is one continent of gentle green, which is
        // exactly what ours was. Dwarf Fortress controls the same thing with
        // its per-field FREQUENCY weights; this is the same idea with one knob.
        let field = |rng: &mut ChaCha8Rng, coarse: usize, spread: f32| -> Vec<f32> {
            let gw = width / coarse + 2;
            let gh = height / coarse + 2;
            let grid: Vec<f32> = (0..gw * gh).map(|_| rng.gen_range(0.0f32..1.0)).collect();
            let mut out = vec![0.0f32; width * height];
            for y in 0..height {
                for x in 0..width {
                    let fx = x as f32 / coarse as f32;
                    let fy = y as f32 / coarse as f32;
                    let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
                    let (tx, ty) = (fx.fract(), fy.fract());
                    let g = |gx: usize, gy: usize| grid[gy * gw + gx];
                    let top = g(x0, y0) * (1.0 - tx) + g(x0 + 1, y0) * tx;
                    let bot = g(x0, y0 + 1) * (1.0 - tx) + g(x0 + 1, y0 + 1) * tx;
                    let v = top * (1.0 - ty) + bot * ty;
                    // A soft curve, NOT a stretch-and-clamp. Clamping pinned
                    // every raw value past 0.833 to exactly 1.0 — which on the
                    // elevation field meant a fifth of the world was flat
                    // tabletop at exactly 400. Tables have no downhill, so the
                    // river tracer found no lower neighbour and flagged them
                    // all as basins: measured, 3621 of 3844 lakes across forty
                    // worlds sat on mountain SUMMITS. This saturates instead of
                    // clipping, so high ground stays distinct and tapers.
                    let t = (v - 0.5) * spread * 2.0;
                    out[y * width + x] = (0.5 + 0.5 * t.tanh()).clamp(0.0, 1.0);
                }
            }
            out
        };
        // Dwarf Fortress seeds six fields and fills them in fractally:
        // elevation, rainfall, temperature, drainage, volcanism, and
        // wildness. Each gets its own coarseness, so mountains run in long
        // ranges while rainfall varies over shorter distances.
        let elevation = field(rng, 16, 1.5);
        let rainfall = field(rng, 12, 1.8);
        let drainage = field(rng, 10, 1.7);
        // Volcanism's extremes are set deliberately later — see place_volcanoes.
        let volcanism = field(rng, 20, 1.0);
        let savagery = field(rng, 14, 1.4);
        let temp_noise = field(rng, 24, 1.0);

        let mut regions = Vec::with_capacity(width * height);
        for y in 0..height {
            for x in 0..width {
                let i = y * width + x;
                let elevation = (elevation[i] * Self::MAX_ELEVATION as f32) as u16;
                let rainfall = (rainfall[i] * 100.0) as u8;
                let drainage = (drainage[i] * 100.0) as u8;
                let volcanism = (volcanism[i] * 100.0) as u8;
                let savagery = (savagery[i] * 100.0) as u8;
                // Temperature and biome are BOTH set properly further down —
                // Dwarf Fortress revises the rain for the mountains, then
                // recalculates the heat, and only then asks what grows here.
                // Placeholders until then.
                let temperature = 0;
                let biome = Biome::Ocean;
                regions.push(Region {
                    elevation,
                    temperature,
                    rainfall,
                    drainage,
                    volcanism,
                    savagery,
                    // Both painted on later, once the land is known.
                    alignment: Alignment::Neutral,
                    subregion: 0,
                    biome,
                    river: false,
                    river_in: None,
                    river_out: None,
                    lake: false,
                });
            }
        }
        let mut world = Overworld {
            width,
            height,
            regions,
            named: Vec::new(),
            volcanoes: Vec::new(),
        };
        // Dwarf Fortress's own order, as Toady describes it — and the order
        // matters, because the passes feed each other. A one-pass world (which
        // this was) cannot look like a DF world: its rain has never heard of
        // its mountains.
        //
        //   ... select points for highest peaks ... smooth mid-level
        //   elevations to make more plains ... place volcanoes respecting
        //   volcanism hot spots ... EROSION AND RIVER STAGE ... rainfall
        //   adjusted for rain shadow and orographic precipitation ...
        //   temperature recalculated from elevation and rainfall ...
        //   detect/name biome regions ...
        world.raise_peaks(rng);
        world.smooth_midlands();
        world.place_volcanoes(rng);
        world.trace_rivers();
        world.revise_rainfall();
        world.set_temperature(&temp_noise);
        // Only now is it known what grows here.
        world.classify_all();
        world.place_alignment(rng);
        // Named last: a region is contiguous land of one kind sharing one
        // alignment, so both must be settled before anything can be named.
        world.name_regions(rng);
        world
    }

    /// Build a world, and keep building until one is fit to live in.
    ///
    /// This is Dwarf Fortress's rejection loop, and it is not an error path —
    /// it is how the thing works: "Worlds are generated with parameters which
    /// are LIKELY to produce worlds that can support a required number of
    /// mountains, and are then checked to make sure they meet the criteria",
    /// because "factors like mountain-tile count can't be determined ahead of
    /// time". DF rejects and retries; the player watches the counter climb.
    ///
    /// Ours needed it. Measured across three seeds before this existed, one
    /// world's highest ground was elevation 294 — below the mountain line —
    /// so it had no mountains, and therefore nowhere for dwarves to live and
    /// no dwarven civilization in its history at all.
    ///
    /// Deterministic: the same seed runs the same rejections in the same order
    /// and lands on the same world.
    fn generate_verified(rng: &mut ChaCha8Rng, width: usize, height: usize) -> Self {
        let mut last = None;
        for _ in 0..MAX_WORLD_ATTEMPTS {
            let world = Overworld::generate(rng, width, height);
            if world.verify().is_ok() {
                return world;
            }
            last = Some(world);
        }
        // Every one of them was built AND checked, and the last failed too.
        // Take it rather than loop forever — a strange world is still a world,
        // and a hang is not. (Written as generate-then-check so that every
        // world handed back has actually been looked at: the earlier form built
        // one more world than it checked and returned that unexamined one,
        // which could be exactly the mountainless world this loop exists to
        // reject.)
        last.expect("MAX_WORLD_ATTEMPTS is nonzero")
    }

    /// Is this world fit to live in? Dwarf Fortress checks its own criteria
    /// after the fact for exactly the things that cannot be arranged up front.
    fn verify(&self) -> Result<(), &'static str> {
        let n = self.regions.len();
        // A world that is two-thirds sea is a world with nowhere to put a
        // fortress. Measured: one seed in six came out 63% ocean before this.
        let land = self.regions.iter().filter(|r| r.biome.embarkable()).count();
        if land * 5 < n * 2 {
            return Err("not enough land");
        }
        let mountains = self
            .regions
            .iter()
            .filter(|r| r.biome == Biome::Mountains)
            .count();
        if mountains < n / 100 {
            return Err("not enough mountains for a dwarf to live in");
        }
        // Every check here was a floor, and floors alone let the opposite
        // failure straight through: mountains are embarkable, so a world that
        // is half mountain passes every "enough of X" test while having room
        // for nothing else. Measured, twelve seeds in thirty came out a quarter
        // mountain or more, one of them 54%.
        if mountains * 4 > n {
            return Err("nothing but mountain");
        }
        // And a continent with no coastline is no world either — no ports, no
        // beaches, nowhere for a caravan to come from.
        if land * 10 > n * 9 {
            return Err("no sea to speak of");
        }
        // A world of one climate is a boring world. DF rejects on distribution
        // too ("Volcanism not evenly distributed" is a named rejection).
        let kinds = self
            .regions
            .iter()
            .map(|r| RegionKind::of(r.biome))
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        if kinds < 4 {
            return Err("too few kinds of country");
        }
        if self.volcanoes.is_empty() {
            return Err("no volcanoes");
        }
        Ok(())
    }

    /// Is any neighbour of this tile higher than it? The difference between a
    /// basin (walls around it) and a summit (nothing above it).
    fn has_higher_neighbour(&self, i: usize) -> bool {
        let (x, y) = (i % self.width, i / self.width);
        let e = self.regions[i].elevation;
        Dir::ALL.iter().any(|d| {
            let (dx, dy) = d.delta();
            let (nx, ny) = (x as i32 + dx, y as i32 + dy);
            if nx < 0 || ny < 0 || nx >= self.width as i32 || ny >= self.height as i32 {
                return false;
            }
            self.regions[ny as usize * self.width + nx as usize].elevation > e
        })
    }

    /// Pick the world's high peaks and drive them up.
    ///
    /// Dwarf Fortress "select[s] points for highest peaks" as a deliberate
    /// step, and it is the reason its worlds reliably have mountains. Ours had
    /// none: the raw noise field topped out wherever it happened to, and one
    /// seed in three produced a world whose highest ground was below the
    /// mountain line — a world with no dwarven homeland in it at all.
    fn raise_peaks(&mut self, rng: &mut ChaCha8Rng) {
        let n = self.width * self.height;
        let peaks = (n / 380).max(3);
        for _ in 0..peaks {
            // Peaks belong on high ground, not out at sea: try for somewhere
            // already raised, and take the best of a few looks.
            let mut best: Option<(usize, usize, u16)> = None;
            for _ in 0..12 {
                let x = rng.gen_range(0..self.width);
                let y = rng.gen_range(0..self.height);
                let e = self.get(x, y).elevation;
                if best.is_none_or(|(_, _, be)| e > be) {
                    best = Some((x, y, e));
                }
            }
            let Some((cx, cy, _)) = best else { continue };
            // A peak and the range that falls away from it.
            let radius = rng.gen_range(3i32..7);
            for dy in -radius..=radius {
                for dx in -radius..=radius {
                    let (x, y) = (cx as i32 + dx, cy as i32 + dy);
                    if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
                        continue;
                    }
                    let d = ((dx * dx + dy * dy) as f32).sqrt();
                    if d > radius as f32 {
                        continue;
                    }
                    // Full height at the peak, tapering to nothing at the rim.
                    let lift = (1.0 - d / radius as f32).powf(1.6);
                    let i = y as usize * self.width + x as usize;
                    let want = Self::SEA_LEVEL as f32
                        + (Self::MAX_ELEVATION - Self::SEA_LEVEL) as f32 * lift;
                    let e = self.regions[i].elevation as f32;
                    self.regions[i].elevation = e.max(want).min(Self::MAX_ELEVATION as f32) as u16;
                }
            }
        }
    }

    /// Flatten the middle ground. Dwarf Fortress "smooth[s] mid-level
    /// elevations to make more plains" — without it a world is all slope and
    /// nowhere to live.
    fn smooth_midlands(&mut self) {
        let before: Vec<u16> = self.regions.iter().map(|r| r.elevation).collect();
        for y in 0..self.height {
            for x in 0..self.width {
                let i = y * self.width + x;
                let e = before[i];
                // Leave the sea and the mountains alone; plane the rest.
                if e < Self::SEA_LEVEL || e >= Self::MOUNTAIN_LEVEL {
                    continue;
                }
                let mut sum = e as u32;
                let mut count = 1u32;
                for d in Dir::ALL {
                    let (dx, dy) = d.delta();
                    let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                    if nx < 0 || ny < 0 || nx >= self.width as i32 || ny >= self.height as i32 {
                        continue;
                    }
                    sum += before[ny as usize * self.width + nx as usize] as u32;
                    count += 1;
                }
                self.regions[i].elevation = (sum / count) as u16;
            }
        }
    }

    /// Set the volcanoes.
    ///
    /// Dwarf Fortress: "a square must have volcanism exactly 100 to form one",
    /// and it "place[s] volcanoes respecting volcanism hot spots". Our
    /// volcanism was raw noise scaled to 0..100, which in practice topped out
    /// around 90 — so no square ever reached 100, no volcano could ever form,
    /// and the embark screen's VOLCANO readout was a line that could never
    /// print. The hot spots are now driven to 100, and that is where they go.
    fn place_volcanoes(&mut self, rng: &mut ChaCha8Rng) {
        let n = self.width * self.height;
        let wanted = (n / 700).max(2);
        for _ in 0..wanted {
            // The hottest ground of several looks, and it must be land.
            let mut best: Option<(usize, u8)> = None;
            for _ in 0..24 {
                let i = rng.gen_range(0..n);
                if self.regions[i].elevation < Self::SEA_LEVEL {
                    continue;
                }
                let v = self.regions[i].volcanism;
                if best.is_none_or(|(_, bv)| v > bv) {
                    best = Some((i, v));
                }
            }
            let Some((i, _)) = best else { continue };
            self.regions[i].volcanism = 100;
            // A volcano stands up out of its country.
            self.regions[i].elevation = self.regions[i].elevation.max(Self::MOUNTAIN_LEVEL);
            let (x, y) = (i % self.width, i / self.width);
            if !self.volcanoes.contains(&(x, y)) {
                self.volcanoes.push((x, y));
            }
        }
    }

    /// Revise the rain for the mountains — the pass that makes a world look
    /// like a world.
    ///
    /// Dwarf Fortress adjusts "rainfall for rain shadow and orographic
    /// precipitation" AFTER the terrain is settled, and it is why its deserts
    /// sit where they do. Ours was raw noise: rainfall had never heard of the
    /// mountains, so a range could have rainforest on both sides.
    ///
    /// The model is the real one, kept simple. Weather comes off the sea
    /// carrying water. Forced up a slope it drops what it carries — that is
    /// orographic precipitation, and it soaks the windward side. Over the
    /// crest there is nothing left to fall, and the lee is a desert: the rain
    /// shadow. The wind blows west to east here, which is our choice; Dwarf
    /// Fortress does not say what its own does.
    fn revise_rainfall(&mut self) {
        for y in 0..self.height {
            // Air arrives off the western sea, fully laden.
            let mut moisture = 1.0f32;
            for x in 0..self.width {
                let i = y * self.width + x;
                let e = self.regions[i].elevation;
                if e < Self::SEA_LEVEL {
                    moisture = 1.0; // the sea puts it back
                    continue;
                }
                // Against the LOWEST of the last few tiles upwind, not just
                // the one next door. A range is broad, and across its flat top
                // the tile-to-tile climb is zero — so measuring one step back
                // said "no climb here" all the way over a mountain and the air
                // sailed across fully laden.
                let upwind = (1..=3)
                    .filter_map(|d| x.checked_sub(d))
                    .map(|ux| self.regions[y * self.width + ux].elevation)
                    .min()
                    .unwrap_or(Self::SEA_LEVEL);
                // Only a real climb wrings the air out. A gentle rise does
                // nothing — when every slope counted, the whole continent sat
                // in a permanent shadow and the world came out one dry plain
                // from coast to coast.
                let climb = (e as i32 - upwind as i32).max(0) as f32;
                let wrung = if climb > MIN_OROGRAPHIC_CLIMB {
                    (((climb - MIN_OROGRAPHIC_CLIMB) / 110.0).min(1.0) * moisture * 0.85).max(0.0)
                } else {
                    0.0
                };
                moisture -= wrung;
                // Land gives it back as it goes, so a shadow reaches some way
                // inland and then fades — it does not last to the far coast.
                // But not up here: high ground has nothing to give, and letting
                // a range re-wet the air on its own summit undid the shadow
                // before it ever reached the lee.
                if e < Self::MOUNTAIN_LEVEL {
                    moisture = (moisture + SHADOW_RECOVERY).min(1.0);
                }
                let base = self.regions[i].rainfall as f32;
                // Dry air suppresses the local rain; climbing air adds to it.
                let revised = base * (0.18 + 0.82 * moisture) + wrung * 70.0;
                self.regions[i].rainfall = revised.clamp(0.0, 100.0) as u8;
            }
        }
    }

    /// Work out how warm it is, now that the land is finished.
    ///
    /// Dwarf Fortress recalculates temperature late, "from elevation, rainfall
    /// and forest damping", which is why it must come after the peaks are
    /// raised and the rain revised — a mountain that grew in step six is cold
    /// in step sixteen.
    fn set_temperature(&mut self, noise: &[f32]) {
        for y in 0..self.height {
            for x in 0..self.width {
                let i = y * self.width + x;
                // Two poles and a warm middle.
                let lat = y as f32 / self.height as f32;
                let from_equator = (lat - 0.5).abs() * 2.0;
                let e = self.regions[i].elevation;
                let above_sea =
                    e.saturating_sub(Self::SEA_LEVEL) as f32 / Self::MAX_ELEVATION as f32;
                // Wet air moderates: a rainy coast swings less than a dry
                // interior. (DF damps with forest; rainfall is what makes the
                // forest, and we have it to hand here.)
                let damp = self.regions[i].rainfall as f32 / 100.0;
                let t = 45.0 - from_equator.powf(1.7) * 72.0 + (noise[i] - 0.5) * 20.0
                    - above_sea * 55.0
                    + damp * 4.0;
                self.regions[i].temperature = t.clamp(-60.0, 60.0) as i16;
            }
        }
    }

    /// Ask what grows here — last, once everything it depends on is settled.
    fn classify_all(&mut self) {
        for r in &mut self.regions {
            r.biome = Self::classify(r.elevation, r.rainfall, r.drainage, r.temperature);
        }
    }

    /// Find and name the world's regions.
    ///
    /// Dwarf Fortress: "A region/subregion is a contiguous set of world tiles
    /// with the same or similar biomes AND the same alignment; the whole region
    /// is uniformly evil, neutral, or good." So we flood-fill on (kind,
    /// alignment) and give each patch a name — which is why a fort's home reads
    /// "the Forest of Whispering" instead of "region (24, 24)".
    fn name_regions(&mut self, rng: &mut ChaCha8Rng) {
        let n = self.width * self.height;
        let mut seen = vec![false; n];
        for start in 0..n {
            if seen[start] {
                continue;
            }
            let kind = RegionKind::of(self.regions[start].biome);
            let align = self.regions[start].alignment;
            let id = self.named.len();
            // Flood-fill this stretch, breadth-first from the seed tile.
            let mut queue = vec![start];
            let mut tiles = 0usize;
            seen[start] = true;
            while let Some(i) = queue.pop() {
                self.regions[i].subregion = id;
                tiles += 1;
                let (x, y) = (i % self.width, i / self.width);
                for d in Dir::ALL {
                    let (dx, dy) = d.delta();
                    let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                    if nx < 0 || ny < 0 || nx >= self.width as i32 || ny >= self.height as i32 {
                        continue;
                    }
                    let j = ny as usize * self.width + nx as usize;
                    if seen[j] {
                        continue;
                    }
                    if RegionKind::of(self.regions[j].biome) == kind
                        && self.regions[j].alignment == align
                    {
                        seen[j] = true;
                        queue.push(j);
                    }
                }
            }
            self.named.push(NamedRegion {
                name: names::region_name(rng, kind.noun()),
                kind,
                alignment: align,
                tiles,
            });
        }
    }

    /// Paint good and evil onto the land.
    ///
    /// Dwarf Fortress does not seed alignment as a field with the other six —
    /// it places it late, in whole regions, against target counts, so a stretch
    /// of country is uniformly kindly or uniformly wrong and you can feel the
    /// border when you cross it. Ours does the same: a few seed points, each
    /// flooding out over the land that shares its nature.
    ///
    /// Drawn after the rivers deliberately: `trace_rivers` uses no RNG, so the
    /// stream reaches this in a known state whatever the terrain did.
    fn place_alignment(&mut self, rng: &mut ChaCha8Rng) {
        let n = self.width * self.height;
        let blots = (n / 260).max(2); // a handful of good and a handful of evil
        for pass in 0..2 {
            let align = if pass == 0 { Alignment::Good } else { Alignment::Evil };
            for _ in 0..blots {
                let cx = rng.gen_range(0..self.width);
                let cy = rng.gen_range(0..self.height);
                if !self.get(cx, cy).biome.embarkable() {
                    continue;
                }
                let radius = rng.gen_range(2i32..5);
                for dy in -radius..=radius {
                    for dx in -radius..=radius {
                        let (x, y) = (cx as i32 + dx, cy as i32 + dy);
                        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
                            continue;
                        }
                        // A rough blot, not a disc — the border should wander.
                        if dx * dx + dy * dy > radius * radius {
                            continue;
                        }
                        let i = y as usize * self.width + x as usize;
                        if self.regions[i].biome.embarkable() {
                            self.regions[i].alignment = align;
                        }
                    }
                }
            }
        }
    }

    /// Carve a river network into the overworld, derived PURELY from the
    /// elevation and rainfall fields (no rng), so a river only flows where the
    /// land actually drains one — and downstream civ/history generation, which
    /// runs after this, is unaffected. Standard flow-direction + flow-
    /// accumulation hydrology: water runs to the lowest neighbour, and the
    /// cells that gather the most flow become rivers on their way to the sea.
    fn trace_rivers(&mut self) {
        let (w, h) = (self.width, self.height);
        let n = w * h;
        let land = |r: &Region| r.elevation >= Self::SEA_LEVEL;
        let elev = |i: usize| self.regions[i].elevation;

        // Flow direction: each land cell runs to its lowest LOWER neighbour;
        // a cell with no lower neighbour is a basin sink.
        let mut flow: Vec<Option<Dir>> = vec![None; n];
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                if !land(&self.regions[i]) {
                    continue;
                }
                let e = elev(i);
                let mut best: Option<(u16, Dir)> = None;
                for d in Dir::ALL {
                    let (dx, dy) = d.delta();
                    let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                    if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                        continue;
                    }
                    let ne = elev(ny as usize * w + nx as usize);
                    if ne < e && best.is_none_or(|(be, _)| ne < be) {
                        best = Some((ne, d));
                    }
                }
                flow[i] = best.map(|(_, d)| d);
            }
        }

        // Flow accumulation: process cells from high to low so every upstream
        // cell has passed its water down before we reach a cell. Each cell
        // starts with its own rainfall and adds it to its downhill neighbour.
        let mut order: Vec<usize> = (0..n).filter(|&i| land(&self.regions[i])).collect();
        order.sort_by(|&a, &b| elev(b).cmp(&elev(a)));
        // Each cell starts with its own rainfall (as a fraction) and hands it
        // downhill.
        let mut acc: Vec<f32> =
            (0..n).map(|i| 0.3 + self.regions[i].rainfall as f32 / 100.0).collect();
        for &i in &order {
            if let Some(d) = flow[i] {
                let (dx, dy) = d.delta();
                let (x, y) = (i % w, i / w);
                let ni = (y as i32 + dy) as usize * w + (x as i32 + dx) as usize;
                acc[ni] += acc[i];
            }
        }

        // Rivers are the land cells that gather the most flow — a modest top
        // slice, so rivers stay sparse and channelled, not everywhere.
        let mut sorted: Vec<f32> = order.iter().map(|&i| acc[i]).collect();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let river_thresh = sorted
            .get(sorted.len() * 94 / 100)
            .copied()
            .unwrap_or(f32::MAX);

        for i in 0..n {
            if land(&self.regions[i]) && acc[i] >= river_thresh {
                self.regions[i].river = true;
                self.regions[i].river_out = flow[i];
            }
            // A land basin with nowhere to drain cradles a lake — but it must
            // actually be a basin. `flow == None` only means "no neighbour is
            // strictly lower", which is also true of flat ground and of a
            // summit. Ask for real walls: somewhere around it must be HIGHER.
            // And no lake sits on a mountaintop, whatever the arithmetic says.
            if land(&self.regions[i])
                && flow[i].is_none()
                && self.regions[i].elevation < Self::MOUNTAIN_LEVEL
                && self.has_higher_neighbour(i)
            {
                self.regions[i].lake = true;
            }
        }

        // A river's inflow edge: the upstream river neighbour, that flows into
        // it, carrying the most water.
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                if !self.regions[i].river {
                    continue;
                }
                let mut best_in: Option<(f32, Dir)> = None;
                for d in Dir::ALL {
                    let (dx, dy) = d.delta();
                    let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                    if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                        continue;
                    }
                    let ni = ny as usize * w + nx as usize;
                    if self.regions[ni].river && flow[ni] == Some(d.opposite()) {
                        if best_in.is_none_or(|(ba, _)| acc[ni] > ba) {
                            best_in = Some((acc[ni], d));
                        }
                    }
                }
                self.regions[i].river_in = best_in.map(|(_, d)| d);
            }
        }
    }
}

// ------------------------------------------------------------ civilizations

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Race {
    Dwarven,
    Human,
    Elven,
    Goblin,
}

impl Race {
    pub fn name(self) -> &'static str {
        match self {
            Race::Dwarven => "dwarves",
            Race::Human => "humans",
            Race::Elven => "elves",
            Race::Goblin => "goblins",
        }
    }

    pub fn hostile(self) -> bool {
        matches!(self, Race::Goblin)
    }

    /// Where a people settles. Hills are no longer a biome of their own —
    /// they are what drainage does to open country — so the hill-dwellers take
    /// grassland and shrubland instead.
    fn home_biomes(self) -> &'static [Biome] {
        match self {
            Race::Dwarven => &[Biome::Mountains, Biome::Shrubland],
            Race::Human => &[Biome::Grassland, Biome::Savanna, Biome::Shrubland],
            Race::Elven => &[
                Biome::BroadleafForest,
                Biome::ConiferForest,
                Biome::Taiga,
            ],
            Race::Goblin => &[
                Biome::Swamp,
                Biome::Marsh,
                Biome::SandDesert,
                Biome::RockyWasteland,
                Biome::Badlands,
                Biome::Tundra,
            ],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Civilization {
    pub id: usize,
    pub name: String,
    pub race: Race,
    pub home: (usize, usize),
    pub population: u32,
    pub sites: Vec<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Site {
    pub id: usize,
    pub name: String,
    pub civ: usize,
    pub region: (usize, usize),
    pub founded_year: u32,
    pub ruined: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    Leader,
    Warlord,
    Warrior,
    Scholar,
}

impl Role {
    pub fn name(self) -> &'static str {
        match self {
            Role::Leader => "leader",
            Role::Warlord => "warlord",
            Role::Warrior => "warrior",
            Role::Scholar => "scholar",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Figure {
    pub id: usize,
    pub name: String,
    pub civ: usize,
    pub role: Role,
    /// May be negative: founding-era figures were born before year 0.
    pub born_year: i64,
    pub died_year: Option<u32>,
    pub kills: u32,
    /// (year, text) — personal reasons for hatred, referenced by sieges.
    pub grudges: Vec<(u32, String)>,
}

/// The great monsters of the world — dragons, titans, and their kin. Their
/// life and death is what carves history into named Ages, exactly as in Dwarf
/// Fortress: while they roam, the world is young and mythic; when the last is
/// slain, the Age of Heroes gives way to quieter times.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BeastKind {
    Dragon,
    Titan,
    Roc,
    Hydra,
    Colossus,
    Ettin,
    Cyclops,
    Minotaur,
}

impl BeastKind {
    pub fn noun(self) -> &'static str {
        match self {
            BeastKind::Dragon => "dragon",
            BeastKind::Titan => "titan",
            BeastKind::Roc => "roc",
            BeastKind::Hydra => "hydra",
            BeastKind::Colossus => "bronze colossus",
            BeastKind::Ettin => "ettin",
            BeastKind::Cyclops => "cyclops",
            BeastKind::Minotaur => "minotaur",
        }
    }

    pub const ALL: [BeastKind; 8] = [
        BeastKind::Dragon,
        BeastKind::Titan,
        BeastKind::Roc,
        BeastKind::Hydra,
        BeastKind::Colossus,
        BeastKind::Ettin,
        BeastKind::Cyclops,
        BeastKind::Minotaur,
    ];
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Megabeast {
    pub id: usize,
    pub name: String,
    pub kind: BeastKind,
    /// Where it lairs and where heroes must go to end it.
    pub lair: (usize, usize),
    pub born_year: i64,
    pub died_year: Option<u32>,
    /// The name of the hero who slew it, once one does.
    pub slayer: Option<String>,
    pub kills: u32,
    pub razed: u32,
}

/// A legendary object — forged by a namable hand, sometimes carried off by a
/// beast or lost in a fallen hall.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub id: usize,
    pub name: String,
    /// "a steel battle axe", "an adamantine crown".
    pub kind: String,
    pub creator: String,
    pub civ: usize,
    pub created_year: u32,
    /// Stolen by a beast or entombed in ruins — its whereabouts unknown.
    pub lost: bool,
}

/// A named span of history, in the manner of Dwarf Fortress's Ages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Age {
    pub name: String,
    pub start: u32,
    pub end: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoricalEvent {
    pub year: u32,
    pub text: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct World {
    pub seed: u64,
    pub overworld: Overworld,
    pub civs: Vec<Civilization>,
    pub sites: Vec<Site>,
    pub figures: Vec<Figure>,
    #[serde(default)]
    pub beasts: Vec<Megabeast>,
    #[serde(default)]
    pub artifacts: Vec<Artifact>,
    pub events: Vec<HistoricalEvent>,
    pub years_simulated: u32,
}

/// Uppercase the first letter — civ names begin with "the", and events
/// beginning with them need a capital.
fn cap(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

impl World {
    /// Generate the overworld, place civilizations, and simulate history.
    /// Fully deterministic in `seed`.
    pub fn generate(seed: u64, width: usize, height: usize, years: u32) -> Self {
        let mut rng = dk_core::rng_from_seed(seed ^ 0x9E37_79B9_7F4A_7C15);
        let overworld = Overworld::generate_verified(&mut rng, width, height);
        let mut world = World {
            seed,
            overworld,
            civs: Vec::new(),
            sites: Vec::new(),
            figures: Vec::new(),
            beasts: Vec::new(),
            artifacts: Vec::new(),
            events: Vec::new(),
            years_simulated: 0,
        };
        world.place_civs(&mut rng);
        world.simulate(&mut rng, years);
        world
    }

    fn event(&mut self, year: u32, text: String) {
        self.events.push(HistoricalEvent { year, text });
    }

    fn place_civs(&mut self, rng: &mut ChaCha8Rng) {
        let wanted = [
            Race::Dwarven,
            Race::Human,
            Race::Elven,
            Race::Goblin,
            Race::Goblin, // a second goblin civ keeps the borders ugly
            Race::Human,
        ];
        for race in wanted {
            // Find a home region of a suitable biome, far from other civs.
            let mut best: Option<(usize, usize, u32)> = None;
            for _ in 0..200 {
                let x = rng.gen_range(0..self.overworld.width);
                let y = rng.gen_range(0..self.overworld.height);
                if !race.home_biomes().contains(&self.overworld.get(x, y).biome) {
                    continue;
                }
                let dist = self
                    .civs
                    .iter()
                    .map(|c| c.home.0.abs_diff(x) as u32 + c.home.1.abs_diff(y) as u32)
                    .min()
                    .unwrap_or(u32::MAX);
                if best.is_none_or(|(_, _, bd)| dist > bd) {
                    best = Some((x, y, dist));
                }
            }
            let Some((x, y, _)) = best else { continue };
            let id = self.civs.len();
            let name = names::civ_name(rng, race);
            self.event(0, format!("The {} of {} arose in the {}.", race.name(), name, self.overworld.get(x, y).biome.name()));
            self.civs.push(Civilization {
                id,
                name,
                race,
                home: (x, y),
                population: rng.gen_range(300..900),
                sites: Vec::new(),
            });
            self.found_site(rng, id, (x, y), 0);
        }
    }

    fn found_site(&mut self, rng: &mut ChaCha8Rng, civ: usize, near: (usize, usize), year: u32) {
        // A site lands in or next to `near`, on land.
        let mut spot = near;
        for _ in 0..20 {
            let dx = rng.gen_range(-3i64..=3);
            let dy = rng.gen_range(-3i64..=3);
            let x = (near.0 as i64 + dx).clamp(0, self.overworld.width as i64 - 1) as usize;
            let y = (near.1 as i64 + dy).clamp(0, self.overworld.height as i64 - 1) as usize;
            if self.overworld.get(x, y).biome.embarkable() {
                spot = (x, y);
                break;
            }
        }
        let id = self.sites.len();
        let race = self.civs[civ].race;
        let name = names::site_name(rng, race);
        self.event(
            year,
            format!("{} of the {} founded {}.", cap(&self.civs[civ].name), race.name(), name),
        );
        self.sites.push(Site { id, name, civ, region: spot, founded_year: year, ruined: false });
        self.civs[civ].sites.push(id);
    }

    fn spawn_figure(&mut self, rng: &mut ChaCha8Rng, civ: usize, role: Role, year: u32) -> usize {
        let id = self.figures.len();
        let name = names::figure_name(rng, self.civs[civ].race);
        if role != Role::Warrior {
            self.event(
                year,
                format!("{} rose to prominence as a {} of {}.", name, role.name(), self.civs[civ].name),
            );
        }
        self.figures.push(Figure {
            id,
            name,
            civ,
            role,
            born_year: year as i64 - rng.gen_range(18..40),
            died_year: None,
            kills: 0,
            grudges: Vec::new(),
        });
        id
    }

    /// The name of the country a region belongs to — for beast lairs and deeds.
    fn region_name_at(&self, at: (usize, usize)) -> String {
        let sub = self.overworld.get(at.0, at.1).subregion;
        self.overworld.named[sub].name.clone()
    }

    /// Loose a handful of great beasts upon the young world, each lairing in
    /// wild, high, or sinister country far from the hearths of the civilized.
    fn spawn_beasts(&mut self, rng: &mut ChaCha8Rng) {
        let count = 4 + rng.gen_range(0..4); // 4..=7 great beasts
        for _ in 0..count {
            let mut lair = (
                rng.gen_range(0..self.overworld.width),
                rng.gen_range(0..self.overworld.height),
            );
            // Prefer a mountain fastness, a savage waste, or an evil land.
            for _ in 0..24 {
                let x = rng.gen_range(0..self.overworld.width);
                let y = rng.gen_range(0..self.overworld.height);
                let r = self.overworld.get(x, y);
                if r.biome.embarkable()
                    && (r.biome == Biome::Mountains
                        || r.savagery >= 66
                        || r.alignment == Alignment::Evil)
                {
                    lair = (x, y);
                    break;
                }
            }
            let kind = BeastKind::ALL[rng.gen_range(0..BeastKind::ALL.len())];
            let name = names::beast_name(rng);
            let born_year = -(rng.gen_range(50i64..400));
            let id = self.beasts.len();
            let where_ = self.region_name_at(lair);
            self.event(
                0,
                format!("In the first age, {}, a {}, awoke in {}.", name, kind.noun(), where_),
            );
            self.beasts.push(Megabeast {
                id,
                name,
                kind,
                lair,
                born_year,
                died_year: None,
                slayer: None,
                kills: 0,
                razed: 0,
            });
        }
    }

    /// One year in the lives of the great beasts: a rampage, and the rise of a
    /// hero who marches on a lair to end one (and usually does).
    fn beast_year(&mut self, rng: &mut ChaCha8Rng, year: u32) {
        let living: Vec<usize> =
            self.beasts.iter().filter(|b| b.died_year.is_none()).map(|b| b.id).collect();
        if living.is_empty() {
            return;
        }

        // A rampage against the nearest standing settlement.
        if rng.gen_ratio(1, 3) {
            let bid = living[rng.gen_range(0..living.len())];
            let lair = self.beasts[bid].lair;
            if let Some(site_id) = self
                .sites
                .iter()
                .filter(|s| !s.ruined)
                .min_by_key(|s| s.region.0.abs_diff(lair.0) + s.region.1.abs_diff(lair.1))
                .map(|s| s.id)
            {
                let dead = rng.gen_range(3..40);
                let civ = self.sites[site_id].civ;
                self.civs[civ].population =
                    self.civs[civ].population.saturating_sub(dead).max(50);
                self.beasts[bid].kills += dead;
                let bname = self.beasts[bid].name.clone();
                let kind = self.beasts[bid].kind.noun();
                let sname = self.sites[site_id].name.clone();
                if dead > 18 && rng.gen_ratio(1, 2) {
                    self.sites[site_id].ruined = true;
                    self.beasts[bid].razed += 1;
                    self.event(
                        year,
                        format!(
                            "{}, the {}, descended upon {} and laid it to ruin, {} slain.",
                            bname, kind, sname, dead
                        ),
                    );
                } else {
                    self.event(
                        year,
                        format!(
                            "{}, the {}, fell upon {}; {} were slain before it withdrew.",
                            bname, kind, sname, dead
                        ),
                    );
                }
            }
        }

        // A hero marches on a lair. Beasts are hardy and heroes rare, so the
        // great monsters cast a long shadow over the early ages before they
        // fall. Many who march do not return.
        if rng.gen_ratio(1, 8) {
            let bid = living[rng.gen_range(0..living.len())];
            let civs: Vec<usize> =
                self.civs.iter().filter(|c| !c.race.hostile()).map(|c| c.id).collect();
            if civs.is_empty() {
                return;
            }
            let civ = civs[rng.gen_range(0..civs.len())];
            let hero = self.spawn_figure(rng, civ, Role::Warrior, year);
            let bname = self.beasts[bid].name.clone();
            let kind = self.beasts[bid].kind.noun();
            let where_ = self.region_name_at(self.beasts[bid].lair);
            let civ_name = self.civs[civ].name.clone();
            if rng.gen_ratio(2, 3) {
                self.beasts[bid].died_year = Some(year);
                self.figures[hero].kills += 1;
                let hname = self.figures[hero].name.clone();
                self.beasts[bid].slayer = Some(hname.clone());
                self.event(
                    year,
                    format!("{} of {} slew {}, the {}, in {}.", hname, civ_name, bname, kind, where_),
                );
                // A deed of legend sometimes yields a trophy of legend.
                if rng.gen_ratio(1, 2) {
                    let occ = format!("to mark the slaying of {}", bname);
                    self.forge_named_artifact(rng, civ, hname, year, &occ);
                }
            } else {
                self.figures[hero].died_year = Some(year);
                self.beasts[bid].kills += 1;
                let hname = self.figures[hero].name.clone();
                self.event(
                    year,
                    format!(
                        "{} of {} marched on {}, the {}, and perished in {}.",
                        hname, civ_name, bname, kind, where_
                    ),
                );
            }
        }
    }

    /// A living, named figure forges an artifact out of the ordinary run of days.
    fn forge_artifact(&mut self, rng: &mut ChaCha8Rng, year: u32) {
        let makers: Vec<usize> =
            self.figures.iter().filter(|f| f.died_year.is_none()).map(|f| f.id).collect();
        if makers.is_empty() {
            return;
        }
        let f = makers[rng.gen_range(0..makers.len())];
        let (name, civ) = (self.figures[f].name.clone(), self.figures[f].civ);
        self.forge_named_artifact(rng, civ, name, year, "");
    }

    /// Mint a named artifact by `creator` of `civ`, recording it in the annals.
    fn forge_named_artifact(
        &mut self,
        rng: &mut ChaCha8Rng,
        civ: usize,
        creator: String,
        year: u32,
        occasion: &str,
    ) {
        let id = self.artifacts.len();
        let aname = names::artifact_name(rng);
        let kind = names::artifact_kind(rng);
        let civ_name = self.civs[civ].name.clone();
        let tail = if occasion.is_empty() {
            ".".to_string()
        } else {
            format!(", {}.", occasion)
        };
        self.event(year, format!("{} of {} forged {}, {}{}", creator, civ_name, aname, kind, tail));
        self.artifacts.push(Artifact {
            id,
            name: aname,
            kind,
            creator,
            civ,
            created_year: year,
            lost: false,
        });
    }

    fn simulate(&mut self, rng: &mut ChaCha8Rng, years: u32) {
        if self.civs.is_empty() {
            // A barren world (no suitable biomes) has no history to tell.
            self.event(0, "No peoples ever arose in this desolate world.".to_string());
            self.years_simulated = years;
            return;
        }
        // Seed each civ with a leader and a few notables.
        for c in 0..self.civs.len() {
            self.spawn_figure(rng, c, Role::Leader, 0);
            let extra = match self.civs[c].race {
                Race::Goblin => Role::Warlord,
                Race::Elven => Role::Scholar,
                _ => Role::Warrior,
            };
            self.spawn_figure(rng, c, extra, 0);
        }
        // And loose the great beasts upon the young world.
        self.spawn_beasts(rng);

        for year in 1..=years {
            self.simulate_year(rng, year);
        }
        self.years_simulated = years;

        // Every hostile civ ends history with at least one living
        // grudge-bearer, so an embarking fortress always has a named enemy.
        let hostiles: Vec<usize> = self
            .civs
            .iter()
            .filter(|c| c.race.hostile())
            .map(|c| c.id)
            .collect();
        for c in hostiles {
            if !self.siege_leaders(c).is_empty() {
                continue;
            }
            let target = self
                .civs
                .iter()
                .find(|v| !v.race.hostile())
                .map(|v| v.name.clone())
                .unwrap_or_else(|| "the soft peoples of the lowlands".to_string());
            let id = self.spawn_figure(rng, c, Role::Warlord, years);
            let reason = format!("nursed an old burning hatred of {} from the wars of years past", target);
            self.figures[id].grudges.push((years, reason.clone()));
            let name = self.figures[id].name.clone();
            self.event(years, format!("{} {}.", name, reason));
        }
    }

    /// One year of the world's life: populations drift, sites are founded,
    /// figures rise and die, and the hostile civs go raiding. Worldgen runs
    /// this `years` times up front; a live fortress runs it once a year
    /// through `advance_year`, so the same machinery writes the ancient past
    /// and the news of today.
    fn simulate_year(&mut self, rng: &mut ChaCha8Rng, year: u32) {
        if self.civs.is_empty() {
            return;
        }

        // Populations drift; prosperous civs found new sites.
        for c in 0..self.civs.len() {
            let growth = rng.gen_range(-30i32..60);
            let civ = &mut self.civs[c];
            civ.population = civ.population.saturating_add_signed(growth).max(50);
            if civ.population > 600 && rng.gen_ratio(1, 6) {
                let near = civ.home;
                civ.population -= 150;
                self.found_site(rng, c, near, year);
            }
        }

        // New figures appear now and then.
        if rng.gen_ratio(1, 2) {
            let c = rng.gen_range(0..self.civs.len());
            let role = if self.civs[c].race == Race::Goblin && rng.gen_ratio(1, 3) {
                Role::Warlord
            } else {
                Role::Warrior
            };
            self.spawn_figure(rng, c, role, year);
        }

        // Wars: goblin civs raid their neighbors.
        if rng.gen_ratio(1, 3) {
            let attackers: Vec<usize> = self
                .civs
                .iter()
                .filter(|c| c.race.hostile())
                .map(|c| c.id)
                .collect();
            let defenders: Vec<usize> = self
                .civs
                .iter()
                .filter(|c| !c.race.hostile())
                .map(|c| c.id)
                .collect();
            if let (Some(&a), Some(&d)) = (
                attackers.get(rng.gen_range(0..attackers.len().max(1))),
                defenders.get(rng.gen_range(0..defenders.len().max(1))),
            ) {
                self.battle(rng, a, d, year);
            }
        }

        // The great beasts stir: a living megabeast may descend on a settlement,
        // and now and then a hero rises to end one — the pulse that drives the
        // Ages of the world.
        self.beast_year(rng, year);

        // Now and then a namable hand forges something the world remembers.
        if rng.gen_ratio(1, 5) {
            self.forge_artifact(rng, year);
        }

        // Mortality among the named.
        for f in 0..self.figures.len() {
            if self.figures[f].died_year.is_none()
                && year as i64 - self.figures[f].born_year > 50
                && rng.gen_ratio(1, 12)
            {
                self.figures[f].died_year = Some(year);
                let (name, civ) = (self.figures[f].name.clone(), self.civs[self.figures[f].civ].name.clone());
                self.event(year, format!("{} of {} died of old age.", name, civ));
            }
        }
    }

    /// Live the world forward by one year while a fortress plays, and return
    /// the fresh events as news. The world does not hold still for you: wars
    /// grind on, sites burn, and the warlord who hates you may die in a ditch
    /// somewhere before he ever reaches your gates.
    ///
    /// Determinism comes from a per-year RNG derived from the world seed, so
    /// no stream state has to survive a save round-trip.
    pub fn advance_year(&mut self) -> Vec<String> {
        let year = self.years_simulated + 1;
        let mut rng = dk_core::rng_from_seed(
            self.seed ^ 0x11FE_0F17_5E17_0000 ^ (year as u64).wrapping_mul(0x9E37_79B9),
        );
        let before = self.events.len();
        self.simulate_year(&mut rng, year);
        self.years_simulated = year;
        self.events[before..].iter().map(|e| e.text.clone()).collect()
    }

    /// A raid: casualties on both sides, kills credited to named figures,
    /// grudges sworn over the fallen.
    fn battle(&mut self, rng: &mut ChaCha8Rng, attacker: usize, defender: usize, year: u32) {
        let Some(&site_id) = self.sites.iter().find(|s| s.civ == defender && !s.ruined).map(|s| &s.id)
        else {
            return;
        };
        let site_name = self.sites[site_id].name.clone();
        let a_name = self.civs[attacker].name.clone();
        let d_name = self.civs[defender].name.clone();
        let a_deaths = rng.gen_range(5..80);
        let d_deaths = rng.gen_range(5..80);
        self.civs[attacker].population = self.civs[attacker].population.saturating_sub(a_deaths).max(50);
        self.civs[defender].population = self.civs[defender].population.saturating_sub(d_deaths).max(50);
        self.event(
            year,
            format!(
                "{} raided {}: {} attackers and {} defenders fell.",
                cap(&a_name), site_name, a_deaths, d_deaths
            ),
        );

        // A catastrophic defeat can raze the site outright.
        if d_deaths > 60 && rng.gen_ratio(1, 3) {
            self.sites[site_id].ruined = true;
            self.event(
                year,
                format!("{} was razed and left in ruins by {}.", site_name, a_name),
            );
        }

        // A named attacker earns kills — or falls, and is avenged. War
        // always produces someone to remember it: mint a warlord if the
        // civ's named figures have all died.
        let mut living: Vec<usize> = self
            .figures
            .iter()
            .filter(|f| f.civ == attacker && f.died_year.is_none())
            .map(|f| f.id)
            .collect();
        if living.is_empty() {
            living.push(self.spawn_figure(rng, attacker, Role::Warlord, year));
        }
        let champ = living[rng.gen_range(0..living.len())];
        if rng.gen_ratio(3, 4) {
            let n = rng.gen_range(1..6);
            self.figures[champ].kills += n;
            let name = self.figures[champ].name.clone();
            self.event(year, format!("{} slew {} defenders at {}.", name, n, site_name));
            // Victory breeds contempt: swear a grudge against the defenders.
            if rng.gen_ratio(3, 4) {
                let reason = format!(
                    "swore destruction upon {} after the raid on {} in year {}",
                    d_name, site_name, year
                );
                self.figures[champ].grudges.push((year, reason.clone()));
                let name = self.figures[champ].name.clone();
                self.event(year, format!("{} {}.", name, reason));
            }
        } else {
            // The champion falls; a comrade swears vengeance.
            self.figures[champ].died_year = Some(year);
            let fallen = self.figures[champ].name.clone();
            self.event(year, format!("{} was struck down at {}.", fallen, site_name));
            let comrades: Vec<usize> = self
                .figures
                .iter()
                .filter(|f| f.civ == attacker && f.died_year.is_none())
                .map(|f| f.id)
                .collect();
            if let Some(&avenger) = comrades.first() {
                let reason = format!(
                    "swore vengeance upon {} for the death of {} at {} in year {}",
                    d_name, fallen, site_name, year
                );
                self.figures[avenger].grudges.push((year, reason.clone()));
                let name = self.figures[avenger].name.clone();
                self.event(year, format!("{} {}.", name, reason));
            }
        }
    }

    // -------------------------------------------------------------- queries

    /// Friendly civ nearest to a map position (for embark caravan wiring).
    pub fn nearest_friendly_civ(&self, x: usize, y: usize) -> Option<&Civilization> {
        self.civs
            .iter()
            .filter(|c| !c.race.hostile())
            .min_by_key(|c| c.home.0.abs_diff(x) + c.home.1.abs_diff(y))
    }

    /// Hostile civ nearest to a map position (for embark siege wiring).
    pub fn nearest_hostile_civ(&self, x: usize, y: usize) -> Option<&Civilization> {
        self.civs
            .iter()
            .filter(|c| c.race.hostile())
            .min_by_key(|c| c.home.0.abs_diff(x) + c.home.1.abs_diff(y))
    }

    /// Living figures of a civ with grudges — siege leaders in waiting,
    /// most aggrieved first.
    pub fn siege_leaders(&self, civ: usize) -> Vec<&Figure> {
        let mut leaders: Vec<&Figure> = self
            .figures
            .iter()
            .filter(|f| f.civ == civ && f.died_year.is_none() && !f.grudges.is_empty())
            .collect();
        leaders.sort_by_key(|f| std::cmp::Reverse((f.grudges.len(), f.kills)));
        leaders
    }

    /// Everything needed to wire sieges for an embark at (x, y): the
    /// nearest hostile civ's name and its grudge-bearers as
    /// (leader name, latest grudge). One glue point for app and tests.
    pub fn siege_pack(&self, x: usize, y: usize) -> Option<(String, Vec<(String, String)>)> {
        let civ = self.nearest_hostile_civ(x, y)?;
        let leaders = self
            .siege_leaders(civ.id)
            .into_iter()
            .map(|f| {
                (
                    f.name.clone(),
                    f.grudges.last().map(|(_, g)| g.clone()).unwrap_or_default(),
                )
            })
            .collect();
        Some((civ.name.clone(), leaders))
    }

    /// All legends lines, oldest first, for the viewer.
    /// Inscribe a fresh deed — an adventurer's feat — into the annals, so it
    /// takes its place in Legends alongside the deeds of ages past.
    pub fn record_deed(&mut self, year: u32, text: String) {
        self.events.push(HistoricalEvent { year, text });
    }

    /// Has this civilization been wiped from the map? A people with every
    /// site in ruins sends no more caravans and musters no more sieges.
    /// Named rather than indexed because a fortress remembers its neighbors
    /// by name across a save round-trip.
    pub fn civ_fallen(&self, name: &str) -> bool {
        let Some(civ) = self.civs.iter().find(|c| c.name == name) else {
            return true;
        };
        !civ.sites.is_empty() && civ.sites.iter().all(|&s| self.sites[s].ruined)
    }

    pub fn legends_lines(&self) -> Vec<String> {
        self.events
            .iter()
            .map(|e| format!("Year {:>3}: {}", e.year, e.text))
            .collect()
    }

    /// The named Ages of the world, carved by the life and death of its great
    /// beasts — Dwarf Fortress's device for giving a history shape. While every
    /// beast still roams it is the Age of Myth; as they fall it passes through
    /// Legends and Heroes; when the last is slain, the Age of the Sword begins.
    pub fn ages(&self) -> Vec<Age> {
        let total = self.beasts.len();
        if total == 0 {
            return vec![Age {
                name: "the Age of Civilization".into(),
                start: 0,
                end: self.years_simulated,
            }];
        }
        let mut deaths: Vec<u32> = self.beasts.iter().filter_map(|b| b.died_year).collect();
        deaths.sort_unstable();
        let living_at = |y: u32| total - deaths.iter().filter(|&&d| d <= y).count();
        let age_name = |living: usize| -> &'static str {
            if living == total {
                "the Age of Myth"
            } else if living * 2 > total {
                "the Age of Legends"
            } else if living > 0 {
                "the Age of Heroes"
            } else {
                "the Age of the Sword"
            }
        };
        let mut ages: Vec<Age> = Vec::new();
        let mut cur = age_name(living_at(0)).to_string();
        let mut start = 0u32;
        for y in 1..=self.years_simulated {
            let next = age_name(living_at(y)).to_string();
            if next != cur {
                ages.push(Age { name: std::mem::replace(&mut cur, next), start, end: y });
                start = y;
            }
        }
        ages.push(Age { name: cur, start, end: self.years_simulated });
        ages
    }

    /// The age the world stands in now — for the embark header.
    pub fn current_age(&self) -> String {
        self.ages()
            .last()
            .map(|a| a.name.clone())
            .unwrap_or_else(|| "the Age of Myth".to_string())
    }

    /// One line per Age, for the Legends browser.
    pub fn ages_lines(&self) -> Vec<String> {
        self.ages()
            .iter()
            .map(|a| format!("{} — year {} to {}", a.name, a.start, a.end))
            .collect()
    }

    /// One line per great beast: where it lairs, or who laid it low.
    pub fn beast_lines(&self) -> Vec<String> {
        self.beasts
            .iter()
            .map(|b| match (&b.slayer, b.died_year) {
                (Some(slayer), Some(dy)) => format!(
                    "{}, the {} — {} slain, {} sites razed; slain by {} in year {}",
                    b.name, b.kind.noun(), b.kills, b.razed, slayer, dy
                ),
                _ => format!(
                    "{}, the {} — still stalks {} ({} slain, {} sites razed)",
                    b.name,
                    b.kind.noun(),
                    self.region_name_at(b.lair),
                    b.kills,
                    b.razed
                ),
            })
            .collect()
    }

    /// One line per artifact, for the Legends browser.
    pub fn artifact_lines(&self) -> Vec<String> {
        self.artifacts
            .iter()
            .map(|a| {
                let lost = if a.lost { " (now lost)" } else { "" };
                format!("{}, {}, forged by {} in year {}{}", a.name, a.kind, a.creator, a.created_year, lost)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn same_seed_same_world() {
        let a = World::generate(42, 48, 48, 60);
        let b = World::generate(42, 48, 48, 60);
        assert_eq!(a.legends_lines(), b.legends_lines());
        assert_eq!(a.civs.len(), b.civs.len());
        assert_eq!(
            a.figures.iter().map(|f| &f.name).collect::<Vec<_>>(),
            b.figures.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn different_seeds_different_worlds() {
        let a = World::generate(1, 48, 48, 60);
        let b = World::generate(2, 48, 48, 60);
        let names_a: BTreeSet<&String> = a.civs.iter().map(|c| &c.name).collect();
        let names_b: BTreeSet<&String> = b.civs.iter().map(|c| &c.name).collect();
        assert_ne!(names_a, names_b, "civ names must differ between seeds");
        let biomes_a: Vec<Biome> = a.overworld.regions.iter().map(|r| r.biome).collect();
        let biomes_b: Vec<Biome> = b.overworld.regions.iter().map(|r| r.biome).collect();
        assert_ne!(biomes_a, biomes_b, "terrain must differ between seeds");
    }

    #[test]
    fn the_world_lives_on_while_a_fortress_plays() {
        let mut w = World::generate(7, 48, 48, 80);
        assert_eq!(w.years_simulated, 80);
        let before = w.events.len();
        let mut news = Vec::new();
        for _ in 0..25 {
            news.extend(w.advance_year());
        }
        assert_eq!(w.years_simulated, 105, "25 played years age the world 25 years");
        assert!(
            w.events.len() > before,
            "a quarter-century of history writes fresh events"
        );
        assert!(!news.is_empty(), "some of those years produce news to report");
        assert!(
            w.events[before..].iter().all(|e| e.year > 80),
            "live events are stamped with the years they happened in"
        );
    }

    #[test]
    fn live_years_are_deterministic() {
        let mut a = World::generate(9, 48, 48, 60);
        let mut b = World::generate(9, 48, 48, 60);
        for _ in 0..15 {
            assert_eq!(a.advance_year(), b.advance_year());
        }
        assert_eq!(a.legends_lines(), b.legends_lines());
        assert_eq!(
            a.figures.iter().map(|f| &f.name).collect::<Vec<_>>(),
            b.figures.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_civ_with_every_site_in_ruins_has_fallen() {
        let mut w = World::generate(3, 48, 48, 60);
        let name = w.civs[0].name.clone();
        assert!(!w.civ_fallen(&name), "a civ with standing sites lives");
        for s in w.civs[0].sites.clone() {
            w.sites[s].ruined = true;
        }
        assert!(w.civ_fallen(&name), "every site razed means the people are gone");
        assert!(w.civ_fallen("the Nonexistent Horde"), "an unknown civ is no civ at all");
    }

    #[test]
    fn an_adventurers_deed_is_written_into_legends() {
        let mut w = World::generate(11, 48, 48, 80);
        let before = w.events.len();
        w.record_deed(81, "Urist slew the beast Gorlak in single combat".to_string());
        assert_eq!(w.events.len(), before + 1, "the deed is recorded as an event");
        let lines = w.legends_lines();
        assert!(
            lines.iter().any(|l| l.contains("slew the beast Gorlak")),
            "the fresh deed reads back in Legends alongside ancient history"
        );
        assert!(
            lines.last().unwrap().contains("Year  81"),
            "the deed lands at the year it was done"
        );
    }

    #[test]
    fn history_produces_civs_figures_and_grudges() {
        let w = World::generate(7, 48, 48, 80);
        assert!(w.civs.len() >= 4, "world should host several civilizations");
        assert!(w.civs.iter().any(|c| c.race.hostile()), "somebody must hate you");
        assert!(w.events.len() > 30, "eighty years should leave a record");
        assert!(!w.figures.is_empty());

        let hostile = w.nearest_hostile_civ(24, 24).expect("a hostile civ exists");
        let leaders = w.siege_leaders(hostile.id);
        assert!(
            !leaders.is_empty(),
            "80 years of goblin raiding should mint at least one grudge-bearer"
        );
        // The grudge is findable in the legends record.
        let lines = w.legends_lines();
        let leader = leaders[0];
        assert!(
            lines.iter().any(|l| l.contains(&leader.name)),
            "siege leader {} must appear in legends",
            leader.name
        );
        assert!(
            lines.iter().any(|l| l.contains(&leader.name)
                && (l.contains("vengeance") || l.contains("destruction") || l.contains("hatred"))),
            "the leader's grudge must be readable in legends"
        );
    }

    #[test]
    fn overworld_has_varied_embarkable_land() {
        let w = World::generate(99, 48, 48, 10);
        let embarkable = w
            .overworld
            .regions
            .iter()
            .filter(|r| r.biome.embarkable())
            .count();
        assert!(embarkable > 48 * 48 / 4, "at least a quarter of the world is land");
        let distinct: BTreeSet<u8> = w
            .overworld
            .regions
            .iter()
            .map(|r| r.biome as u8)
            .collect();
        assert!(distinct.len() >= 4, "a world needs varied biomes, got {distinct:?}");
    }
}
