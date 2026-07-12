//! Weather exit tests, run headlessly: the sky turns with the seasons, and
//! rain quickens the crops.

mod common;

use dk_agents::{BuildingKind, FarmState, Sim, Weather};
use dk_core::{Season, DAYS_PER_SEASON, TICKS_PER_DAY};
use dk_world::path::Pos;

fn weather_fort(seed: u64) -> (Sim, dk_raws::Raws) {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, seed);
    let mut sim = Sim::new(map, &raws, rng, 4);
    sim.invasions = false;
    let _ = BuildingKind::Still;
    (sim, raws)
}

#[test]
fn the_sky_turns_with_the_seasons() {
    let (mut sim, raws) = weather_fort(1901);
    // Walk a full year, recording which weathers appear in each season.
    let mut winter_snow = false;
    let mut wet_rain = false;
    for _ in 0..(TICKS_PER_DAY * DAYS_PER_SEASON * 4) {
        sim.step(&raws);
        match sim.clock.season() {
            Season::Winter if sim.weather == Weather::Snow => winter_snow = true,
            Season::Spring | Season::Autumn if sim.weather == Weather::Rain => wet_rain = true,
            _ => {}
        }
    }
    assert!(winter_snow, "winter should bring snow");
    assert!(wet_rain, "spring/autumn should bring rain");
}

#[test]
fn rain_quickens_the_crops() {
    // Two identical farms stepped the same number of ticks — one under
    // forced rain, one under forced clear — the rained crop grows further.
    let grow = |weather: Weather| -> u32 {
        let (mut sim, raws) = weather_fort(1902);
        let cx = sim.map.width as i32 / 2;
        let cy = sim.map.height as i32 / 2;
        let barley = raws.plants.index_of("barley").unwrap();
        let (fa, _) = sim.find_flat_patch(cx, cy).unwrap();
        sim.add_farm(fa, fa, barley);
        // Plant it into the growing state up front so both runs are
        // identical except for the weather.
        if let Some(t) = sim.farms.get_mut(&fa) {
            t.state = FarmState::Growing { progress: 0 };
            t.reserved = true; // keep a dwarf from re-planting/harvesting it
        }
        // Force the whole run's weather by pinning it before each step
        // (the daily roll happens after grow_farms, so the pin holds).
        for _ in 0..(TICKS_PER_DAY * 6) {
            sim.weather = weather;
            sim.step(&raws);
        }
        match sim.farms.get(&fa).map(|t| t.state) {
            Some(FarmState::Growing { progress }) => progress,
            Some(FarmState::Grown) => u32::MAX, // fully grown counts as "more"
            _ => 0,
        }
    };
    let rained = grow(Weather::Rain);
    let dry = grow(Weather::Clear);
    assert!(rained > dry, "rain should grow crops faster ({rained} vs {dry})");
    let _ = Pos::new(0, 0, 0);
}
