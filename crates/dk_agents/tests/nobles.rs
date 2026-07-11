//! Nobility exit tests, run headlessly: a baron arises with population,
//! issues mandates, rewards fulfillment, and punishes failure — feeding
//! the stress pipeline like everything else.

mod common;

use dk_agents::{BuildingKind, MandateKind, Sim, ThoughtKind, BARONY_AT, MANDATE_DAYS};
use dk_core::TICKS_PER_DAY;
use dk_world::path::Pos;

fn barony_fort(seed: u64) -> (Sim, dk_raws::Raws) {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, seed);
    let mut sim = Sim::new(map, &raws, rng, BARONY_AT + 1);
    sim.invasions = false;
    sim.add_embark_supplies(&raws);
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    let barley = raws.plants.index_of("barley").unwrap();
    if let Some((fa, fb)) = sim.find_flat_patch(cx, cy) {
        sim.add_farm(fa, fb, barley);
    }
    if let Some((wa, _)) = sim.find_flat_patch(cx, cy) {
        sim.add_building(BuildingKind::Still, wa);
        sim.add_building(BuildingKind::Kitchen, Pos::new(wa.x + 1, wa.y, wa.z));
    }
    sim.place_flat_stockpiles(cx, cy, 27);
    (sim, raws)
}

#[test]
fn a_baron_arises_and_makes_demands() {
    let (mut sim, raws) = barony_fort(901);
    assert!(sim.baron.is_none());

    // Within a few days of fort life, the barony is claimed and a demand made.
    for _ in 0..(3 * TICKS_PER_DAY) {
        sim.step(&raws);
    }
    let baron = sim.baron.expect("a fort of this size attracts a baron");
    assert!(sim.dwarves[baron].alive);
    assert!(
        sim.dwarves[baron]
            .thoughts
            .iter()
            .any(|(_, t)| *t == ThoughtKind::BecameBaron),
        "elevation is a life event"
    );
    assert!(sim.log.iter().any(|(_, m)| m.contains("elevated to baron")));
    assert!(sim.mandate.is_some(), "barons do not sit idle");
    assert!(sim.log.iter().any(|(_, m)| m.contains("demands that")));
}

#[test]
fn a_fulfilled_mandate_pleases_the_baron() {
    let (mut sim, raws) = barony_fort(902);
    for _ in 0..(3 * TICKS_PER_DAY) {
        sim.step(&raws);
    }
    let baron = sim.baron.expect("baron appointed");
    // Run the working fort for the mandate window: with a farm, still, and
    // kitchen going, production mandates get met in the normal course of
    // life (possibly across several mandates).
    let horizon = (MANDATE_DAYS + 25) * TICKS_PER_DAY;
    for _ in 0..horizon {
        sim.step(&raws);
        if sim.stats.mandates_met > 0 {
            break;
        }
    }
    assert!(
        sim.stats.mandates_met > 0,
        "a working fort should satisfy at least one mandate (failed: {})",
        sim.stats.mandates_failed
    );
    assert!(
        sim.dwarves[baron]
            .thoughts
            .iter()
            .any(|(_, t)| *t == ThoughtKind::MandateMet),
        "satisfaction is felt"
    );
    assert!(sim.log.iter().any(|(_, m)| m.contains("fulfilled")));
}

#[test]
fn a_failed_mandate_means_a_beating() {
    let (mut sim, raws) = barony_fort(903);
    for _ in 0..(3 * TICKS_PER_DAY) {
        sim.step(&raws);
    }
    let baron = sim.baron.expect("baron appointed");
    // Force an impossible demand: a mountain of boulders with no miners.
    sim.mandate = Some(dk_agents::Mandate {
        kind: MandateKind::MineBoulders,
        amount: 500,
        deadline: sim.clock.tick + 2 * TICKS_PER_DAY,
        baseline: sim.stats.boulders_mined,
    });
    for _ in 0..(3 * TICKS_PER_DAY) {
        sim.step(&raws);
        if sim.stats.mandates_failed > 0 {
            break;
        }
    }
    assert_eq!(sim.stats.mandates_failed, 1, "the impossible demand fails");
    let punished = sim
        .dwarves
        .iter()
        .enumerate()
        .find(|(_, d)| d.thoughts.iter().any(|(_, t)| *t == ThoughtKind::Punished));
    let (culprit, victim) = punished.expect("someone answers for it");
    assert_ne!(culprit, baron, "the baron never blames themselves");
    assert!(victim.stress > 0.0, "injustice is stressful");
    assert!(
        victim.body.iter().any(|p| p.hp < p.max_hp),
        "the beating leaves bruises"
    );
    assert!(victim.alive, "justice stops short of murder");
    assert!(sim.log.iter().any(|(_, m)| m.contains("is beaten")));
    // The fort watched, and hated it.
    let disturbed = sim
        .dwarves
        .iter()
        .filter(|d| d.thoughts.iter().any(|(_, t)| *t == ThoughtKind::SawPunishment))
        .count();
    assert!(disturbed >= 2, "punishment poisons the room");
}
