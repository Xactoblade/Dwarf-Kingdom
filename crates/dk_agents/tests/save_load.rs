//! Save/load round-trip integrity test: a richly-populated fortress — every
//! zone, a construction plan, an engraving, quality goods, culture, an alarm —
//! must survive a save and reload byte-for-byte, so no field is silently
//! dropped as the save format grows.

mod common;

use dk_agents::{load_sim, save_sim, BuildingKind, DesignationKind, ItemKind, Sim};
use dk_world::path::Pos;

/// Build a fort exercising as much serialized state as we can reach.
fn rich_fort(seed: u64) -> (Sim, dk_raws::Raws) {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, seed);
    let mut sim = Sim::new(map, &raws, rng, 5);
    sim.invasions = false;
    sim.add_embark_supplies(&raws);
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    sim.place_flat_stockpiles(cx, cy, 24);

    // One of every zone.
    for add in [
        Sim::add_tavern as fn(&mut Sim, Pos, Pos),
        Sim::add_temple,
        Sim::add_hospital,
        Sim::add_barracks,
        Sim::add_burrow,
        Sim::add_library,
    ] {
        if let Some((a, b)) = sim.find_flat_patch(cx, cy) {
            add(&mut sim, a, b);
        }
    }
    // A workshop, a construction plan, a designation, a soldier, the alarm.
    if let Some((wa, _)) = sim.find_flat_patch(cx, cy) {
        sim.add_building(BuildingKind::Craftsdwarf, wa);
    }
    let wz = sim.map.walk_surface_z(cx as usize, cy as usize).unwrap() as i32;
    sim.designate_rect(DesignationKind::Mine, Pos::new(cx, cy, wz - 4), Pos::new(cx, cy, wz - 4));
    let floor = Pos::new(cx + 1, cy, wz);
    if sim.map.walkable(floor) {
        sim.designate_construction(floor);
    }
    let sp = sim.dwarves[0].pos;
    sim.toggle_soldier(sp);
    sim.toggle_alarm();

    // Run a while so items, quality goods, culture, and engravings accrue.
    for _ in 0..(dk_core::TICKS_PER_DAY * 8) {
        sim.step(&raws);
    }
    (sim, raws)
}

#[test]
fn a_rich_fortress_survives_a_save_and_reload() {
    let (sim, raws) = rich_fort(4242);

    // Sanity: the fort really is rich in serialized state.
    assert!(!sim.hospitals.is_empty() && !sim.barracks.is_empty() && !sim.burrows.is_empty());
    assert!(!sim.library.is_empty() && !sim.taverns.is_empty() && !sim.temples.is_empty());
    assert!(sim.alarm, "the alarm was sounded");
    assert!(!sim.buildings.is_empty());
    assert!(sim.count_kind(ItemKind::Meal) > 0 || sim.count_kind(ItemKind::Drink) > 0);

    let path = std::env::temp_dir().join("dk_save_load").join("rich.bin");
    save_sim(&sim, &path, &raws).expect("save the fortress");
    let loaded = load_sim(&path, &raws).expect("reload the fortress");

    // Every serialized collection round-trips identically. Comparing the
    // whole Sim at once would trip over the material-manifest remap; compare
    // each collection's bytes instead.
    macro_rules! same {
        ($field:ident) => {
            assert_eq!(
                bincode::serialize(&sim.$field).unwrap(),
                bincode::serialize(&loaded.$field).unwrap(),
                concat!(stringify!($field), " differ across save/load")
            );
        };
    }
    same!(dwarves);
    same!(items);
    same!(buildings);
    same!(stockpiles);
    same!(hospitals);
    same!(barracks);
    same!(burrows);
    same!(library);
    same!(taverns);
    same!(temples);
    same!(designations);
    same!(constructions);
    same!(engravings);
    same!(poems);
    same!(treatises);
    assert_eq!(sim.alarm, loaded.alarm, "alarm state differs");
    assert_eq!(sim.clock.tick, loaded.clock.tick, "the clock differs");

    // And the reloaded fort keeps running without a hiccup.
    let mut loaded = loaded;
    for _ in 0..500 {
        loaded.step(&raws);
    }
    assert!(loaded.dwarves.iter().any(|d| d.alive));
}
