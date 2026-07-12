//! Smelter exit tests, run headlessly: a smelter refines raw stone/ore
//! boulders into metal bars — the first stage of the fort's metal industry and
//! the stock the forge works into weapons. Without a smelter, no bars appear,
//! so the whole chain is gated on the building existing.

mod common;

use dk_agents::{BuildingKind, ItemKind, Sim};

fn smelt_fort(seed: u64) -> (Sim, dk_raws::Raws) {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, seed);
    let mut sim = Sim::new(map, &raws, rng, 4);
    sim.invasions = false;
    (sim, raws)
}

#[test]
fn a_smelter_turns_boulders_into_bars() {
    let (mut sim, raws) = smelt_fort(8101);
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;

    sim.add_embark_supplies(&raws);
    let (sa, _) = sim.find_flat_patch(cx, cy).expect("smelter site");
    assert!(sim.add_building(BuildingKind::Smelter, sa));
    sim.place_flat_stockpiles(cx, cy, 18);
    // Enlist a soldier so the fort actually wants bars (they feed weapons).
    sim.toggle_soldier(sim.dwarves[0].pos);
    let sp = sim.dwarves[0].pos;
    for _ in 0..8 {
        sim.debug_spawn_boulder(0, sp);
    }
    assert_eq!(sim.count_kind(ItemKind::Bar), 0, "no bars smelted yet");

    let mut made = false;
    for _ in 0..12_000 {
        sim.step(&raws);
        if sim.stats.bars_smelted > 0 {
            made = true;
            break;
        }
    }
    assert!(made, "the smelter should refine a boulder into a bar");
    assert!(sim.count_kind(ItemKind::Bar) > 0, "a metal bar exists in the fort");
}

#[test]
fn no_smelter_means_no_bars() {
    // A forge alone, with no smelter, can never produce bars — and so, with
    // the forge now working bars instead of raw stone, no weapons either.
    let (mut sim, raws) = smelt_fort(8102);
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;

    sim.add_embark_supplies(&raws);
    let (fa, _) = sim.find_flat_patch(cx, cy).expect("forge site");
    assert!(sim.add_building(BuildingKind::Forge, fa));
    sim.place_flat_stockpiles(cx, cy, 18);
    sim.toggle_soldier(sim.dwarves[0].pos);
    let sp = sim.dwarves[0].pos;
    for _ in 0..8 {
        sim.debug_spawn_boulder(0, sp);
    }

    for _ in 0..4_000 {
        sim.step(&raws);
    }
    assert_eq!(sim.stats.bars_smelted, 0, "no smelter, no bars");
    assert_eq!(sim.count_kind(ItemKind::Bar), 0, "no bars without a smelter");
    assert_eq!(
        sim.stats.weapons_forged, 0,
        "the forge has no bars to work, so no weapons"
    );
}
