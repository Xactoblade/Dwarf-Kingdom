//! Necromancer tests, run headlessly: a necromancer raises the fort's fallen
//! dead as hostile undead. A map with no necromancer is untouched — the whole
//! mechanic is gated on a living necromancer, so it draws no rng and changes
//! nothing without one (necromancers arrive only with invasion sieges, which no
//! headless test enables).

mod common;

use dk_agents::{Faction, ItemKind, Sim};
use dk_world::path::Pos;

fn fort(seed: u64, n: usize) -> (Sim, dk_raws::Raws) {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, seed);
    let mut sim = Sim::new(map, &raws, rng, n);
    sim.invasions = false;
    (sim, raws)
}

#[test]
fn a_necromancer_raises_a_nearby_corpse() {
    // A lone necromancer (hostile) beside a corpse — no fort dwarves to fight,
    // so it simply works its dark art.
    let (mut sim, raws) = fort(9701, 1);
    let z = sim.dwarves[0].pos.z;
    sim.dwarves[0].necromancer = true;
    sim.dwarves[0].faction = Faction::Hostile;
    sim.dwarves[0].pos = Pos::new(10, 10, z);
    sim.debug_spawn_corpse(Pos::new(11, 10, z));
    let before = sim.dwarves.len();

    let mut raised = false;
    for _ in 0..400 {
        sim.step(&raws);
        if sim.stats.raised > 0 {
            raised = true;
            break;
        }
    }
    assert!(raised, "the necromancer should raise the corpse");
    assert!(sim.dwarves.len() > before, "an undead now walks");
    assert_eq!(sim.count_kind(ItemKind::Corpse), 0, "the corpse was consumed in the raising");
    assert!(
        sim.dwarves.last().is_some_and(|d| d.alive && d.faction == Faction::Hostile),
        "the risen dead are hostile"
    );
}

#[test]
fn corpses_lie_still_without_a_necromancer() {
    let (mut sim, raws) = fort(9702, 1);
    let z = sim.dwarves[0].pos.z;
    sim.debug_spawn_corpse(Pos::new(11, 10, z));
    for _ in 0..400 {
        sim.step(&raws);
    }
    assert_eq!(sim.stats.raised, 0, "no necromancer, no rising");
    assert!(sim.count_kind(ItemKind::Corpse) > 0, "the corpse still lies where it fell");
}
