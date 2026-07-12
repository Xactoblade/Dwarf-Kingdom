//! Adventure region-travel exit tests, run headlessly: a hero can journey
//! from one land to another, carrying their body, skills, deeds, and quest,
//! while the old region's people are left behind and the nemesis follows.

mod common;

use dk_agents::{Faction, PlayerAction, SiegeLeader, SiegeRoster, Sim};
use dk_history::World;

fn adventure_sim(seed: u64) -> (Sim, dk_raws::Raws, String) {
    let world = World::generate(seed, 48, 48, 80);
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 48, 48, 16, seed);
    let mut sim = Sim::new(map, &raws, rng, 1);
    sim.invasions = false;
    let (civ_name, leaders) = world.siege_pack(24, 24).expect("a nemesis");
    let nemesis = leaders[0].0.clone();
    sim.siege_roster = Some(SiegeRoster {
        civ_name,
        leaders: leaders
            .into_iter()
            .map(|(name, grudge)| SiegeLeader { name, grudge })
            .collect(),
    });
    (sim, raws, nemesis)
}

/// Generate a fresh local map for "another region" (a different seed).
fn another_land(raws: &dk_raws::Raws, seed: u64) -> dk_world::Map {
    let mut rng = dk_core::rng_from_seed(seed ^ 0xABCD);
    dk_world::generate(&raws.materials, &mut rng, 48, 48, 16, seed ^ 0xABCD)
}

#[test]
fn the_hero_carries_their_life_into_a_new_land() {
    let (mut sim, raws, nemesis) = adventure_sim(2201);
    let hero = sim.begin_adventure(&raws).unwrap();
    // Give the hero a distinguishing history.
    sim.dwarves[hero].name = "Wanderer".to_string();
    sim.deeds.push("slew a lesser foe".to_string());
    // Wound them so we can confirm the body travels intact.
    sim.dwarves[hero].body[2].hp = 5;
    let carried_hp = sim.dwarves[hero].body[2].hp;
    let deeds_before = sim.deeds.len();

    // Populate the old land so we can confirm it's left behind.
    let old_population = sim.dwarves.len();
    assert!(old_population >= 2, "the old land had the hero and a nemesis");

    sim.relocate_player(another_land(&raws, 2201), &raws);

    // The hero survives the journey, singular, at the new land's edge.
    let p = sim.player.expect("still an adventurer");
    assert_eq!(sim.dwarves[p].name, "Wanderer");
    assert_eq!(sim.dwarves[p].body[2].hp, carried_hp, "wounds travel with you");
    assert_eq!(sim.deeds.len(), deeds_before, "deeds are remembered");
    assert!(sim.map.walkable(sim.dwarves[p].pos), "the hero stands on solid ground");
    assert!(
        sim.log.iter().any(|(_, m)| m.contains("travels into a new land")),
        "the journey is recorded"
    );

    // The nemesis followed (quest still live), so exactly hero + nemesis.
    assert_eq!(
        sim.dwarves.iter().filter(|d| d.alive).count(),
        2,
        "the old land's crowd is gone; only hero and the following nemesis remain"
    );
    let follower = sim
        .dwarves
        .iter()
        .find(|d| d.faction == Faction::Hostile && d.name == nemesis);
    assert!(follower.is_some(), "the quarry follows the hunt");
    assert!(sim.log.iter().any(|(_, m)| m.contains("followed you here")));
}

#[test]
fn the_hero_can_still_act_after_traveling() {
    let (mut sim, raws, _) = adventure_sim(2202);
    sim.begin_adventure(&raws).unwrap();
    sim.relocate_player(another_land(&raws, 2202), &raws);
    let t0 = sim.clock.tick;
    // The world only moves when the traveled hero acts.
    assert!(sim.player_step(PlayerAction::Wait, &raws));
    assert!(sim.clock.tick > t0, "the hero acts in the new land");
}
