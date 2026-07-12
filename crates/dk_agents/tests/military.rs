//! Military exit tests, run headlessly: enlisted soldiers proactively hunt
//! hostiles, where ordinary citizens only defend themselves when cornered.

mod common;

use dk_agents::{Faction, Sim, Task};
use dk_world::path::Pos;
use dk_world::{Map, Tile};

fn arena(dwarves: usize) -> (Sim, dk_raws::Raws) {
    let raws = common::test_raws();
    let mut m = Map::new_air(30, 30, 4, 0);
    for y in 0..30 {
        for x in 0..30 {
            m.set(x, y, 0, Tile::solid(2));
            m.set(x, y, 1, Tile::floor(2));
        }
    }
    let rng = dk_core::rng_from_seed(1701);
    let mut sim = Sim::new(m, &raws, rng, dwarves);
    sim.water.springs.clear();
    sim.invasions = false;
    sim.rebuild_caches();
    (sim, raws)
}

#[test]
fn a_soldier_marches_on_a_distant_raider() {
    let (mut sim, raws) = arena(2);
    // Put the citizens in one corner, a raider far away in another.
    sim.dwarves[0].pos = Pos::new(2, 2, 1);
    sim.dwarves[1].pos = Pos::new(3, 2, 1);
    sim.spawn_raider_at(Pos::new(26, 26, 1), &raws);
    let raider = sim.dwarves.len() - 1;

    // Enlist dwarf 0; dwarf 1 stays a civilian.
    assert_eq!(sim.toggle_soldier(Pos::new(2, 2, 1)), Some(true));
    let start_dist = sim.dwarves[0].pos.manhattan(sim.dwarves[raider].pos);

    for _ in 0..2_000 {
        sim.step(&raws);
        if !sim.dwarves[raider].alive {
            break;
        }
    }
    // The soldier closed on (and likely felled) the raider.
    let engaged = !sim.dwarves[raider].alive
        || sim.dwarves[0].pos.manhattan(sim.dwarves[raider].pos) < start_dist / 2;
    assert!(engaged, "an enlisted soldier should hunt the raider down");
    assert!(
        sim.log.iter().any(|(_, m)| m.contains("takes up arms")),
        "enlistment is recorded"
    );
    assert_eq!(sim.soldier_count(), if sim.dwarves[0].alive { 1 } else { 0 });
}

#[test]
fn civilians_do_not_go_looking_for_a_fight() {
    let (mut sim, raws) = arena(2);
    sim.dwarves[0].pos = Pos::new(2, 2, 1);
    sim.dwarves[1].pos = Pos::new(3, 2, 1);
    // A raider penned far away and unable to reach the citizens (walled off).
    for y in 0..30 {
        sim.map.set(15, y, 1, Tile::solid(2));
    }
    sim.rebuild_caches();
    sim.spawn_raider_at(Pos::new(26, 26, 1), &raws);

    // Nobody is enlisted; the citizens keep to their side.
    for _ in 0..2_000 {
        sim.step(&raws);
        for c in [0usize, 1] {
            assert!(
                sim.dwarves[c].pos.x < 15,
                "an un-enlisted citizen must not march across the map to fight"
            );
            assert!(!matches!(sim.dwarves[c].task, Task::Fight { .. }));
        }
    }
}

#[test]
fn enlist_and_dismiss_toggles() {
    let (mut sim, _raws) = arena(1);
    let p = sim.dwarves[0].pos;
    assert_eq!(sim.toggle_soldier(p), Some(true));
    assert!(sim.dwarves[0].soldier);
    assert_eq!(sim.toggle_soldier(p), Some(false));
    assert!(!sim.dwarves[0].soldier);
    // Nothing to enlist on empty ground.
    assert_eq!(sim.toggle_soldier(Pos::new(20, 20, 1)), None);
    assert_ne!(Faction::Fort, Faction::Hostile); // (silence unused import)
}
