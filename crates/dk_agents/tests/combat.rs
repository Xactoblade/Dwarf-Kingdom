//! Combat (BLUEPRINT.md §2.5): damage types meet armour and flesh the way
//! Dwarf Fortress's do. An edge cuts and is turned by equal armour; a point
//! defeats armour better; a blunt head drives its mass straight through it. A
//! keener or harder metal beats a softer one, and adamantine beats everything.

mod common;

use dk_agents::{resolve_blow, CombatStats, DamageType, ItemKind, Sim, WeaponKind};
use dk_world::path::Pos;

fn mat(sharpness: f32, density: f32, hardness: f32) -> CombatStats {
    CombatStats { sharpness, density, hardness }
}

// The fort's metals, for reference.
fn iron() -> CombatStats {
    mat(1.0, 7.8, 100.0)
}
fn copper() -> CombatStats {
    mat(0.9, 8.9, 40.0)
}
fn adamantine() -> CombatStats {
    mat(10.0, 0.2, 200.0)
}

fn edge(w: CombatStats, armor: Option<CombatStats>) -> i16 {
    resolve_blow(15.0, Some((DamageType::Edge, WeaponKind::Sword.heft(), w)), armor).damage
}
fn pierce(w: CombatStats, armor: Option<CombatStats>) -> i16 {
    resolve_blow(15.0, Some((DamageType::Pierce, WeaponKind::Spear.heft(), w)), armor).damage
}
fn blunt(w: CombatStats, armor: Option<CombatStats>) -> i16 {
    resolve_blow(15.0, Some((DamageType::Blunt, WeaponKind::Hammer.heft(), w)), armor).damage
}

#[test]
fn a_blade_is_turned_by_armour_a_hammer_is_not() {
    // The reason a fort forges hammers as well as swords. Against an equal
    // suit of armour a sword nearly bounces, while the hammer's mass carries
    // through — "adamantine armour only prevents an estimated 13% of blows
    // from a war hammer".
    let sword_bare = edge(iron(), None);
    let sword_armored = edge(iron(), Some(iron()));
    let hammer_bare = blunt(iron(), None);
    let hammer_armored = blunt(iron(), Some(iron()));

    assert!(sword_armored * 3 < sword_bare, "armour turns the edge ({sword_armored} vs {sword_bare})");
    assert!(
        hammer_armored * 2 > sword_armored * 3,
        "a hammer beats an armoured foe where a sword cannot ({hammer_armored} vs {sword_armored})"
    );
    assert!(hammer_armored * 3 > hammer_bare, "and the hammer barely notices the armour");
}

#[test]
fn a_point_defeats_armour_better_than_an_edge() {
    // A spear concentrates its force, so it punches through where a slash is
    // turned — the reason spears are prized against armoured enemies.
    assert!(
        pierce(iron(), Some(iron())) > edge(iron(), Some(iron())),
        "the point defeats the armour the edge could not"
    );
}

#[test]
fn a_better_metal_beats_a_lesser_one() {
    // Copper is soft: it holds no edge and folds, so a copper sword bites far
    // worse than an iron one, and adamantine cuts anything at all.
    assert!(edge(copper(), None) * 2 < edge(iron(), None), "copper is a poor blade");
    assert!(edge(adamantine(), None) > edge(iron(), None) * 5, "adamantine murders");
    assert!(
        edge(adamantine(), Some(iron())) > edge(iron(), None),
        "and adamantine through armour still beats iron in the open"
    );
}

#[test]
fn a_bare_fist_is_feeble() {
    let fist = resolve_blow(15.0, None, None).damage;
    assert!(fist < edge(iron(), None), "a fist is no match for a blade");
    assert!(fist >= 1, "but it still does something");
}

#[test]
fn a_landed_blow_always_does_something_and_draws_blood() {
    // Even the worst weapon against the best armour leaves a mark.
    let b = resolve_blow(8.0, Some((DamageType::Edge, 1.0, copper())), Some(adamantine()));
    assert!(b.damage >= 1);
    assert!(b.bleed >= 1);
}

// --- and the same rules, driven through a real fight ---

fn duel_fort(seed: u64) -> (Sim, dk_raws::Raws) {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, seed);
    let mut sim = Sim::new(map, &raws, rng, 2);
    sim.invasions = false;
    (sim, raws)
}

#[test]
fn an_armed_soldier_beats_a_bare_handed_raider() {
    let (mut sim, raws) = duel_fort(7801);
    // Arm the first dwarf: enlist and hand the armoury an iron sword.
    sim.toggle_soldier(sim.dwarves[0].pos);
    let iron_idx = raws.materials.index_of("hematite").unwrap();
    let sp = sim.dwarves[0].pos;
    sim.debug_spawn_item(ItemKind::Weapon, iron_idx, sp);
    // A bare-handed raider walks up next to the soldier.
    sim.debug_spawn_raider_at(Pos::new(sp.x + 1, sp.y, sp.z), &raws);

    let raider = sim
        .dwarves
        .iter()
        .position(|d| d.alive && d.faction == dk_agents::Faction::Hostile)
        .expect("a raider");
    let mut won = false;
    for _ in 0..4_000 {
        sim.step(&raws);
        if !sim.dwarves[raider].alive {
            won = true;
            break;
        }
    }
    assert!(won, "an armed soldier cuts down a bare-handed raider");
    assert!(sim.dwarves[0].alive, "and lives to tell it");
}
