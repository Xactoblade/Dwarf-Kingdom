//! Well exit tests, run headlessly: when the fort has run dry of brewed drink,
//! a thirsty dwarf draws clean water from a well and lives. Without a well, the
//! same parched dwarf has nothing to drink. A well is a pure fallback — it only
//! fires when no drink is at hand — so a fort without one is unaffected.

mod common;

use dk_agents::{BuildingKind, ItemKind, Sim};

/// A fort with NO brewed drink at all (never adds embark supplies).
fn dry_fort(seed: u64) -> (Sim, dk_raws::Raws) {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, seed);
    let mut sim = Sim::new(map, &raws, rng, 4);
    sim.invasions = false;
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    sim.place_flat_stockpiles(cx, cy, 18);
    (sim, raws)
}

#[test]
fn a_well_slakes_a_parched_dwarf() {
    let (mut sim, raws) = dry_fort(3701);
    assert_eq!(sim.count_kind(ItemKind::Drink), 0, "the fort has no brewed drink");
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    let (wa, _) = sim.find_flat_patch(cx, cy).expect("well site");
    assert!(sim.add_building(BuildingKind::Well, wa));

    // Parch the dwarf well past the "seek a drink" threshold.
    sim.dwarves[0].thirst = 90.0;
    let mut slaked = false;
    for _ in 0..3_000 {
        sim.step(&raws);
        if sim.dwarves[0].thirst < 20.0 {
            slaked = true;
            break;
        }
    }
    assert!(slaked, "the dwarf should draw water from the well (thirst {})", sim.dwarves[0].thirst);
}

#[test]
fn no_well_and_no_drink_leaves_the_dwarf_parched() {
    // The same dry fort, but with no well: nothing quenches the thirst, so it
    // only ever climbs. (Confirms the well is what does the slaking, and that a
    // well-less fort's thirst path is unchanged.)
    let (mut sim, raws) = dry_fort(3702);
    sim.dwarves[0].thirst = 90.0;
    let before = sim.dwarves[0].thirst;
    for _ in 0..1_500 {
        sim.step(&raws);
        if !sim.dwarves[0].alive {
            break;
        }
    }
    assert!(
        sim.dwarves[0].thirst >= before || !sim.dwarves[0].alive,
        "with no well and no drink, thirst is never slaked"
    );
}
