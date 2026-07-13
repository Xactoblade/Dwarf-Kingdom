//! Statue exit tests, run headlessly: a mason carves stone into statues, and a
//! fort adorned with them is calmer — every citizen's stress eases a little
//! faster. A fort with no statues is unaffected (the whole effect is gated on a
//! statue existing, so it draws no rng and shifts no stress without one).

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
fn a_mason_carves_stone_into_statues() {
    let (mut sim, raws) = fort(2201, 4);
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;

    sim.add_embark_supplies(&raws);
    let (ma, _) = sim.find_flat_patch(cx, cy).expect("mason site");
    assert!(sim.add_building(BuildingKind::Mason, ma));
    sim.place_flat_stockpiles(cx, cy, 24);
    let sp = sim.dwarves[0].pos;
    for _ in 0..12 {
        sim.debug_spawn_boulder(0, sp);
    }
    assert_eq!(sim.count_kind(ItemKind::Statue), 0, "nothing carved yet");

    let mut carved = false;
    for _ in 0..12_000 {
        sim.step(&raws);
        if sim.stats.statues_carved > 0 {
            carved = true;
            break;
        }
    }
    assert!(carved, "the mason should carve a statue from stone");
    assert!(sim.count_kind(ItemKind::Statue) > 0, "a statue stands in the fort");
}

#[test]
fn a_hall_of_statues_calms_the_fort() {
    // Two identical one-dwarf forts at the same seed; the only difference is a
    // statue, so the rng streams stay in lockstep and the sole variable is the
    // calmer stress-drain of an adorned fort.
    let setup = |adorned: bool| {
        let (mut sim, raws) = fort(2202, 1);
        sim.add_embark_supplies(&raws);
        sim.dwarves[0].stress = 100.0;
        if adorned {
            let sp = sim.dwarves[0].pos;
            sim.debug_spawn_statue(0, sp); // no rng — keeps the streams aligned
        }
        (sim, raws)
    };
    let (mut adorned, ra) = setup(true);
    let (mut bare, rb) = setup(false);

    for _ in 0..3_000 {
        adorned.step(&ra);
        bare.step(&rb);
    }

    assert!(
        adorned.dwarves[0].stress < bare.dwarves[0].stress,
        "a fort of statues should soothe its people (adorned {} vs bare {})",
        adorned.dwarves[0].stress,
        bare.dwarves[0].stress
    );
}
