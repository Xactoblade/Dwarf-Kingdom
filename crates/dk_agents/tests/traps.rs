//! Weapon-trap exit tests, run headlessly: a raider that blunders onto a
//! weapon trap is struck by hidden blades — and enough of them will kill.

mod common;

use dk_agents::{BuildingKind, Faction, Sim};
use dk_world::path::Pos;

fn trap_fort(seed: u64) -> (Sim, dk_raws::Raws) {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, seed);
    let mut sim = Sim::new(map, &raws, rng, 3);
    sim.invasions = false;
    (sim, raws)
}

#[test]
fn a_raider_is_wounded_crossing_a_trap() {
    let (mut sim, raws) = trap_fort(3401);
    let dpos = sim.dwarves[0].pos;
    // Scatter the other citizens so dwarf 0 is the raider's clear quarry.
    for j in 1..sim.dwarves.len() {
        sim.dwarves[j].pos = Pos::new(dpos.x, dpos.y + 8, dpos.z);
    }
    // A trap one step from the quarry, and a raider one step beyond it, so the
    // raider must tread on the trap to close the distance.
    let trap = Pos::new(dpos.x + 1, dpos.y, dpos.z);
    assert!(sim.add_building(BuildingKind::Trap, trap), "trap sits on floor");
    sim.spawn_raider_at(Pos::new(dpos.x + 2, dpos.y, dpos.z), &raws);
    let raider = sim
        .dwarves
        .iter()
        .position(|d| d.faction == Faction::Hostile)
        .unwrap();
    let full: i16 = sim.dwarves[raider].body.iter().map(|p| p.hp).sum();

    let mut sprung = false;
    for _ in 0..3000 {
        sim.step(&raws);
        let hurt = !sim.dwarves[raider].alive
            || sim.dwarves[raider].body.iter().map(|p| p.hp).sum::<i16>() < full;
        if hurt {
            sprung = true;
            break;
        }
    }
    assert!(sprung, "treading on the trap should wound the raider");
    assert!(
        sim.log.iter().any(|(_, m)| m.contains("weapon trap")),
        "the trap's bite is recorded"
    );
}

#[test]
fn traps_do_not_fire_without_a_trap() {
    // A sanity check that the trap hook is inert when no traps exist: a raider
    // approaching the fort takes no trap damage (it may still be fought, so we
    // only assert no trap log appears).
    let (mut sim, raws) = trap_fort(3402);
    let dpos = sim.dwarves[0].pos;
    sim.spawn_raider_at(Pos::new(dpos.x + 6, dpos.y, dpos.z), &raws);
    for _ in 0..400 {
        sim.step(&raws);
    }
    assert!(
        !sim.log.iter().any(|(_, m)| m.contains("weapon trap")),
        "no traps, no trap strikes"
    );
}
