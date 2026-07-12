//! Culture exit tests, run headlessly: a fort with a tavern grows a body of
//! poetry over time, and that culture is deterministic — the same fortress
//! always composes the same songs.

mod common;

use dk_agents::Sim;
use dk_core::TICKS_PER_DAY;

fn tavern_fort(seed: u64) -> (Sim, dk_raws::Raws) {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, seed);
    let mut sim = Sim::new(map, &raws, rng, 4);
    sim.invasions = false;
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    sim.place_flat_stockpiles(cx, cy, 18);
    let (ta, tb) = sim.find_flat_patch(cx, cy).expect("tavern site");
    sim.add_tavern(ta, tb);
    (sim, raws)
}

#[test]
fn a_fort_with_a_tavern_composes_poetry() {
    let (mut sim, raws) = tavern_fort(9901);
    assert!(sim.poems.is_empty(), "no poetry on day one");
    for _ in 0..(TICKS_PER_DAY * 6) {
        sim.step(&raws);
    }
    assert!(!sim.poems.is_empty(), "a week's taverning should yield some verse");
    // Each work is a titled piece.
    assert!(sim.poems.iter().all(|p| p.contains('"')), "works are titled");
    assert!(
        sim.log.iter().any(|(_, m)| m.contains("composes")),
        "the performance is recorded"
    );
}

#[test]
fn without_a_tavern_there_is_no_poetry() {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(9902);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, 9902);
    let mut sim = Sim::new(map, &raws, rng, 3);
    sim.invasions = false;
    for _ in 0..(TICKS_PER_DAY * 5) {
        sim.step(&raws);
    }
    assert!(sim.poems.is_empty(), "no tavern, no bards");
}

#[test]
fn a_fortress_always_sings_the_same_songs() {
    // Culture is deterministic in the seed.
    let (mut a, raws) = tavern_fort(9903);
    let (mut b, _) = tavern_fort(9903);
    for _ in 0..(TICKS_PER_DAY * 5) {
        a.step(&raws);
        b.step(&raws);
    }
    assert_eq!(a.poems, b.poems, "the same fort composes the same anthology");
    assert!(!a.poems.is_empty());
}
