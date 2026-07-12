//! Barracks exit tests, run headlessly: an enlisted soldier drills at the
//! barracks between battles and grows more skilled in fighting for it — and
//! without a barracks, no such peacetime training happens.

mod common;

use dk_agents::{Sim, Skill, Task};
use dk_core::TICKS_PER_DAY;

fn garrison(seed: u64, with_barracks: bool) -> (Sim, dk_raws::Raws) {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, seed);
    let mut sim = Sim::new(map, &raws, rng, 4);
    sim.invasions = false;
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    sim.place_flat_stockpiles(cx, cy, 18);
    sim.add_embark_supplies(&raws);
    if with_barracks {
        let (a, b) = sim.find_flat_patch(cx, cy).expect("barracks site");
        sim.add_barracks(a, b);
    }
    // Enlist dwarf 0 as a soldier.
    let p = sim.dwarves[0].pos;
    sim.toggle_soldier(p);
    (sim, raws)
}

fn fighting_xp(sim: &Sim, i: usize) -> u32 {
    sim.dwarves[i].skills.get(&Skill::Fighting).copied().unwrap_or(0)
}

#[test]
fn a_soldier_drills_at_the_barracks_and_grows_skilled() {
    let (mut sim, raws) = garrison(7001, true);
    assert_eq!(fighting_xp(&sim, 0), 0, "a raw recruit knows no fighting");

    let mut drilled = false;
    for _ in 0..(TICKS_PER_DAY * 3) {
        sim.step(&raws);
        if matches!(sim.dwarves[0].task, Task::Spar { .. }) {
            drilled = true;
        }
    }
    assert!(drilled, "an idle soldier should drill at the barracks");
    assert!(
        fighting_xp(&sim, 0) > 0,
        "and gain fighting experience from the drilling"
    );
}

#[test]
fn without_a_barracks_soldiers_do_not_train_in_peace() {
    let (mut sim, raws) = garrison(7002, false);
    for _ in 0..(TICKS_PER_DAY * 2) {
        sim.step(&raws);
        assert!(!matches!(sim.dwarves[0].task, Task::Spar { .. }));
    }
    assert_eq!(
        fighting_xp(&sim, 0),
        0,
        "no barracks, no peacetime fighting practice"
    );
}
