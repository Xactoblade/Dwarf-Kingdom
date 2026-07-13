//! Werebeast tests, run headlessly: a cursed dwarf twists into a hostile beast
//! under the full moon and reverts at dawn after; the curse spreads by the
//! beast's bite. A fort with no curse is untouched — the whole mechanic is
//! gated on a cursed dwarf existing, so it draws no rng and changes nothing
//! without one.

mod common;

use dk_agents::{Faction, Sim};
use dk_core::TICKS_PER_DAY;
use dk_world::path::Pos;

fn were_fort(seed: u64, n: usize) -> (Sim, dk_raws::Raws) {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, seed);
    let mut sim = Sim::new(map, &raws, rng, n);
    sim.invasions = false;
    (sim, raws)
}

#[test]
fn a_werebeast_transforms_under_the_full_moon_and_reverts() {
    // The clock starts on day 0 — a full moon — so a cursed dwarf transforms
    // almost at once, then reverts once the moon passes (day >= 2).
    let (mut sim, raws) = were_fort(6601, 1);
    sim.curse_a_werebeast();
    assert!(sim.dwarves[0].werebeast, "the founder carries the curse");
    assert!(!sim.dwarves[0].were_form, "not yet transformed");

    sim.step(&raws);
    assert!(sim.dwarves[0].were_form, "the full moon should wake the beast");
    assert_eq!(sim.dwarves[0].faction, Faction::Hostile, "the beast turns on the fort");

    for _ in 0..(TICKS_PER_DAY * 3) {
        sim.step(&raws);
    }
    assert!(!sim.dwarves[0].were_form, "dawn after the moon returns the dwarf");
    assert_eq!(sim.dwarves[0].faction, Faction::Fort, "and it rejoins the fort");
    assert!(sim.dwarves[0].werebeast, "but the curse endures");
}

#[test]
fn a_fort_with_no_curse_never_transforms() {
    let (mut sim, raws) = were_fort(6602, 4);
    for _ in 0..(TICKS_PER_DAY * 3) {
        sim.step(&raws);
    }
    assert!(!sim.dwarves.iter().any(|d| d.were_form), "no curse, no beasts");
    assert!(!sim.dwarves.iter().any(|d| d.werebeast));
}

#[test]
fn cursing_marks_exactly_one_werebeast() {
    let (mut sim, _raws) = were_fort(6603, 5);
    sim.curse_a_werebeast();
    assert_eq!(sim.dwarves.iter().filter(|d| d.werebeast).count(), 1);
}

#[test]
fn the_curse_spreads_by_the_beasts_bite() {
    // A transformed beast beside a fort-mate: kept both pinned and healthy so
    // the beast keeps biting until the curse takes (1 in 4 per bite).
    let (mut sim, raws) = were_fort(6604, 2);
    sim.curse_a_werebeast(); // dwarves[0]
    let z = sim.dwarves[0].pos.z;
    sim.dwarves[0].pos = Pos::new(10, 10, z);
    sim.dwarves[1].pos = Pos::new(11, 10, z);

    let mut spread = false;
    for _ in 0..6_000 {
        sim.step(&raws);
        // Keep them adjacent and alive so the mauling continues.
        sim.dwarves[0].pos = Pos::new(10, 10, z);
        sim.dwarves[1].pos = Pos::new(11, 10, z);
        for k in 0..2 {
            for p in &mut sim.dwarves[k].body {
                p.hp = p.max_hp;
                p.bleeding = 0;
            }
            sim.dwarves[k].blood = 100.0;
        }
        if sim.dwarves[1].werebeast {
            spread = true;
            break;
        }
    }
    assert!(spread, "the beast's bite should eventually spread the curse");
}
