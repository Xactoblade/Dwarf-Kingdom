//! Retired-fortress exit tests, run headlessly: a fortress can be retired to
//! its own file, keyed to its region, and later reclaimed exactly as it was —
//! its people, its stores, and the memory of where it stands all intact.

mod common;

use dk_agents::{load_sim, save_sim, Sim};

fn founded_fort(seed: u64, region: (usize, usize)) -> (Sim, dk_raws::Raws) {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, seed);
    let mut sim = Sim::new(map, &raws, rng, 4);
    sim.invasions = false;
    sim.home_region = Some(region);
    (sim, raws)
}

#[test]
fn a_retired_fortress_is_reclaimed_as_it_was() {
    let region = (3, 5);
    let (mut sim, raws) = founded_fort(4401, region);

    // Let the colony live a little so it has a history worth preserving.
    for _ in 0..200 {
        sim.step(&raws);
    }
    let population = sim.dwarves.iter().filter(|d| d.alive).count();
    let a_name = sim.dwarves[0].name.clone();
    let tick = sim.clock.tick;

    // Retire it to its own region-keyed file, then reclaim it.
    let path = std::env::temp_dir()
        .join("dk_retire_test")
        .join(format!("fort_{}_{}.bin", region.0, region.1));
    save_sim(&sim, &path, &raws).expect("the fortress retires to history");
    let reclaimed = load_sim(&path, &raws).expect("the fortress is reclaimed");

    assert_eq!(
        reclaimed.home_region,
        Some(region),
        "a reclaimed fort remembers where it stands"
    );
    assert_eq!(
        reclaimed.dwarves.iter().filter(|d| d.alive).count(),
        population,
        "its people are all still here"
    );
    assert_eq!(reclaimed.dwarves[0].name, a_name, "and they are the same folk");
    assert_eq!(reclaimed.clock.tick, tick, "time picks up where it left off");
}

#[test]
fn a_reclaimed_fortress_lives_on() {
    let region = (1, 1);
    let (mut sim, raws) = founded_fort(4402, region);
    for _ in 0..50 {
        sim.step(&raws);
    }
    let path = std::env::temp_dir()
        .join("dk_retire_test")
        .join("fort_1_1.bin");
    save_sim(&sim, &path, &raws).unwrap();
    let mut reclaimed = load_sim(&path, &raws).unwrap();

    // The reclaimed colony keeps simulating without missing a beat.
    let t0 = reclaimed.clock.tick;
    for _ in 0..50 {
        reclaimed.step(&raws);
    }
    assert!(reclaimed.clock.tick > t0, "the reclaimed fort marches on");
    assert!(
        reclaimed.dwarves.iter().any(|d| d.alive),
        "and its people still live"
    );
}
