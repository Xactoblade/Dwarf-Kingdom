//! Carpenter exit tests, run headlessly: a carpenter's workshop works logs
//! into barrels, and the whole wood chain runs end to end — a standing tree is
//! felled into a log, and the log is worked into a barrel. A fort with no
//! carpenter (or no logs) makes no barrels.

mod common;

use dk_agents::{BuildingKind, DesignationKind, ItemKind, Sim};

fn carpentry_fort(seed: u64) -> (Sim, dk_raws::Raws) {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, seed);
    let mut sim = Sim::new(map, &raws, rng, 4);
    sim.invasions = false;
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    sim.add_embark_supplies(&raws);
    sim.place_flat_stockpiles(cx, cy, 24);
    (sim, raws)
}

#[test]
fn a_carpenter_works_a_log_into_a_barrel() {
    let (mut sim, raws) = carpentry_fort(4401);
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;

    let (ca, _) = sim.find_flat_patch(cx, cy).expect("carpenter site");
    assert!(sim.add_building(BuildingKind::Carpenter, ca));
    let sp = sim.dwarves[0].pos;
    for _ in 0..6 {
        sim.debug_spawn_log(sp);
    }
    assert_eq!(sim.count_kind(ItemKind::Barrel), 0, "nothing made yet");

    let mut made = false;
    for _ in 0..12_000 {
        sim.step(&raws);
        if sim.stats.barrels_made > 0 {
            made = true;
            break;
        }
    }
    assert!(made, "the carpenter should work a log into a barrel");
    assert!(sim.count_kind(ItemKind::Barrel) > 0, "a barrel exists in the fort");
}

#[test]
fn the_whole_wood_chain_yields_a_barrel() {
    // Tree -> chop -> log -> carpenter -> barrel, with no debug shortcuts.
    let (mut sim, raws) = carpentry_fort(4402);
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;

    let (ca, _) = sim.find_flat_patch(cx, cy).expect("carpenter site");
    assert!(sim.add_building(BuildingKind::Carpenter, ca));
    // A small stand of trees to fell, each on reachable flat ground.
    for _ in 0..4 {
        if let Some((t, _)) = sim.find_flat_patch(cx, cy) {
            if !sim.tree_at(t) {
                sim.trees.insert(t);
                sim.designate_rect(DesignationKind::Chop, t, t);
            }
        }
    }

    let mut made = false;
    for _ in 0..20_000 {
        sim.step(&raws);
        if sim.stats.barrels_made > 0 {
            made = true;
            break;
        }
    }
    assert!(sim.stats.trees_felled > 0, "a tree should have been felled for a log");
    assert!(made, "the felled log should be worked into a barrel");
}

#[test]
fn no_carpenter_means_no_barrels() {
    let (mut sim, raws) = carpentry_fort(4403);
    let sp = sim.dwarves[0].pos;
    for _ in 0..6 {
        sim.debug_spawn_log(sp);
    }
    for _ in 0..3_000 {
        sim.step(&raws);
    }
    assert_eq!(sim.stats.barrels_made, 0, "no carpenter, no barrels");
    assert_eq!(sim.count_kind(ItemKind::Barrel), 0);
}
