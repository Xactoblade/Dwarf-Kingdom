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

/// A fort built from the real shipped raws (which define steel and mark the
/// flux stones), for the steelmaking tests.
fn real_smelt_fort(seed: u64) -> (Sim, dk_raws::Raws) {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let raws = dk_raws::Raws::load(&dir).expect("load real raws");
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

#[test]
fn iron_smelted_with_flux_becomes_steel() {
    // Dwarf Fortress's steelmaking: iron ore + a flux stone, cooked together at
    // the smelter, yield steel — a harder metal than plain iron. Needs the real
    // raws (which define steel and mark the flux stones), not the minimal test
    // fixture.
    let (mut sim, raws) = real_smelt_fort(8104);
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;

    sim.add_embark_supplies(&raws);
    let (sa, _) = sim.find_flat_patch(cx, cy).expect("smelter site");
    assert!(sim.add_building(BuildingKind::Smelter, sa));
    sim.place_flat_stockpiles(cx, cy, 18);
    sim.toggle_soldier(sim.dwarves[0].pos); // the fort wants bars

    let hematite = raws.materials.index_of("hematite").expect("iron ore exists");
    let limestone = raws.materials.index_of("limestone").expect("flux stone exists");
    let steel = raws.materials.index_of("steel").expect("steel exists");
    let sp = sim.dwarves[0].pos;
    for _ in 0..4 {
        sim.debug_spawn_boulder(hematite, sp);
        sim.debug_spawn_boulder(limestone, sp);
    }

    let mut got_steel = false;
    for _ in 0..12_000 {
        sim.step(&raws);
        if sim.items.iter().any(|it| it.active() && it.kind == ItemKind::Bar && it.stuff == steel) {
            got_steel = true;
            break;
        }
    }
    assert!(got_steel, "iron smelted with flux on hand should yield a steel bar");
}

#[test]
fn iron_without_flux_stays_iron() {
    // No flux, no steel: the same iron ore smelts down to a plain iron bar.
    let (mut sim, raws) = real_smelt_fort(8105);
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;

    sim.add_embark_supplies(&raws);
    let (sa, _) = sim.find_flat_patch(cx, cy).expect("smelter site");
    assert!(sim.add_building(BuildingKind::Smelter, sa));
    sim.place_flat_stockpiles(cx, cy, 18);
    sim.toggle_soldier(sim.dwarves[0].pos);

    let hematite = raws.materials.index_of("hematite").expect("iron ore exists");
    let steel = raws.materials.index_of("steel").expect("steel exists");
    let sp = sim.dwarves[0].pos;
    for _ in 0..6 {
        sim.debug_spawn_boulder(hematite, sp);
    }

    for _ in 0..12_000 {
        sim.step(&raws);
        if sim.stats.bars_smelted > 0 {
            break;
        }
    }
    assert!(sim.stats.bars_smelted > 0, "iron was smelted");
    assert!(
        !sim.items.iter().any(|it| it.active() && it.kind == ItemKind::Bar && it.stuff == steel),
        "with no flux, no steel is made"
    );
}
