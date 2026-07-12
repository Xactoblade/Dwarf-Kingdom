//! Companion exit tests, run headlessly: a hero can recruit a nearby
//! townsfolk to travel at their side. Companions shadow the hero, fight
//! adjacent enemies, and journey on into new lands — while un-recruited
//! folk are left behind.

mod common;

use dk_agents::{Faction, Sim};
use dk_world::path::Pos;

fn adventure_with_neighbor(seed: u64) -> (Sim, dk_raws::Raws, usize) {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 48, 48, 16, seed);
    let mut sim = Sim::new(map, &raws, rng, 4);
    sim.invasions = false;
    let hero = sim.begin_adventure(&raws).unwrap();
    let hp = sim.dwarves[hero].pos;
    // Scatter every other fort-dweller far away, then stand exactly one of
    // them at the hero's elbow — so the recruit target is unambiguous.
    let others: Vec<usize> = sim
        .dwarves
        .iter()
        .enumerate()
        .filter(|&(j, d)| j != hero && d.alive && d.faction == Faction::Fort)
        .map(|(j, _)| j)
        .collect();
    for &j in &others {
        sim.dwarves[j].pos = Pos::new(hp.x, hp.y + 10, hp.z);
    }
    let ally = others[0];
    sim.dwarves[ally].pos = Pos::new(hp.x + 1, hp.y, hp.z);
    (sim, raws, ally)
}

fn another_land(raws: &dk_raws::Raws, seed: u64) -> dk_world::Map {
    let mut rng = dk_core::rng_from_seed(seed ^ 0xABCD);
    dk_world::generate(&raws.materials, &mut rng, 48, 48, 16, seed ^ 0xABCD)
}

#[test]
fn the_hero_recruits_a_companion_at_their_side() {
    let (mut sim, _raws, ally) = adventure_with_neighbor(3301);
    let name = sim.dwarves[ally].name.clone();

    let recruited = sim.recruit_companion().expect("someone stood adjacent");
    assert_eq!(recruited, name, "the adjacent townsfolk joins");
    assert!(sim.dwarves[ally].follower, "they are now a follower");
    assert!(
        sim.log.iter().any(|(_, m)| m.contains("joins your band")),
        "the recruitment is recorded"
    );

    // Nobody standing at arm's reach a second time -> nobody new joins.
    // (The one neighbour is already recruited.)
    let before = sim.dwarves.iter().filter(|d| d.follower).count();
    let _ = sim.recruit_companion();
    let after = sim.dwarves.iter().filter(|d| d.follower).count();
    assert_eq!(before, after, "no double-recruiting the same neighbour");
}

#[test]
fn a_companion_journeys_on_with_the_hero() {
    let (mut sim, raws, ally) = adventure_with_neighbor(3302);
    sim.recruit_companion().expect("recruit the neighbour");
    let comp_name = sim.dwarves[ally].name.clone();

    sim.relocate_player(another_land(&raws, 3302), &raws);

    // The hero (index 0) and their companion both stand in the new land.
    let p = sim.player.expect("still adventuring");
    assert_eq!(p, 0);
    let comp = sim
        .dwarves
        .iter()
        .find(|d| d.follower && d.name == comp_name)
        .expect("the companion travelled too");
    assert!(comp.alive);
    assert!(sim.map.walkable(comp.pos), "companion on solid ground");
    assert_ne!(comp.pos, sim.dwarves[0].pos, "not stacked on the hero");
}

#[test]
fn only_companions_are_carried_across_lands() {
    let (mut sim, raws, _ally) = adventure_with_neighbor(3303);
    sim.recruit_companion().expect("one companion");
    // There were 4 fort folk to start; only hero + 1 companion should remain.
    sim.relocate_player(another_land(&raws, 3303), &raws);
    let fort_folk = sim
        .dwarves
        .iter()
        .filter(|d| d.alive && d.faction == Faction::Fort)
        .count();
    assert_eq!(fort_folk, 2, "hero and one companion, the rest left behind");
}
