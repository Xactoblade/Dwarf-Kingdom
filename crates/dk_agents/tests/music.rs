//! Music exit tests, run headlessly: a carpenter works logs into instruments,
//! and once the fort has one, a musician plays songs at the tavern — a growing
//! repertoire, drawn deterministically from the fort's own life (no rng). A
//! tavern with no instrument makes poetry but no music.

mod common;

use dk_agents::{BuildingKind, ItemKind, Sim};
use dk_core::TICKS_PER_DAY;

fn music_fort(seed: u64) -> (Sim, dk_raws::Raws) {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, seed);
    let mut sim = Sim::new(map, &raws, rng, 4);
    sim.invasions = false;
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    sim.add_embark_supplies(&raws);
    sim.place_flat_stockpiles(cx, cy, 24);
    let (ta, tb) = sim.find_flat_patch(cx, cy).expect("tavern");
    sim.add_tavern(ta, tb);
    (sim, raws)
}

#[test]
fn a_fort_with_an_instrument_makes_music() {
    let (mut sim, raws) = music_fort(9101);
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    let (ca, _) = sim.find_flat_patch(cx, cy).expect("carpenter site");
    assert!(sim.add_building(BuildingKind::Carpenter, ca));
    let sp = sim.dwarves[0].pos;
    for _ in 0..16 {
        sim.debug_spawn_log(sp);
    }

    for _ in 0..(TICKS_PER_DAY * 30) {
        sim.step(&raws);
        if !sim.songs.is_empty() {
            break;
        }
    }
    assert!(sim.stats.instruments_made > 0, "the carpenter should craft an instrument");
    assert!(!sim.songs.is_empty(), "with an instrument, the fort should make music");
}

#[test]
fn a_tavern_without_an_instrument_makes_no_music() {
    // Poetry needs only a tavern; music needs an instrument too.
    let (mut sim, raws) = music_fort(9102);
    for _ in 0..(TICKS_PER_DAY * 8) {
        sim.step(&raws);
    }
    assert!(!sim.poems.is_empty(), "a tavern alone still yields poetry");
    assert_eq!(sim.count_kind(ItemKind::Instrument), 0, "no instrument was made");
    assert!(sim.songs.is_empty(), "no instrument, no songs");
}

#[test]
fn the_fort_plays_the_same_songs_from_the_same_seed() {
    let run = |seed: u64| {
        let (mut sim, raws) = music_fort(seed);
        let cx = sim.map.width as i32 / 2;
        let cy = sim.map.height as i32 / 2;
        let (ca, _) = sim.find_flat_patch(cx, cy).unwrap();
        sim.add_building(BuildingKind::Carpenter, ca);
        let sp = sim.dwarves[0].pos;
        for _ in 0..16 {
            sim.debug_spawn_log(sp);
        }
        for _ in 0..(TICKS_PER_DAY * 30) {
            sim.step(&raws);
        }
        sim.songs.clone()
    };
    let a = run(9103);
    let b = run(9103);
    assert!(!a.is_empty(), "the fort composed at least one song");
    assert_eq!(a, b, "the same seed yields the same songs");
}
