//! Pressure plates (Mechanisms slice S3): the first trigger that needs no
//! dwarf's hand. Anything that walks onto a plate fires its linked device
//! through the generalized trigger->device dispatch (S1). Edge-triggered:
//! walking on fires once; standing there does nothing.

mod common;

use dk_agents::Sim;
use dk_world::path::Pos;
use dk_world::{Map, Tile};

/// Flat arena: solid bedrock at z0, walkable floor at z1.
fn flat_map(w: usize, h: usize) -> Map {
    let mut m = Map::new_air(w, h, 4, 0);
    for y in 0..h {
        for x in 0..w {
            m.set(x, y, 0, Tile::solid(2)); // granite
            m.set(x, y, 1, Tile::floor(2));
        }
    }
    m
}

fn arena(seed: u64, dwarves: usize) -> Sim {
    let raws = common::test_raws();
    let mut sim = Sim::new(flat_map(20, 20), &raws, dk_core::rng_from_seed(seed), dwarves);
    sim.water.springs.clear();
    sim.rebuild_caches();
    sim
}

/// Put dwarf `i` on `p` as if it had just walked there (pos moved, last_pos
/// still behind) — exactly the state the movement code leaves behind.
fn step_onto(sim: &mut Sim, i: usize, p: Pos) {
    sim.dwarves[i].last_pos = sim.dwarves[i].pos;
    sim.dwarves[i].pos = p;
}

#[test]
fn stepping_onto_a_plate_raises_a_drawbridge() {
    let mut sim = arena(1, 1);

    let span: Vec<Pos> = (8..=10).map(|x| Pos::new(x, 5, 1)).collect();
    let anchor = sim.add_bridge(span.clone()).expect("the bridge is placed");
    let plate = Pos::new(4, 5, 1);
    assert_eq!(sim.add_pressure_plate(plate), Some(anchor), "the plate wires to the bridge");
    assert!(!sim.bridges[&anchor].raised);

    // A dwarf walks onto the plate: the bridge goes up.
    step_onto(&mut sim, 0, plate);
    sim.tick_pressure_plates();
    assert!(sim.bridges[&anchor].raised, "walking onto the plate raised the bridge");
    for &p in &span {
        assert!(!sim.map.walkable(p), "a raised span blocks the way");
    }

    // Standing on it does NOT re-fire — it is an edge trigger, not a level one.
    sim.dwarves[0].last_pos = plate;
    for _ in 0..5 {
        sim.tick_pressure_plates();
    }
    assert!(sim.bridges[&anchor].raised, "standing on a plate holds it, it does not re-fire");

    // Walking off and back on fires it again, lowering the bridge.
    step_onto(&mut sim, 0, Pos::new(3, 5, 1));
    sim.tick_pressure_plates();
    assert!(sim.bridges[&anchor].raised, "stepping off is not a trigger");
    step_onto(&mut sim, 0, plate);
    sim.tick_pressure_plates();
    assert!(!sim.bridges[&anchor].raised, "stepping back on lowered it");
}

#[test]
fn a_plate_fires_once_however_many_feet_land_on_it() {
    let mut sim = arena(2, 3);

    let anchor = sim.add_bridge(vec![Pos::new(9, 5, 1)]).expect("the bridge is placed");
    let plate = Pos::new(4, 5, 1);
    assert_eq!(sim.add_pressure_plate(plate), Some(anchor));

    // Three bodies pile onto the same tile in one tick. A per-actor trigger
    // would toggle three times (leaving it raised by luck of the count); the
    // plate must fire exactly once.
    for i in 0..3 {
        step_onto(&mut sim, i, plate);
    }
    sim.tick_pressure_plates();
    assert!(sim.bridges[&anchor].raised, "one crowd, one toggle");
}

#[test]
fn a_plate_toggles_a_floodgate_too() {
    let mut sim = arena(3, 1);

    let gate = Pos::new(9, 9, 1);
    assert!(sim.add_building(dk_agents::BuildingKind::Floodgate, gate));
    assert!(!sim.map.walkable(gate), "a floodgate starts closed");

    let plate = Pos::new(4, 9, 1);
    assert_eq!(sim.add_pressure_plate(plate), Some(gate), "the plate wires to the gate");

    step_onto(&mut sim, 0, plate);
    sim.tick_pressure_plates();
    assert!(sim.map.walkable(gate), "the plate opened the floodgate");
}

#[test]
fn a_plate_needs_open_floor_and_something_to_wire_to() {
    let mut sim = arena(4, 1);

    // Nothing linkable in the fort yet.
    assert!(sim.add_pressure_plate(Pos::new(4, 4, 1)).is_none(), "nothing to wire to");

    let span: Vec<Pos> = (8..=10).map(|x| Pos::new(x, 5, 1)).collect();
    let anchor = sim.add_bridge(span.clone()).expect("the bridge is placed");

    // Not on the bridge span itself — a raise would swallow the plate.
    assert!(sim.add_pressure_plate(span[1]).is_none(), "not on a drawbridge span");
    // Not into a wall.
    sim.map.set_at(Pos::new(2, 2, 1), Tile::solid(2));
    assert!(sim.add_pressure_plate(Pos::new(2, 2, 1)).is_none(), "not inside rock");
    // Not on top of another building.
    let taken = Pos::new(3, 3, 1);
    assert_eq!(sim.add_pressure_plate(taken), Some(anchor));
    assert!(sim.add_pressure_plate(taken).is_none(), "not stacked on another building");
}

#[test]
fn a_fort_with_no_plates_is_untouched_by_the_plate_tick() {
    let mut sim = arena(5, 2);
    let anchor = sim.add_bridge(vec![Pos::new(9, 5, 1)]).expect("the bridge is placed");
    // Dwarves walking about a plate-less fort toggle nothing.
    step_onto(&mut sim, 0, Pos::new(4, 5, 1));
    step_onto(&mut sim, 1, Pos::new(9, 5, 1));
    sim.tick_pressure_plates();
    assert!(!sim.bridges[&anchor].raised);
}

#[test]
fn a_plate_fires_inside_the_real_tick() {
    // The direct-call tests above prove the mechanism; this one proves the
    // wiring — that step() actually runs the plate pass, and runs it before
    // tick_footprints stamps last_pos out from under the edge trigger.
    let raws = common::test_raws();
    let mut sim = arena(6, 1);

    let anchor = sim.add_bridge(vec![Pos::new(9, 5, 1)]).expect("the bridge is placed");
    let plate = Pos::new(4, 5, 1);
    assert_eq!(sim.add_pressure_plate(plate), Some(anchor));

    step_onto(&mut sim, 0, plate);
    sim.step(&raws);
    assert!(sim.bridges[&anchor].raised, "a full tick fires the plate");
}
