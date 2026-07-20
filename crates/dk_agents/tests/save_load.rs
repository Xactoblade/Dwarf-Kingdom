//! Save/load round-trip integrity test: a richly-populated fortress — every
//! zone, a construction plan, an engraving, quality goods, culture, an alarm —
//! must survive a save and reload byte-for-byte, so no field is silently
//! dropped as the save format grows.

mod common;

use dk_agents::{load_sim, save_sim, BuildingKind, DesignationKind, ItemKind, Sim};
use dk_raws::{EconomyConfig, MaterialCategory, MaterialDef, MaterialRegistry, PlantDef, PlantRegistry, Raws};
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
    assert!(!sim.squads.is_empty(), "enlisting mustered a squad");
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
    same!(squads);
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

/// The material index of the given id in a registry.
fn mat_idx(raws: &Raws, id: &str) -> u16 {
    (0..raws.materials.len() as u16)
        .find(|&i| raws.materials.get(i).id == id)
        .expect("material present")
}

#[test]
fn a_reordered_registry_remaps_dwarf_favorites_and_tree_species() {
    // Regression (wood-species review): a dwarf's favorite_material/favorite_crop
    // and a tree's stored species are raws indices. If the material registry is
    // reordered between save and load (a same-version data edit), load_sim must
    // remap them by id — else a dwarf ends up fond of the wrong wood and a tree
    // changes species (or, on a shrunk registry, panics when named).
    let m = |id: &str, cat: MaterialCategory| MaterialDef {
        id: id.into(),
        name: id.into(),
        category: cat,
        color: [100, 100, 100],
        value: 1,
        combat: Default::default(),
        is_flux: false,
    };
    // Same six materials (covering the categories mapgen needs), two orders.
    let defs = |rev: bool| {
        let mut v = vec![
            m("loam", MaterialCategory::Soil),
            m("limestone", MaterialCategory::Sedimentary),
            m("granite", MaterialCategory::Igneous),
            m("hematite", MaterialCategory::Ore),
            m("oak", MaterialCategory::Wood),
            m("pine", MaterialCategory::Wood),
        ];
        if rev {
            v.reverse();
        }
        v
    };
    let plants = || {
        PlantRegistry::from_defs(vec![
            PlantDef { id: "barley".into(), name: "Barley".into(), color: [0, 0, 0], grow_days: 10, seasons: vec![0], brewable: true },
            PlantDef { id: "potato".into(), name: "Potato".into(), color: [0, 0, 0], grow_days: 8, seasons: vec![0], brewable: false },
        ])
        .unwrap()
    };
    let raws_a = Raws { materials: MaterialRegistry::from_defs(defs(false)).unwrap(), plants: plants(), tileset: None, economy: EconomyConfig::default() };
    let raws_b = Raws { materials: MaterialRegistry::from_defs(defs(true)).unwrap(), plants: plants(), tileset: None, economy: EconomyConfig::default() };

    let mut rng = dk_core::rng_from_seed(99);
    let map = dk_world::generate(&raws_a.materials, &mut rng, 24, 24, 12, 99);
    let mut sim = Sim::new(map, &raws_a, rng, 2);
    sim.invasions = false;

    // Pin dwarf 0 fond of oak, and plant an oak tree — both by their raws_a index.
    let oak_a = mat_idx(&raws_a, "oak");
    sim.dwarves[0].favorite_material = oak_a;
    let (tree, _) = {
        let cx = sim.map.width as i32 / 2;
        let cy = sim.map.height as i32 / 2;
        sim.find_flat_patch(cx, cy).expect("a flat tile")
    };
    sim.trees.insert(tree, oak_a);

    let path = std::env::temp_dir().join("dk_save_load").join("reorder.bin");
    save_sim(&sim, &path, &raws_a).expect("save");
    let loaded = load_sim(&path, &raws_b).expect("reload under the reordered registry");

    // The stored indices now point at oak *in raws_b*, not the stale number.
    let oak_b = mat_idx(&raws_b, "oak");
    assert_ne!(oak_a, oak_b, "the two registries really do order oak differently");
    assert_eq!(
        loaded.dwarves[0].favorite_material, oak_b,
        "the dwarf is still fond of oak after the registry was reordered"
    );
    assert_eq!(
        loaded.trees.get(&tree).copied(),
        Some(oak_b),
        "the tree is still oak after the registry was reordered"
    );
}
