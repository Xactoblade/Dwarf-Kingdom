//! Combat-veterancy exit tests, run headlessly: dwarves who draw blood grow
//! into seasoned fighters whose blows land harder — so a fort's defenders
//! become deadlier the longer they survive the fight.

mod common;

use dk_agents::{fighting_bonus, Faction, Sim, Skill};
use dk_world::path::Pos;

fn brawl_fort(seed: u64) -> (Sim, dk_raws::Raws) {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, seed);
    let mut sim = Sim::new(map, &raws, rng, 3);
    sim.invasions = false;
    (sim, raws)
}

#[test]
fn a_landed_blow_hones_prowess() {
    // A veteran hits strictly harder than a raw recruit, at every level.
    assert_eq!(fighting_bonus(0), 0, "a green recruit gets no bonus");
    assert!(fighting_bonus(6) > fighting_bonus(0), "a master hits harder");
    assert!(fighting_bonus(3) > fighting_bonus(1), "prowess climbs with level");
}

#[test]
fn fighters_grow_seasoned_by_battle() {
    let (mut sim, raws) = brawl_fort(5501);
    // No Fighting skill to begin with.
    assert_eq!(sim.dwarves[0].skill_level(Skill::Fighting), 0);
    assert_eq!(sim.veterans(), 0);

    // Drop a raider right next to a citizen and let them trade blows until
    // the fight is settled.
    let hero_pos = sim.dwarves[0].pos;
    sim.spawn_raider_at(Pos::new(hero_pos.x + 1, hero_pos.y, hero_pos.z), &raws);
    let mut fought = false;
    for _ in 0..4000 {
        sim.step(&raws);
        if sim
            .dwarves
            .iter()
            .any(|d| d.faction == Faction::Fort && d.skills.get(&Skill::Fighting).copied().unwrap_or(0) > 0)
        {
            fought = true;
        }
        if sim.alive_hostiles() == 0 {
            break;
        }
    }
    assert!(fought, "a citizen who traded blows should gain Fighting experience");
}
