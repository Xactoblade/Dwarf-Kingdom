//! Forge / weapons exit tests, run headlessly: a forge turns boulders into
//! weapons, the fortress armory arms its soldiers, and an armed soldier
//! strikes harder than a bare-fisted one.

mod common;

use dk_agents::{
    resolve_blow, BuildingKind, CombatStats, DamageType, ItemKind, Sim,
};

fn forge_fort(seed: u64) -> (Sim, dk_raws::Raws) {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, seed);
    let mut sim = Sim::new(map, &raws, rng, 4);
    sim.invasions = false;
    (sim, raws)
}

#[test]
fn a_forged_blade_bites_far_deeper_than_a_bare_fist() {
    let iron = CombatStats { sharpness: 1.0, density: 7.8, hardness: 100.0 };
    let sword = Some((DamageType::Edge, 1.0, iron));
    let armed = resolve_blow(15.0, sword, None).damage;
    let fist = resolve_blow(15.0, None, None).damage;
    assert!(armed > fist * 3, "a sword ({armed}) is far deadlier than a fist ({fist})");
}

#[test]
fn a_forge_arms_the_soldiers() {
    let (mut sim, raws) = forge_fort(7701);
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;

    // The full metal chain: a smelter turns boulders into bars, a forge turns
    // bars into weapons, and the armory arms a soldier. Plus stone and food.
    sim.add_embark_supplies(&raws); // food & drink so the smiths aren't starving
    let (fa, _) = sim.find_flat_patch(cx, cy).expect("forge site");
    assert!(sim.add_building(BuildingKind::Forge, fa));
    let (sa, _) = sim.find_flat_patch(cx, cy).expect("smelter site");
    assert!(sim.add_building(BuildingKind::Smelter, sa));
    sim.place_flat_stockpiles(cx, cy, 18);
    // Boulders on a known-walkable tile so the smelter can reach them.
    let sp = sim.dwarves[0].pos;
    for _ in 0..10 {
        sim.debug_spawn_boulder(0, sp);
    }
    // Enlist a soldier so the fort wants weapons.
    sim.toggle_soldier(sim.dwarves[0].pos);
    assert_eq!(sim.armed_soldiers(), 0, "no weapons forged yet");

    let mut forged = false;
    for _ in 0..12_000 {
        sim.step(&raws);
        if sim.stats.weapons_forged > 0 {
            forged = true;
        }
        if sim.armed_soldiers() > 0 {
            break;
        }
    }
    assert!(sim.stats.bars_smelted > 0, "the smelter should smelt a bar from ore");
    assert!(forged, "the smith should forge a weapon from a bar");
    assert!(
        sim.count_kind(ItemKind::Weapon) > 0,
        "a weapon exists in the fort"
    );
    assert!(
        sim.armed_soldiers() >= 1,
        "the armory arms an enlisted soldier"
    );
}
