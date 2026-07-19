//! Severed body parts: a limb hacked past destruction is struck clean off,
//! dropping to the ground as gore while the stump sprays blood.

mod common;

use dk_agents::{ItemKind, PartKind, Sim};

fn fort(seed: u64) -> (Sim, dk_raws::Raws) {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, seed);
    let mut sim = Sim::new(map, &raws, rng, 4);
    sim.invasions = false;
    (sim, raws)
}

#[test]
fn a_severed_limb_drops_as_gore_and_sprays_blood() {
    let (mut sim, _raws) = fort(200);
    let d = 0;
    let pos = sim.dwarves[d].pos;
    assert!(sim.blood.is_empty(), "no blood before the blow");
    let gore_before = sim
        .items
        .iter()
        .filter(|i| i.kind == ItemKind::BodyPart)
        .count();

    sim.sever_part(d, PartKind::LeftArm);

    // A severed part now lies on the ground, named for the limb.
    let gore: Vec<_> = sim
        .items
        .iter()
        .filter(|i| i.kind == ItemKind::BodyPart)
        .collect();
    assert_eq!(gore.len(), gore_before + 1, "the arm dropped as gore");
    let part = gore.last().unwrap();
    assert_eq!(part.pos, pos, "it landed where the creature stood");
    assert_eq!(part.name.as_deref(), Some("severed left arm"));
    // The wound sprayed blood on the tile it fell on.
    assert!(
        sim.blood.get(&pos).copied().unwrap_or(0) > 0,
        "the stump sprayed blood on the ground"
    );
    // The stump gushes: the arm now bleeds hard.
    let arm = sim.dwarves[d]
        .body
        .iter()
        .find(|p| p.kind == PartKind::LeftArm)
        .unwrap();
    assert!(arm.bleeding >= 50, "the stump gushes ({})", arm.bleeding);
}
