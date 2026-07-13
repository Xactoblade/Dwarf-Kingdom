//! Woodcutting exit tests, run headlessly: a tree standing on the surface can
//! be designated for chopping, and a woodcutter walks to it, fells it, and
//! leaves a log where it stood. A map with no trees is unaffected — the whole
//! system is gated on trees existing (placed only at embark), so headless forts
//! stay byte-identical.

mod common;

use dk_agents::{DesignationKind, ItemKind, Sim};

fn wood_fort(seed: u64) -> (Sim, dk_raws::Raws) {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, seed);
    let mut sim = Sim::new(map, &raws, rng, 4);
    sim.invasions = false;
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    sim.add_embark_supplies(&raws);
    sim.place_flat_stockpiles(cx, cy, 18);
    (sim, raws)
}

#[test]
fn a_woodcutter_fells_a_tree_for_a_log() {
    let (mut sim, raws) = wood_fort(3301);
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;

    // A tree standing on a reachable flat tile, marked to be felled.
    let (tree, _) = sim.find_flat_patch(cx, cy).expect("a flat tile for a tree");
    sim.trees.insert(tree);
    assert!(sim.tree_at(tree));
    assert_eq!(sim.designate_rect(DesignationKind::Chop, tree, tree), 1);
    assert_eq!(sim.count_kind(ItemKind::Log), 0, "nothing felled yet");

    let mut felled = false;
    for _ in 0..12_000 {
        sim.step(&raws);
        if sim.stats.trees_felled > 0 {
            felled = true;
            break;
        }
    }
    assert!(felled, "a woodcutter should fell the marked tree");
    assert!(!sim.tree_at(tree), "the felled tree is gone");
    assert!(sim.count_kind(ItemKind::Log) > 0, "a log lies where the tree stood");
}

#[test]
fn a_treeless_fort_grows_no_logs() {
    // With no trees planted, there is nothing to chop and no logs ever appear.
    let (mut sim, raws) = wood_fort(3302);
    assert!(sim.trees.is_empty());
    for _ in 0..3_000 {
        sim.step(&raws);
    }
    assert_eq!(sim.stats.trees_felled, 0);
    assert_eq!(sim.count_kind(ItemKind::Log), 0);
}

#[test]
fn planting_scatters_trees_on_open_ground() {
    // plant_trees drops trees only on walkable surface tiles, never in water or
    // on a building, and is deterministic for a given seed.
    let (mut sim, _raws) = wood_fort(3303);
    sim.plant_trees(40);
    assert!(!sim.trees.is_empty(), "some trees should take root");
    for &t in &sim.trees {
        assert!(sim.map.walkable(t), "a tree stands on walkable ground");
        assert_eq!(sim.map.water_at(t), 0, "no tree grows in open water");
    }
}
