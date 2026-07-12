//! Library / scholarship exit tests, run headlessly: a fort with a library
//! sets down treatises — its accumulated knowledge — deterministically, and a
//! fort without one writes none.

mod common;

use dk_agents::Sim;
use dk_core::TICKS_PER_DAY;

fn scholarly_fort(seed: u64, with_library: bool) -> (Sim, dk_raws::Raws) {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, seed);
    let mut sim = Sim::new(map, &raws, rng, 4);
    sim.invasions = false;
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    sim.place_flat_stockpiles(cx, cy, 18);
    if with_library {
        let (a, b) = sim.find_flat_patch(cx, cy).expect("library site");
        sim.add_library(a, b);
    }
    (sim, raws)
}

#[test]
fn a_fort_with_a_library_writes_treatises() {
    let (mut sim, raws) = scholarly_fort(1001, true);
    assert!(sim.treatises.is_empty());
    for _ in 0..(TICKS_PER_DAY * 6) {
        sim.step(&raws);
    }
    assert!(!sim.treatises.is_empty(), "a week of scholarship yields treatises");
    assert!(
        sim.treatises.iter().all(|t| t.contains("on") || t.contains("of") || t.contains("into")),
        "each is a titled scholarly work"
    );
    assert!(
        sim.log.iter().any(|(_, m)| m.contains("sets down") && m.contains("library")),
        "the work is recorded"
    );
}

#[test]
fn without_a_library_no_treatises() {
    let (mut sim, raws) = scholarly_fort(1002, false);
    for _ in 0..(TICKS_PER_DAY * 5) {
        sim.step(&raws);
    }
    assert!(sim.treatises.is_empty(), "no library, no scholarship");
}

#[test]
fn scholarship_is_deterministic() {
    let (mut a, raws) = scholarly_fort(1003, true);
    let (mut b, _) = scholarly_fort(1003, true);
    for _ in 0..(TICKS_PER_DAY * 5) {
        a.step(&raws);
        b.step(&raws);
    }
    assert_eq!(a.treatises, b.treatises, "the same fort writes the same books");
    assert!(!a.treatises.is_empty());
}
