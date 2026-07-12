//! Spoils-of-war exit tests, run headlessly: in an adventure, a slain raider
//! drops their weapon, the hero can take it up, and a wielded blade makes the
//! hero's blows land harder. A fortress (no played hero) sees no such drops.

mod common;

use dk_agents::{Faction, ItemKind, ItemState, PlayerAction, Sim};
use dk_world::path::Pos;

fn adventurer(seed: u64) -> (Sim, dk_raws::Raws) {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, seed);
    let mut sim = Sim::new(map, &raws, rng, 3);
    sim.invasions = false;
    sim.begin_adventure(&raws).expect("a hero sets out");
    (sim, raws)
}

#[test]
fn a_slain_raider_leaves_a_blade_the_hero_can_take_up() {
    let (mut sim, raws) = adventurer(4501);
    let hero = sim.player.unwrap();
    let hp = sim.dwarves[hero].pos;
    // A raider right beside the hero. Slay it; a weapon should drop.
    sim.spawn_raider_at(Pos::new(hp.x + 1, hp.y, hp.z), &raws);
    let raider = sim.dwarves.iter().rposition(|d| d.faction == Faction::Hostile).unwrap();
    let rpos = sim.dwarves[raider].pos;
    sim.slay(raider);
    let weapon = sim
        .items
        .iter()
        .find(|it| it.active() && it.kind == ItemKind::Weapon && it.pos == rpos);
    assert!(weapon.is_some(), "a fallen raider leaves their weapon");

    // The hero steps onto the blade and takes it up.
    // Move the hero to the weapon's tile (adjacent), then grab.
    sim.dwarves[hero].pos = rpos;
    assert!(sim.player_step(PlayerAction::Grab, &raws));
    let carried = sim.items.iter().any(|it| {
        it.active() && it.kind == ItemKind::Weapon && it.state == ItemState::Carried { by: hero }
    });
    assert!(carried, "the hero now wields the looted weapon");
}

#[test]
fn a_fortress_raid_drops_no_loot() {
    // No played hero: a raider dying in fortress mode leaves no weapon.
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(4502);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, 4502);
    let mut sim = Sim::new(map, &raws, rng, 3);
    sim.invasions = false;
    assert!(sim.player.is_none());
    let dp = sim.dwarves[0].pos;
    sim.spawn_raider_at(Pos::new(dp.x + 1, dp.y, dp.z), &raws);
    let raider = sim.dwarves.iter().position(|d| d.faction == Faction::Hostile).unwrap();
    sim.slay(raider);
    assert!(
        !sim.items.iter().any(|it| it.active() && it.kind == ItemKind::Weapon),
        "fortress raids leave no weapon drops (keeps raider-wealth determinism intact)"
    );
}
