//! Severed body parts: a limb hacked past destruction is struck clean off,
//! dropping to the ground as gore while the stump sprays blood.

mod common;

use dk_agents::{ItemKind, PartKind, Sim, GORE_ROT, GORE_SKELETONIZE};

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
    // The item stores the bare part; the display layer prefixes it by stage.
    assert_eq!(part.name.as_deref(), Some("left arm"));
    assert_eq!(part.stuff, 0, "it lands fresh (rot stage 0)");
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

#[test]
fn gore_rots_and_skeletonizes_over_time() {
    let (mut sim, _raws) = fort(201);
    sim.sever_part(0, PartKind::RightLeg);
    // The leg is the only gore item and items aren't removed, so its index holds.
    let g = sim
        .items
        .iter()
        .position(|i| i.kind == ItemKind::BodyPart)
        .expect("the severed leg");
    assert_eq!(sim.items[g].stuff, 0, "fresh when it falls");

    // A day on, the flesh has soured to carrion.
    sim.clock.tick = GORE_ROT;
    sim.decay_gore();
    assert_eq!(sim.items[g].stuff, 1, "rots within a day");

    // A few days on, it's picked clean to bone.
    sim.clock.tick = GORE_SKELETONIZE;
    sim.decay_gore();
    assert_eq!(sim.items[g].stuff, 2, "skeletonizes over a few days");

    // Bone is the last stage — it doesn't decay further or vanish.
    sim.clock.tick = GORE_SKELETONIZE * 10;
    sim.decay_gore();
    assert_eq!(sim.items[g].stuff, 2, "bone lies there for good");
}
