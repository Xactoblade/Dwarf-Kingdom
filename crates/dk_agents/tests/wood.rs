//! Woodcutting exit tests, run headlessly: a tree standing on the surface can
//! be designated for chopping, and a woodcutter walks to it, fells it, and
//! leaves a log where it stood. A map with no trees is unaffected — the whole
//! system is gated on trees existing (placed only at embark), so headless forts
//! stay byte-identical.

mod common;

use dk_agents::{DesignationKind, ItemKind, Sim};
use dk_raws::MaterialCategory;

/// The first wood species in the test raws — a valid `stuff` for a tree/log.
fn oak(raws: &dk_raws::Raws) -> u16 {
    raws.materials.indices_in_category(MaterialCategory::Wood)[0]
}

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
    sim.trees.insert(tree, oak(&raws));
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
fn cancelling_a_chop_spares_the_tree() {
    // Cancelling a chop designation must actually stop the woodcutter — the
    // tree should still be standing and no log produced (regression: cancel_rect
    // handled Mine/Build but not Chop, so the cutter felled it anyway).
    let (mut sim, raws) = wood_fort(3304);
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    let (tree, _) = sim.find_flat_patch(cx, cy).expect("a flat tile for a tree");
    sim.trees.insert(tree, oak(&raws));
    sim.designate_rect(DesignationKind::Chop, tree, tree);

    // Let a cutter get assigned and start walking, then cancel the designation.
    for _ in 0..40 {
        sim.step(&raws);
    }
    sim.cancel_rect(tree, tree);

    // Give it plenty of time to (wrongly) finish felling if the cancel failed.
    for _ in 0..4_000 {
        sim.step(&raws);
    }
    assert!(sim.tree_at(tree), "the cancelled tree should still stand");
    assert_eq!(sim.stats.trees_felled, 0, "nothing should have been felled");
    assert_eq!(sim.count_kind(ItemKind::Log), 0, "no log from a cancelled chop");
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
    let (mut sim, raws) = wood_fort(3303);
    sim.plant_trees(40, &raws);
    assert!(!sim.trees.is_empty(), "some trees should take root");
    for &t in sim.trees.keys() {
        assert!(sim.map.walkable(t), "a tree stands on walkable ground");
        assert_eq!(sim.map.water_at(t), 0, "no tree grows in open water");
    }
}

#[test]
fn a_planted_forest_regrows_over_time() {
    // plant_trees sets tree_cap above the planted count, so a felled woodland
    // grows fresh saplings toward that ceiling over the seasons.
    let (mut sim, raws) = wood_fort(3305);
    sim.plant_trees(30, &raws);
    let planted = sim.trees.len();
    assert!(planted > 0 && sim.tree_cap > planted, "there is room to regrow");
    // Thin the forest, leaving room under the cap for regrowth.
    let doomed: Vec<_> = sim.trees.keys().take(planted / 2).copied().collect();
    for t in doomed {
        sim.trees.remove(&t);
    }
    let thinned = sim.trees.len();

    for _ in 0..(dk_core::TICKS_PER_DAY as usize * 120) {
        sim.step(&raws);
    }
    assert!(
        sim.trees.len() > thinned,
        "the forest regrew: {} -> {}",
        thinned,
        sim.trees.len()
    );
    assert!(sim.trees.len() <= sim.tree_cap, "but never past its ceiling");
}

#[test]
fn a_hand_planted_tree_never_regrows() {
    // Regression / determinism guard: trees inserted directly (as the headless
    // tests do) leave tree_cap at 0, so tick_regrowth draws no rng and grows
    // nothing -- a fort that didn't embark-plant a forest stays byte-identical.
    let (mut sim, raws) = wood_fort(3306);
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    let (tree, _) = sim.find_flat_patch(cx, cy).expect("a flat tile");
    sim.trees.insert(tree, oak(&raws));
    assert_eq!(sim.tree_cap, 0, "no cap without plant_trees");

    for _ in 0..(dk_core::TICKS_PER_DAY as usize * 60) {
        sim.step(&raws);
    }
    assert_eq!(sim.trees.len(), 1, "a lone hand-planted tree never spreads");
}

#[test]
fn a_wall_is_never_planned_over_a_tree() {
    // A wall raised over a standing tree would seal it inside the masonry.
    let (mut sim, raws) = wood_fort(3307);
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    let (tree, _) = sim.find_flat_patch(cx, cy).expect("a flat tile");
    sim.trees.insert(tree, oak(&raws));
    assert!(!sim.designate_construction(tree), "no wall may be planned on a tree");
}
