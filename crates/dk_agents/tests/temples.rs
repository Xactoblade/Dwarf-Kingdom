//! Temple exit tests, run headlessly: dwarves periodically seek the temple
//! to worship, finding peace (a good thought and a little less stress) on a
//! prayer cadence distinct from the tavern's stress-driven visits.

mod common;

use dk_agents::{Sim, Task, ThoughtKind, PRAYER_INTERVAL};
use dk_core::TICKS_PER_DAY;

fn temple_fort(seed: u64) -> (Sim, dk_raws::Raws) {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, seed);
    let mut sim = Sim::new(map, &raws, rng, 4);
    sim.invasions = false;
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    sim.place_flat_stockpiles(cx, cy, 18);
    let (ta, tb) = sim.find_flat_patch(cx, cy).expect("temple site");
    sim.add_temple(ta, tb);
    (sim, raws)
}

#[test]
fn the_devout_visit_the_temple_and_find_peace() {
    let (mut sim, raws) = temple_fort(2001);
    // Give a dwarf some stress so the peace of prayer is measurable.
    sim.dwarves[0].stress = 30.0;
    let stress_before = sim.dwarves[0].stress;

    let mut prayed = false;
    // A little over the prayer interval, so worship comes due.
    for _ in 0..(PRAYER_INTERVAL + 5 * TICKS_PER_DAY) {
        sim.step(&raws);
        if matches!(sim.dwarves[0].task, Task::Pray { .. }) {
            prayed = true;
        }
        if sim.dwarves[0]
            .thoughts
            .iter()
            .any(|(_, t)| *t == ThoughtKind::PrayedAtTemple)
        {
            break;
        }
    }
    assert!(prayed, "a dwarf overdue for worship should seek the temple");
    assert!(
        sim.dwarves[0]
            .thoughts
            .iter()
            .any(|(_, t)| *t == ThoughtKind::PrayedAtTemple),
        "worship brings a moment of peace"
    );
    assert!(sim.dwarves[0].last_prayer > 0, "the prayer is remembered");
    assert!(
        sim.dwarves[0].stress <= stress_before,
        "prayer soothes rather than stresses"
    );
}

#[test]
fn without_a_temple_nobody_prays() {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(2002);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, 2002);
    let mut sim = Sim::new(map, &raws, rng, 3);
    sim.invasions = false;
    for _ in 0..(PRAYER_INTERVAL + 2 * TICKS_PER_DAY) {
        sim.step(&raws);
        for d in &sim.dwarves {
            assert!(!matches!(d.task, Task::Pray { .. }));
        }
    }
}
