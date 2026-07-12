//! The vampire: a secret night-creature hidden among the fort. It looks and
//! works like any other dwarf, but drains the blood of sleeping fort-mates,
//! never eats or drinks, and cannot be starved. Crucially, a fort that harbours
//! NO vampire must behave exactly as before — the whole mechanic is gated on
//! the secret it keeps, so it draws no rng and touches no state without one.

mod common;

use dk_agents::{Sim, Task, NEED_DEATH_TICKS, VAMPIRE_FEED_INTERVAL};
use dk_world::path::Pos;

/// A fresh fort with `n` dwarves, invasions off, on a small flat-ish map.
fn fort(seed: u64) -> (Sim, dk_raws::Raws) {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, seed);
    let mut sim = Sim::new(map, &raws, rng, 5);
    sim.invasions = false;
    (sim, raws)
}

/// Lay two dwarves down asleep, side by side, on solid ground. Returns their
/// indices (a, b) and the shared z.
fn bed_down_pair(sim: &mut Sim) -> (usize, usize) {
    let (x, y) = (12, 12);
    let z = sim.map.walk_surface_z(x as usize, y as usize).unwrap() as i32;
    sim.dwarves[0].pos = Pos::new(x, y, z);
    sim.dwarves[0].task = Task::Sleep { remaining: 60000 };
    sim.dwarves[1].pos = Pos::new(x + 1, y, z);
    sim.dwarves[1].task = Task::Sleep { remaining: 60000 };
    (0, 1)
}

#[test]
fn a_vampire_drains_a_sleeping_fortmate_and_the_curse_comes_to_light() {
    let (mut sim, raws) = fort(7001);
    let (vamp, victim) = bed_down_pair(&mut sim);
    sim.dwarves[vamp].vampire = true;
    sim.dwarves[vamp].last_fed = 0;
    // Start the victim already weak so a couple of feedings finish them before
    // any other need could — this is a test of the drain, not of thirst.
    sim.dwarves[victim].blood = 40.0;

    // One feeding interval should visibly bleed the victim.
    for _ in 0..(VAMPIRE_FEED_INTERVAL + 50) {
        sim.step(&raws);
    }
    assert!(
        sim.dwarves[victim].blood < 20.0,
        "the vampire should have fed at least once (blood {})",
        sim.dwarves[victim].blood
    );

    // Given more nights, the victim is drained white and the fort learns why.
    let mut steps = 0;
    while sim.stats.drained == 0 && steps < 6 * VAMPIRE_FEED_INTERVAL {
        sim.step(&raws);
        steps += 1;
    }
    assert_eq!(sim.stats.drained, 1, "a fort-mate should have been drained dead");
    assert!(!sim.dwarves[victim].alive, "the drained victim is dead");
    assert!(
        sim.log.iter().any(|(_, m)| m.contains("vampire walks among us")),
        "the discovery should be announced in the log"
    );
}

#[test]
fn a_vampire_neither_eats_nor_drinks_nor_starves() {
    let (mut sim, raws) = fort(7002);
    // No victim adjacent: the vampire simply goes about unfed.
    let z = sim.map.walk_surface_z(12, 12).unwrap() as i32;
    sim.dwarves[0].pos = Pos::new(12, 12, z);
    sim.dwarves[0].vampire = true;
    // Its needs are frozen wherever they started — they must not climb an inch.
    let (h0, t0) = (sim.dwarves[0].hunger, sim.dwarves[0].thirst);

    // Run well past the span in which any ordinary dwarf would have starved.
    for _ in 0..(2 * NEED_DEATH_TICKS) {
        sim.step(&raws);
    }
    let v = &sim.dwarves[0];
    assert!(v.alive, "a vampire cannot die of hunger or thirst");
    assert_eq!(v.hunger, h0, "a vampire never hungers (it feeds on blood)");
    assert_eq!(v.thirst, t0, "a vampire never thirsts");
}

#[test]
fn a_fort_with_no_vampire_is_never_touched() {
    let (mut sim, raws) = fort(7003);
    let (_a, victim) = bed_down_pair(&mut sim); // adjacent + asleep, but no curse
    let blood_before = sim.dwarves[victim].blood;

    for _ in 0..(3 * VAMPIRE_FEED_INTERVAL) {
        sim.step(&raws);
    }
    assert_eq!(sim.stats.drained, 0, "no vampire, no drainings");
    // Blood can only have recovered a hair (natural regen), never been drained.
    assert!(
        sim.dwarves[victim].blood >= blood_before,
        "an un-cursed sleeper loses no blood (before {blood_before}, after {})",
        sim.dwarves[victim].blood
    );
}

#[test]
fn cursing_plants_exactly_one_vampire() {
    let (mut sim, _raws) = fort(7004);
    sim.curse_a_vampire();
    let count = sim.dwarves.iter().filter(|d| d.vampire).count();
    assert_eq!(count, 1, "exactly one of the founders carries the curse");
}
