//! Fortress-fall exit tests, run headlessly: when the last citizen dies the
//! fortress falls, its end is recorded, and it leaves an epitaph of its
//! deeds — but a lone adventurer's death is not a fortress falling.

mod common;

use dk_agents::Sim;

fn small_fort(seed: u64, n: usize) -> (Sim, dk_raws::Raws) {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, seed);
    let mut sim = Sim::new(map, &raws, rng, n);
    sim.invasions = false;
    (sim, raws)
}

#[test]
fn the_fortress_falls_when_the_last_dwarf_dies() {
    let (mut sim, raws) = small_fort(2101, 3);
    assert!(!sim.fallen(), "a peopled fort has not fallen");
    sim.step(&raws);
    assert!(!sim.fallen());

    // Slay them one by one; only the last death fells the fortress.
    sim.slay(0);
    sim.step(&raws);
    assert!(!sim.fallen(), "one death is not the end");
    sim.slay(1);
    sim.step(&raws);
    assert!(!sim.fallen());
    sim.slay(2);
    sim.step(&raws);
    assert!(sim.fallen(), "the last citizen's death fells the fortress");
    assert!(sim.fallen_at.is_some());
    assert!(
        sim.log.iter().any(|(_, m)| m.contains("fortress has fallen")),
        "the fall is recorded"
    );

    // The epitaph recounts its story.
    let epitaph = sim.epitaph();
    assert!(epitaph.contains("endured"));
    assert!(epitaph.contains("remembered"));
}

#[test]
fn an_adventurers_death_is_not_a_fortress_falling() {
    let (mut sim, raws) = small_fort(2102, 1);
    sim.begin_adventure(&raws).expect("a hero sets out");
    let hero = sim.player.unwrap();
    // The lone hero dies — but this is an adventure, not a fort.
    sim.slay(hero);
    for _ in 0..100 {
        sim.step(&raws);
    }
    assert!(
        !sim.fallen(),
        "an adventurer's death must not be treated as a fortress falling"
    );
}
