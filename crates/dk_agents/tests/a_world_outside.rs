//! Phase 4 exit test (BLUEPRINT.md §5, Phase 4), run headlessly:
//! "two worlds with different seeds feel like different places, and a siege
//! leader has a name you can find in Legends with a personal reason to
//! hate you."

mod common;

use dk_agents::{Faction, Sim, SiegeLeader, SiegeRoster};
use dk_history::World;

/// Convert world history into the sim's siege wiring. The selection logic
/// lives in `World::siege_pack` — the same call the app makes at embark —
/// so this test exercises the shipping glue, not a copy of it.
fn roster_from_world(world: &World, embark: (usize, usize)) -> SiegeRoster {
    let (civ_name, leaders) = world
        .siege_pack(embark.0, embark.1)
        .expect("worldgen guarantees a hostile civ");
    SiegeRoster {
        civ_name,
        leaders: leaders
            .into_iter()
            .map(|(name, grudge)| SiegeLeader { name, grudge })
            .collect(),
    }
}

#[test]
fn siege_leader_is_findable_in_legends_with_a_reason() {
    let world = World::generate(2026, 48, 48, 80);
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(2026);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, 2026);
    let mut sim = Sim::new(map, &raws, rng, 5);
    let roster = roster_from_world(&world, (24, 24));
    assert!(!roster.leaders.is_empty(), "history must supply siege leaders");
    let expected_leader = roster.leaders[0].clone();
    sim.siege_roster = Some(roster);

    sim.spawn_raiders(3, &raws);

    // The named leader walks among the raiders...
    assert!(
        sim.dwarves
            .iter()
            .any(|d| d.faction == Faction::Hostile && d.name == expected_leader.name),
        "a raider must bear the historical leader's name"
    );
    // ...the fort log names them and their reason...
    let log_line = sim
        .log
        .iter()
        .find(|(_, m)| m.contains(&expected_leader.name))
        .map(|(_, m)| m.clone())
        .expect("siege announcement names the leader");
    assert!(
        log_line.contains(&expected_leader.grudge),
        "the announcement must carry the grudge: {log_line}"
    );
    // ...and Legends corroborates both the figure and the grudge.
    let legends = world.legends_lines();
    assert!(
        legends.iter().any(|l| l.contains(&expected_leader.name)),
        "the leader exists in world history"
    );
    assert!(
        legends.iter().any(|l| l.contains(&expected_leader.name)
            && (l.contains("vengeance") || l.contains("destruction") || l.contains("hatred"))),
        "the leader's personal reason is readable in Legends"
    );
}

#[test]
fn different_seeds_produce_different_enemies() {
    let a = World::generate(11, 48, 48, 80);
    let b = World::generate(12, 48, 48, 80);
    let ra = roster_from_world(&a, (24, 24));
    let rb = roster_from_world(&b, (24, 24));
    let names_a: Vec<&String> = ra.leaders.iter().map(|l| &l.name).collect();
    let names_b: Vec<&String> = rb.leaders.iter().map(|l| &l.name).collect();
    assert_ne!(
        (&ra.civ_name, names_a),
        (&rb.civ_name, names_b),
        "different worlds must breed different enemies"
    );
}
