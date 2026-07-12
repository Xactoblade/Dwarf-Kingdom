//! Animal-training exit tests, run headlessly: a dog can be trained for war
//! by a handler, and a trained war dog guards the fort — charging raiders and
//! savaging them, at the risk of its own life. Livestock cannot be trained.

mod common;

use dk_agents::{AnimalKind, Faction, Sim};
use dk_world::path::Pos;

fn kennel_fort(seed: u64) -> (Sim, dk_raws::Raws) {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, seed);
    let mut sim = Sim::new(map, &raws, rng, 4);
    sim.invasions = false;
    (sim, raws)
}

/// A flat tile near the dwarves where an animal can stand and be reached.
fn spot_near(sim: &Sim) -> Pos {
    let p = sim.dwarves[0].pos;
    Pos::new(p.x + 1, p.y, p.z)
}

#[test]
fn only_dogs_can_be_war_trained() {
    let (mut sim, _raws) = kennel_fort(6601);
    let here = spot_near(&sim);
    sim.add_animal(AnimalKind::Cow, here, true);
    assert!(
        sim.mark_nearest_for_war(here).is_none(),
        "livestock cannot be war-trained"
    );
    sim.add_animal(AnimalKind::Dog, here, true);
    assert!(
        sim.mark_nearest_for_war(here).is_some(),
        "a grown dog can be marked for training"
    );
}

#[test]
fn a_handler_trains_a_dog_for_war() {
    let (mut sim, raws) = kennel_fort(6602);
    let dog = sim.add_animal(AnimalKind::Dog, spot_near(&sim), true);
    assert!(!sim.animals[dog].war);
    sim.mark_nearest_for_war(spot_near(&sim)).expect("mark the dog");

    // Give a handler time to walk over and put the dog through its paces.
    let mut trained = false;
    for _ in 0..6000 {
        sim.step(&raws);
        if sim.animals[dog].war {
            trained = true;
            break;
        }
    }
    assert!(trained, "a handler should train the marked dog for war");
    assert!(!sim.animals[dog].war_marked, "the training order is cleared");
    assert!(
        sim.log.iter().any(|(_, m)| m.contains("trained for war")),
        "the achievement is recorded"
    );
}

#[test]
fn a_war_dog_savages_a_raider() {
    let (mut sim, raws) = kennel_fort(6603);
    // A ready-made war dog standing guard.
    let dog = sim.add_animal(AnimalKind::Dog, spot_near(&sim), true);
    sim.animals[dog].war = true;
    let dpos = sim.animals[dog].pos;
    // A lone raider a couple of tiles away for the dog to charge.
    sim.spawn_raider_at(Pos::new(dpos.x + 3, dpos.y, dpos.z), &raws);
    let raider = sim.dwarves.iter().position(|d| d.faction == Faction::Hostile).unwrap();
    let full: i16 = sim.dwarves[raider].body.iter().map(|p| p.hp).sum();

    let mut bit = false;
    for _ in 0..2000 {
        sim.step(&raws);
        let now: i16 = sim.dwarves[raider].body.iter().map(|p| p.hp).sum();
        if !sim.dwarves[raider].alive || now < full {
            bit = true;
            break;
        }
    }
    assert!(bit, "a war dog should close on a raider and draw blood");
}
