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

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Region {
    pub elevation: f32, // 0..1, sea level ~0.35
    pub temperature: f32, // 0..1 cold..hot
    pub rainfall: f32, // 0..1
    pub biome: Biome,
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
                regions.push(Region { elevation, temperature, rainfall, biome });
            }
        }
        Overworld { width, height, regions }
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
            if !self.civs.is_empty() && rng.gen_ratio(1, 2) {
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
                    let (name, civ) = (self.figures[f].name.clone(), self.figures[f].civ);
                    self.event(year, format!("{} of {} died of old age.", name, self.civs[civ].name));
                }
            }
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
