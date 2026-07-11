//! Phase 6 exit tests (BLUEPRINT.md §5, Phase 6), run headlessly:
//! "kill (in adventure mode) the named beast that destroyed your fortress,
//! then read the whole saga" — here: the same simulation played through a
//! turn-based lens, a nemesis from world history hunted down by hand, and
//! the deed recorded.

mod common;

use dk_agents::{Faction, PlayerAction, SiegeLeader, SiegeRoster, Sim};
use dk_history::World;
use dk_world::path::Pos;

fn adventure_sim(seed: u64) -> (Sim, dk_raws::Raws, String) {
    let world = World::generate(seed, 48, 48, 80);
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 48, 48, 16, seed);
    let mut sim = Sim::new(map, &raws, rng, 1);
    sim.invasions = false;
    let (civ_name, leaders) = world.siege_pack(24, 24).expect("a nemesis exists");
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

#[test]
fn the_world_only_moves_when_the_player_does() {
    let (mut sim, raws, _) = adventure_sim(601);
    sim.begin_adventure(&raws).expect("a hero steps forward");
    let t0 = sim.clock.tick;
    // No player action, no time (the app never calls step directly in
    // adventure mode).
    assert_eq!(sim.clock.tick, t0);
    assert!(sim.player_step(PlayerAction::Wait, &raws));
    assert!(sim.clock.tick > t0, "acting advances the world");
}

#[test]
fn hunt_down_the_nemesis_and_record_the_deed() {
    let (mut sim, raws, nemesis) = adventure_sim(602);
    let hero = sim.begin_adventure(&raws).expect("a hero steps forward");
    assert_eq!(
        sim.quest.as_ref().map(|(n, done)| (n.as_str(), *done)),
        Some((nemesis.as_str(), false)),
        "the quest names the historical enemy"
    );
    let target_idx = sim
        .dwarves
        .iter()
        .position(|d| d.faction == Faction::Hostile && d.name == nemesis)
        .expect("the nemesis walks the same map");

    // March toward the quarry, swinging when adjacent. The nemesis also
    // hunts us, so the distance closes from both sides.
    let mut turns = 0;
    while sim.dwarves[target_idx].alive && turns < 4000 {
        let me = sim.dwarves[hero].pos;
        let them = sim.dwarves[target_idx].pos;
        let dx = (them.x - me.x).signum();
        let dy = (them.y - me.y).signum();
        // Prefer closing the larger gap; fall back to the other axis.
        let action = if (them.x - me.x).abs() >= (them.y - me.y).abs() && dx != 0 {
            PlayerAction::Move(dx, 0)
        } else if dy != 0 {
            PlayerAction::Move(0, dy)
        } else if dx != 0 {
            PlayerAction::Move(dx, 0)
        } else {
            PlayerAction::Wait
        };
        let alive = sim.player_step(action, &raws);
        if !alive {
            panic!("the hero died on the road after {turns} turns");
        }
        turns += 1;
    }
    assert!(
        !sim.dwarves[target_idx].alive,
        "the nemesis should fall within {turns} turns"
    );
    assert_eq!(
        sim.quest.as_ref().map(|(_, done)| *done),
        Some(true),
        "the quest completes"
    );
    assert!(
        sim.deeds.iter().any(|d| d.contains(&nemesis)),
        "the deed is recorded: {:?}",
        sim.deeds
    );
    assert!(
        sim.log
            .iter()
            .any(|(_, m)| m.contains("slew") && m.contains(&nemesis)),
        "the saga is readable in the log"
    );
}

#[test]
fn the_player_can_climb_stairs() {
    let raws = common::test_raws();
    let mut m = dk_world::Map::new_air(8, 8, 4, 0);
    for y in 0..8 {
        for x in 0..8 {
            m.set(x, y, 0, dk_world::Tile::solid(2));
            m.set(x, y, 1, dk_world::Tile::floor(2));
        }
    }
    let stairs = |m: &mut dk_world::Map, x: usize, y: usize, z: usize| {
        m.set(x, y, z, dk_world::Tile { material: 2, shape: dk_world::TileShape::Stairs, water: 0 });
    };
    stairs(&mut m, 4, 4, 1);
    stairs(&mut m, 4, 4, 2);
    m.set(5, 4, 2, dk_world::Tile::floor(2));
    let rng = dk_core::rng_from_seed(603);
    let mut sim = Sim::new(m, &raws, rng, 1);
    sim.water.springs.clear();
    sim.rebuild_caches();
    sim.invasions = false;
    let hero = sim.begin_adventure(&raws).unwrap();
    sim.dwarves[hero].pos = Pos::new(4, 4, 1); // on the lower stair

    assert!(sim.player_step(PlayerAction::Climb(1), &raws));
    assert_eq!(sim.dwarves[hero].pos, Pos::new(4, 4, 2), "climbed the stairs");
    assert!(sim.player_step(PlayerAction::Move(1, 0), &raws));
    assert_eq!(sim.dwarves[hero].pos, Pos::new(5, 4, 2), "stepped off at the top");
}
