//! Forgotten beast exit tests, run headlessly: a named horror rises from
//! the deep, is far tougher than any raider, hunts the fort, and — when
//! finally slain — is counted as a beast, not a mere raider.

mod common;

use dk_agents::{Faction, Sim};
use dk_world::path::Pos;
use dk_world::{Map, Tile};

fn arena(dwarves: usize) -> (Sim, dk_raws::Raws) {
    let raws = common::test_raws();
    let mut m = Map::new_air(24, 24, 4, 0);
    for y in 0..24 {
        for x in 0..24 {
            m.set(x, y, 0, Tile::solid(2));
            m.set(x, y, 1, Tile::floor(2));
        }
    }
    let rng = dk_core::rng_from_seed(1601);
    let mut sim = Sim::new(m, &raws, rng, dwarves);
    sim.water.springs.clear();
    sim.invasions = false;
    sim.rebuild_caches();
    (sim, raws)
}

#[test]
fn a_named_beast_rises_from_the_deep() {
    let (mut sim, raws) = arena(3);
    let idx = sim.spawn_forgotten_beast(Pos::new(12, 12, 1), &raws);
    let beast = &sim.dwarves[idx];
    assert!(beast.beast, "it is a beast");
    assert_eq!(beast.faction, Faction::Hostile);
    assert!(!beast.name.is_empty());
    // Far tougher than a mortal: total HP dwarfs a raider's ~145.
    let total_hp: i32 = beast.body.iter().map(|p| p.max_hp as i32).sum();
    assert!(total_hp > 500, "a beast is monstrously tough ({total_hp})");
    assert!(
        sim.log.iter().any(|(_, m)| m.contains("forgotten beast has risen")),
        "its emergence is announced with dread"
    );
}

#[test]
fn a_beast_shrugs_off_what_would_fell_a_raider() {
    // One dwarf pitted against one beast: the dwarf should die (or at least
    // the beast should still stand) long after a raider would have fallen.
    let (mut sim, raws) = arena(1);
    let hero = 0;
    let beast = sim.spawn_forgotten_beast(Pos::new(13, 12, 1), &raws);
    sim.dwarves[hero].pos = Pos::new(11, 12, 1);

    for _ in 0..8_000 {
        sim.step(&raws);
    }
    // A single dwarf cannot slay a forgotten beast bare-handed.
    assert!(
        sim.dwarves[beast].alive,
        "one dwarf should not be able to fell a beast"
    );
    assert_eq!(sim.stats.beasts_slain, 0);
}

#[test]
fn a_slain_beast_is_counted_as_a_beast() {
    let (mut sim, raws) = arena(2);
    let beast = sim.spawn_forgotten_beast(Pos::new(12, 12, 1), &raws);
    // Bring it to the brink: destroy every part but leave the torso at 1.
    for part in &mut sim.dwarves[beast].body {
        if !matches!(part.kind, dk_agents::PartKind::Torso) {
            part.hp = 1;
        }
    }
    if let Some(torso) = sim.dwarves[beast]
        .body
        .iter_mut()
        .find(|p| matches!(p.kind, dk_agents::PartKind::Torso))
    {
        torso.hp = 1;
    }
    // Put two dwarves right on top of it; their blows finish the job.
    sim.dwarves[0].pos = Pos::new(12, 11, 1);
    sim.dwarves[1].pos = Pos::new(12, 13, 1);

    for _ in 0..5_000 {
        sim.step(&raws);
        if !sim.dwarves[beast].alive {
            break;
        }
    }
    assert!(!sim.dwarves[beast].alive, "two dwarves finish a weakened beast");
    assert_eq!(sim.stats.beasts_slain, 1, "the kill is credited as a beast");
    assert_eq!(sim.stats.raiders_slain, 0, "a beast is no mere raider");
}
