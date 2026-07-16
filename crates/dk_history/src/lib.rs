//! The world outside the fortress: overworld regions, civilizations, and a
//! history simulation whose events feed Legends mode (BLUEPRINT.md §4.6).
//!
//! Everything here is derived purely from a seed — the world is regenerated
//! identically at every launch, so only the fortress sim needs saving.

use rand::Rng;
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};

pub mod names;

// ---------------------------------------------------------------- overworld

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Biome {
    Ocean,
    Mountains,
    Hills,
    Grassland,
    Forest,
    Desert,
    Swamp,
    Tundra,
}

impl Biome {
    pub fn name(self) -> &'static str {
        match self {
            Biome::Ocean => "ocean",
            Biome::Mountains => "mountains",
            Biome::Hills => "hills",
            Biome::Grassland => "grassland",
            Biome::Forest => "forest",
            Biome::Desert => "desert",
            Biome::Swamp => "swamp",
            Biome::Tundra => "tundra",
        }
    }

    /// Can a fortress embark here?
    pub fn embarkable(self) -> bool {
        !matches!(self, Biome::Ocean)
    }

    /// Display color, sRGB 0-255.
    pub fn color(self) -> [u8; 3] {
        match self {
            Biome::Ocean => [24, 48, 110],
            Biome::Mountains => [128, 124, 120],
            Biome::Hills => [110, 120, 72],
            Biome::Grassland => [96, 140, 60],
            Biome::Forest => [40, 96, 44],
            Biome::Desert => [198, 174, 106],
            Biome::Swamp => [64, 84, 58],
            Biome::Tundra => [176, 188, 196],
        }
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
    pub elevation: f32, // 0..1, sea level ~0.35
    pub temperature: f32, // 0..1 cold..hot
    pub rainfall: f32, // 0..1
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
}

impl Overworld {
    pub fn get(&self, x: usize, y: usize) -> &Region {
        &self.regions[y * self.width + x]
    }

    fn generate(rng: &mut ChaCha8Rng, width: usize, height: usize) -> Self {
        // Coarse random grids, bilinearly interpolated — same trick as the
        // local map, at world scale.
        let field = |rng: &mut ChaCha8Rng, coarse: usize| -> Vec<f32> {
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
                    out[y * width + x] = top * (1.0 - ty) + bot * ty;
                }
            }
            out
        };
        let elevation = field(rng, 16);
        let rainfall = field(rng, 12);
        let temp_noise = field(rng, 24);

        let mut regions = Vec::with_capacity(width * height);
        for y in 0..height {
            for x in 0..width {
                let i = y * width + x;
                let elevation = elevation[i];
                // Temperature: latitude gradient plus noise, colder uphill.
                let lat = y as f32 / height as f32;
                let temperature =
                    (lat * 0.7 + temp_noise[i] * 0.3 - (elevation - 0.35).max(0.0) * 0.5)
                        .clamp(0.0, 1.0);
                let rainfall = rainfall[i];
                let biome = if elevation < 0.35 {
                    Biome::Ocean
                } else if elevation > 0.75 {
                    Biome::Mountains
                } else if temperature < 0.22 {
                    Biome::Tundra
                } else if rainfall < 0.25 {
                    Biome::Desert
                } else if rainfall > 0.75 && temperature > 0.5 {
                    Biome::Swamp
                } else if rainfall > 0.55 {
                    Biome::Forest
                } else if elevation > 0.6 {
                    Biome::Hills
                } else {
                    Biome::Grassland
                };
                regions.push(Region {
                    elevation,
                    temperature,
                    rainfall,
                    biome,
                    river: false,
                    river_in: None,
                    river_out: None,
                    lake: false,
                });
            }
        }
        let mut world = Overworld { width, height, regions };
        world.trace_rivers();
        world
    }

    const SEA_LEVEL: f32 = 0.35;

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
                let mut best: Option<(f32, Dir)> = None;
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
        order.sort_by(|&a, &b| elev(b).partial_cmp(&elev(a)).unwrap_or(std::cmp::Ordering::Equal));
        let mut acc: Vec<f32> = (0..n).map(|i| 0.3 + self.regions[i].rainfall).collect();
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
            // A land basin with nowhere to drain cradles a lake.
            if land(&self.regions[i]) && flow[i].is_none() {
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

    fn home_biomes(self) -> &'static [Biome] {
        match self {
            Race::Dwarven => &[Biome::Mountains, Biome::Hills],
            Race::Human => &[Biome::Grassland, Biome::Hills],
            Race::Elven => &[Biome::Forest],
            Race::Goblin => &[Biome::Swamp, Biome::Desert, Biome::Hills, Biome::Tundra],
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
        let overworld = Overworld::generate(&mut rng, width, height);
        let mut world = World {
            seed,
            overworld,
            civs: Vec::new(),
            sites: Vec::new(),
            figures: Vec::new(),
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
