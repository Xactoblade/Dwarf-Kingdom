//! Clothier exit tests, run headlessly: a clothier's shop sews the fort's
//! woven cloth into clothes, and a dwarf dressed in them frets a little less.
//! Clothes are worn by citizens in index order (the same model as beds and the
//! armory). A fort with no clothes is unaffected — the whole system is gated on
//! a set of clothes existing, so it draws no rng and shifts no stress without one.

mod common;

use dk_agents::{BuildingKind, ItemKind, Sim};

fn fort(seed: u64, dwarves: usize) -> (Sim, dk_raws::Raws) {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, seed);
    let mut sim = Sim::new(map, &raws, rng, dwarves);
    sim.invasions = false;
    (sim, raws)
}

#[test]
fn a_clothier_sews_cloth_into_clothes() {
    let (mut sim, raws) = fort(5401, 4);
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;

    sim.add_embark_supplies(&raws);
    let (ca, _) = sim.find_flat_patch(cx, cy).expect("clothier site");
    assert!(sim.add_building(BuildingKind::Clothier, ca));
    sim.place_flat_stockpiles(cx, cy, 18);
    // Bolts of cloth on a known-walkable tile for the tailor to work.
    let sp = sim.dwarves[0].pos;
    for _ in 0..6 {
        sim.debug_spawn_cloth(sp);
    }
    assert_eq!(sim.count_kind(ItemKind::Clothes), 0, "nothing sewn yet");

    let mut sewn = false;
    for _ in 0..12_000 {
        sim.step(&raws);
        if sim.stats.clothes_sewn > 0 {
            sewn = true;
            break;
        }
    }
    assert!(sewn, "the tailor should sew a bolt of cloth into clothes");
    assert!(sim.count_kind(ItemKind::Clothes) > 0, "a set of clothes exists in the fort");
}

#[test]
fn a_no_clothes_fort_needs_no_clothier() {
    // A clothier with no cloth can sew nothing — the chain is gated on cloth.
    let (mut sim, raws) = fort(5402, 4);
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    sim.add_embark_supplies(&raws);
    let (ca, _) = sim.find_flat_patch(cx, cy).expect("clothier site");
    assert!(sim.add_building(BuildingKind::Clothier, ca));
    sim.place_flat_stockpiles(cx, cy, 18);

    for _ in 0..3_000 {
        sim.step(&raws);
    }
    assert_eq!(sim.stats.clothes_sewn, 0, "no cloth, no clothes");
    assert_eq!(sim.count_kind(ItemKind::Clothes), 0);
}

#[test]
fn fine_clothes_ease_the_mind() {
    // Two identical one-dwarf forts at the same seed; the only difference is a
    // set of clothes, so the rng streams stay in lockstep and the sole variable
    // is the calmer stress-drain of a well-dressed dwarf.
    let setup = |bed_clothes: bool| {
        let (mut sim, raws) = fort(5403, 1);
        sim.add_embark_supplies(&raws); // food & drink so needs don't dominate
        sim.dwarves[0].stress = 100.0;
        if bed_clothes {
            let sp = sim.dwarves[0].pos;
            sim.debug_spawn_clothes(sp); // no rng — keeps the streams aligned
        }
        (sim, raws)
    };
    let (mut dressed, rd) = setup(true);
    let (mut ragged, rr) = setup(false);

    for _ in 0..3_000 {
        dressed.step(&rd);
        ragged.step(&rr);
    }

    assert!(
        dressed.dwarves[0].stress < ragged.dwarves[0].stress,
        "a well-dressed dwarf should fret less (dressed {} vs ragged {})",
        dressed.dwarves[0].stress,
        ragged.dwarves[0].stress
    );
}
