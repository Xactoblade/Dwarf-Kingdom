//! Hospital exit tests, run headlessly: a wounded dwarf seeks the hospital
//! and mends there far faster than they would out in the fort — and a fort
//! with no hospital heals only slowly, as before.

mod common;

use dk_agents::{PartKind, Sim, Task};
use dk_core::TICKS_PER_DAY;

fn hospital_fort(seed: u64, with_ward: bool) -> (Sim, dk_raws::Raws) {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, seed);
    let mut sim = Sim::new(map, &raws, rng, 4);
    sim.invasions = false;
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    sim.place_flat_stockpiles(cx, cy, 18);
    if with_ward {
        let (a, b) = sim.find_flat_patch(cx, cy).expect("ward site");
        sim.add_hospital(a, b);
    }
    (sim, raws)
}

/// Wound a dwarf: torso down to a third, and set it bleeding.
fn wound(sim: &mut Sim, i: usize) {
    for p in &mut sim.dwarves[i].body {
        if p.kind == PartKind::Torso {
            p.hp = p.max_hp / 3;
            p.bleeding = 3;
        }
    }
}

fn torso(sim: &Sim, i: usize) -> (i16, i16) {
    let p = sim.dwarves[i]
        .body
        .iter()
        .find(|p| p.kind == PartKind::Torso)
        .unwrap();
    (p.hp, p.max_hp)
}
fn torso_hp(sim: &Sim, i: usize) -> i16 {
    torso(sim, i).0
}

#[test]
fn the_wounded_seek_the_ward_and_mend() {
    let (mut sim, raws) = hospital_fort(6601, true);
    wound(&mut sim, 0);
    let start = torso_hp(&sim, 0);

    let mut rested = false;
    for _ in 0..(TICKS_PER_DAY * 3) {
        sim.step(&raws);
        if matches!(sim.dwarves[0].task, Task::Recover { .. }) {
            rested = true;
        }
        let (hp, max) = torso(&sim, 0);
        if hp >= max {
            break;
        }
    }
    assert!(rested, "a wounded dwarf should seek the hospital");
    assert!(
        torso_hp(&sim, 0) > start,
        "and mend there (was {start}, now {})",
        torso_hp(&sim, 0)
    );
    assert!(
        sim.dwarves[0].body.iter().all(|p| p.bleeding == 0),
        "the ward stanches the bleeding"
    );
}

#[test]
fn a_ward_heals_faster_than_the_open_fort() {
    // Same wound, same time: with a hospital vs without.
    let (mut with, raws) = hospital_fort(6602, true);
    let (mut without, _) = hospital_fort(6602, false);
    wound(&mut with, 0);
    wound(&mut without, 0);
    for _ in 0..(TICKS_PER_DAY * 2) {
        with.step(&raws);
        without.step(&raws);
    }
    assert!(
        torso_hp(&with, 0) >= torso_hp(&without, 0),
        "the ward mends at least as fast (ward {}, open {})",
        torso_hp(&with, 0),
        torso_hp(&without, 0)
    );
    // And meaningfully: the ward should have made real progress.
    let (whp, wmax) = torso(&with, 0);
    assert!(whp > torso_hp(&without, 0) || whp == wmax, "the ward made real progress");
}
