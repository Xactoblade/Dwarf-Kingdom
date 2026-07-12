//! Glass-industry exit tests, run headlessly: a glass furnace melts stone
//! into blown glass — the fort's finest ordinary trade good — and a skilled
//! glassblower turns out finer, dearer pieces.

mod common;

use dk_agents::{item_value, BuildingKind, ItemKind, Sim};

fn glass_fort(seed: u64) -> (Sim, dk_raws::Raws) {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, seed);
    let mut sim = Sim::new(map, &raws, rng, 4);
    sim.invasions = false;
    (sim, raws)
}

#[test]
fn glass_is_the_finest_ordinary_trade_good() {
    let raws = common::test_raws();
    let glass = dk_agents::Item {
        kind: ItemKind::Glass,
        stuff: 0,
        name: None,
        pos: dk_world::path::Pos::new(0, 0, 0),
        state: dk_agents::ItemState::OnGround,
        reserved_by: None,
        consumed: false,
        quality: 0,
    };
    let cut_gem = dk_agents::Item { kind: ItemKind::CutGem, ..glass.clone() };
    assert!(
        item_value(&glass, &raws) >= item_value(&cut_gem, &raws),
        "blown glass is at least as prized as a cut gem"
    );
    // Quality lifts it further.
    let masterwork = dk_agents::Item { quality: 5, ..glass.clone() };
    assert!(item_value(&masterwork, &raws) > item_value(&glass, &raws));
}

#[test]
fn a_glassblower_melts_stone_into_glass() {
    let (mut sim, raws) = glass_fort(9101);
    sim.add_embark_supplies(&raws);
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    let (wa, _) = sim.find_flat_patch(cx, cy).expect("furnace site");
    assert!(sim.add_building(BuildingKind::GlassFurnace, wa));
    sim.place_flat_stockpiles(cx, cy, 18);
    // Boulders on a known-walkable tile for the glassblower to melt.
    let sp = sim.dwarves[0].pos;
    for _ in 0..8 {
        sim.debug_spawn_boulder(0, sp);
    }

    let mut blown = false;
    for _ in 0..30_000 {
        sim.step(&raws);
        if sim.stats.glass_blown > 0 && sim.count_kind(ItemKind::Glass) > 0 {
            blown = true;
            break;
        }
    }
    assert!(blown, "the glassblower should turn stone into glass");
    for it in sim.items.iter().filter(|i| i.active() && i.kind == ItemKind::Glass) {
        assert!(it.quality <= 5);
    }
}
