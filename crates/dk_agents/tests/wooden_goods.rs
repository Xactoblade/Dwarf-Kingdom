//! Wood species & wooden goods, run headlessly: a felled tree yields a log of
//! its own wood, a carpenter works that log into furniture and art that keep
//! the wood, and a regrown sapling is the same species as the grove it spread
//! from. So the fort's beds and statues come in oak, pine, and the rest.

mod common;

use dk_agents::{BuildingKind, ItemKind, Sim};
use dk_core::TICKS_PER_DAY;
use dk_raws::MaterialCategory;
use dk_world::path::Pos;

fn wood_fort(seed: u64, n: usize) -> (Sim, dk_raws::Raws) {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, seed);
    let mut sim = Sim::new(map, &raws, rng, n);
    sim.invasions = false;
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    sim.add_embark_supplies(&raws);
    sim.place_flat_stockpiles(cx, cy, 24);
    (sim, raws)
}

#[test]
fn a_carpenter_builds_furniture_of_the_logs_wood() {
    // A carpenter with a pile of oak logs and no stone beds should build a
    // wooden bed that carries the oak wood — furniture in its own species.
    let (mut sim, raws) = wood_fort(7701, 4);
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    let (ca, _) = sim.find_flat_patch(cx, cy).expect("carpenter site");
    assert!(sim.add_building(BuildingKind::Carpenter, ca));

    let oak = raws.materials.indices_in_category(MaterialCategory::Wood)[0];
    for _ in 0..24 {
        if let Some((p, _)) = sim.find_flat_patch(cx, cy) {
            sim.debug_spawn_item(ItemKind::Log, oak, p);
        }
    }

    let mut oak_bed = false;
    for _ in 0..15_000 {
        sim.step(&raws);
        if sim
            .items
            .iter()
            .any(|it| it.active() && it.kind == ItemKind::Bed && it.stuff == oak)
        {
            oak_bed = true;
            break;
        }
    }
    assert!(oak_bed, "the carpenter should build an oak bed from the oak logs");
}

#[test]
fn a_felled_tree_yields_a_log_of_its_own_wood() {
    let (mut sim, raws) = wood_fort(7703, 4);
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    let woods = raws.materials.indices_in_category(MaterialCategory::Wood);
    let pine = woods[1];
    let (tree, _) = sim.find_flat_patch(cx, cy).expect("a flat tile");
    sim.trees.insert(tree, pine);
    sim.designate_rect(dk_agents::DesignationKind::Chop, tree, tree);

    let mut logged = false;
    for _ in 0..12_000 {
        sim.step(&raws);
        if sim.stats.trees_felled > 0 {
            logged = true;
            break;
        }
    }
    assert!(logged, "the tree was felled");
    assert!(
        sim.items
            .iter()
            .any(|it| it.active() && it.kind == ItemKind::Log && it.stuff == pine),
        "the log is of the tree's own pine wood"
    );
}

#[test]
fn a_regrown_sapling_inherits_its_groves_wood() {
    // Saplings spread from a tree share its species, so an oak grove stays oak
    // as it regrows.
    let (mut sim, raws) = wood_fort(7702, 4);
    let oak = raws.materials.indices_in_category(MaterialCategory::Wood)[0];
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    for dx in -3..=3 {
        for dy in -3..=3 {
            let (x, y) = (cx + dx, cy + dy);
            if let Some(z) = sim.map.walk_surface_z(x as usize, y as usize) {
                let p = Pos::new(x, y, z as i32);
                if sim.map.walkable(p) {
                    sim.trees.insert(p, oak);
                }
            }
        }
    }
    let initial = sim.trees.len();
    assert!(initial > 0, "the grove was planted");
    // Room to spread well beyond the planted size.
    sim.tree_cap = initial + 40;

    for _ in 0..(TICKS_PER_DAY as usize * 200) {
        sim.step(&raws);
    }
    assert!(sim.trees.len() > initial, "the grove regrew: {} -> {}", initial, sim.trees.len());
    assert!(
        sim.trees.values().all(|&s| s == oak),
        "every tree, planted or regrown, is oak"
    );
}
