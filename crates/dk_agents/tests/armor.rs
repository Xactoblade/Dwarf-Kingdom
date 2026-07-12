//! Armor exit tests, run headlessly: the forge works smelted bars into plate,
//! the armory issues it to soldiers, and a soldier in armor takes markedly less
//! harm from the same blows than a bare one. Armor exists only where a forge
//! made it, so an unarmored fort fights exactly as before.

mod common;

use dk_agents::{BuildingKind, ItemKind, Sim};
use dk_world::path::Pos;

/// A one-dwarf fort: the dwarf is enlisted as a soldier, optionally issued a
/// suit of armor, and a raider is dropped right beside them. Kept to a single
/// citizen so that — at a fixed seed — the ONLY thing that can differ between an
/// armored and a bare run is the damage armor turns aside: no other dwarf takes
/// a job and perturbs the shared rng stream.
fn duel(seed: u64, armored: bool) -> (Sim, dk_raws::Raws) {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, seed);
    let mut sim = Sim::new(map, &raws, rng, 1);
    sim.invasions = false;
    let sp = sim.dwarves[0].pos;
    sim.toggle_soldier(sp);
    if armored {
        sim.debug_spawn_armor(0, sp); // spawns no rng — keeps the streams aligned
        assert_eq!(sim.armored_soldiers(), 1, "the soldier dons the armor");
    }
    sim.spawn_raider_at(Pos::new(sp.x + 1, sp.y, sp.z), &raws);
    (sim, raws)
}

fn total_hp(sim: &Sim, i: usize) -> i32 {
    sim.dwarves[i].body.iter().map(|p| p.hp as i32).sum()
}

#[test]
fn armor_turns_aside_the_worst_of_a_blow() {
    let (mut armored, ra) = duel(9001, true);
    let (mut bare, rb) = duel(9001, false);
    let full = total_hp(&bare, 0);

    for _ in 0..250 {
        armored.step(&ra);
        bare.step(&rb);
    }

    let armored_hp = total_hp(&armored, 0);
    let bare_hp = total_hp(&bare, 0);
    assert!(bare_hp < full, "the bare soldier should have been wounded in the fight");
    assert!(
        armored_hp > bare_hp,
        "armor should spare the soldier harm (armored {armored_hp} vs bare {bare_hp})"
    );
}

#[test]
fn the_forge_armors_the_soldiers() {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(9002);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, 9002);
    let mut sim = Sim::new(map, &raws, rng, 4);
    sim.invasions = false;
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;

    sim.add_embark_supplies(&raws);
    let (fa, _) = sim.find_flat_patch(cx, cy).expect("forge site");
    assert!(sim.add_building(BuildingKind::Forge, fa));
    let (sa, _) = sim.find_flat_patch(cx, cy).expect("smelter site");
    assert!(sim.add_building(BuildingKind::Smelter, sa));
    sim.place_flat_stockpiles(cx, cy, 24);
    let sp = sim.dwarves[0].pos;
    for _ in 0..12 {
        sim.debug_spawn_boulder(0, sp);
    }
    sim.toggle_soldier(sp);
    assert_eq!(sim.armored_soldiers(), 0, "no armor forged yet");

    let mut made = false;
    for _ in 0..16_000 {
        sim.step(&raws);
        if sim.stats.armor_forged > 0 {
            made = true;
            break;
        }
    }
    assert!(made, "the smith should forge armor from a smelted bar");
    assert!(sim.count_kind(ItemKind::Armor) > 0, "a suit of armor exists in the fort");
    assert!(sim.armored_soldiers() >= 1, "the armory outfits an enlisted soldier");
}
