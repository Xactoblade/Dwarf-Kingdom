//! Burrow / alarm exit tests, run headlessly: when the alarm sounds, the
//! fort's civilians drop their work and flee to a burrow, while enlisted
//! soldiers hold the line; when it lifts, everyone goes back to work.

mod common;

use dk_agents::{Faction, Sim, Task};

fn fort(seed: u64) -> (Sim, dk_raws::Raws) {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, seed);
    let mut sim = Sim::new(map, &raws, rng, 5);
    sim.invasions = false;
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    sim.place_flat_stockpiles(cx, cy, 18);
    sim.add_embark_supplies(&raws);
    let (a, b) = sim.find_flat_patch(cx, cy).expect("burrow site");
    sim.add_burrow(a, b);
    (sim, raws)
}

#[test]
fn the_alarm_sends_civilians_to_the_burrow() {
    let (mut sim, raws) = fort(8801);
    // Enlist dwarf 0 as a soldier; the rest are civilians.
    let p = sim.dwarves[0].pos;
    sim.toggle_soldier(p);

    assert!(sim.toggle_alarm(), "the alarm is now sounded");
    let mut sheltered = false;
    for _ in 0..4000 {
        sim.step(&raws);
        // Every living civilian ends up sheltering or already in the burrow.
        let civilians_safe = sim.dwarves.iter().all(|d| {
            !d.alive
                || d.faction != Faction::Fort
                || d.soldier
                || matches!(d.task, Task::Shelter { .. })
                || sim.burrow_at(d.pos)
        });
        if civilians_safe
            && sim
                .dwarves
                .iter()
                .any(|d| d.alive && !d.soldier && matches!(d.task, Task::Shelter { .. }))
        {
            sheltered = true;
            break;
        }
    }
    assert!(sheltered, "civilians should flee to the burrow when the alarm sounds");
    // The soldier does NOT shelter — they hold the line.
    assert!(
        !matches!(sim.dwarves[0].task, Task::Shelter { .. }),
        "an enlisted soldier ignores the alarm"
    );

    // Lift the alarm: sheltering civilians return to work.
    assert!(!sim.toggle_alarm(), "the alarm is lifted");
    let mut back_to_work = false;
    for _ in 0..4000 {
        sim.step(&raws);
        if sim
            .dwarves
            .iter()
            .all(|d| !matches!(d.task, Task::Shelter { .. }))
        {
            back_to_work = true;
            break;
        }
    }
    assert!(back_to_work, "when the alarm lifts, no one is left sheltering");
}

#[test]
fn sheltering_does_not_freeze_the_stressed_or_starve_the_thirsty() {
    // Two regressions from the burrows review: a max-stress civilian must not
    // oscillate Shelter<->Tantrum (and so never reach the burrow), and a
    // parched civilian must be let out to drink rather than starve under a
    // long alarm.
    let (mut sim, raws) = fort(8803);
    // dwarf 1 is deeply stressed; dwarf 2 is very thirsty. Neither is a soldier.
    sim.dwarves[1].stress = 130.0;
    sim.dwarves[2].thirst = 90.0;
    let thirst0 = sim.dwarves[2].thirst;
    sim.toggle_alarm();

    let mut stressed_reached_burrow = false;
    let mut thirst_relieved = false;
    for _ in 0..12_000 {
        sim.step(&raws);
        if sim.burrow_at(sim.dwarves[1].pos) {
            stressed_reached_burrow = true;
        }
        if sim.dwarves[2].thirst < thirst0 {
            thirst_relieved = true;
        }
        if stressed_reached_burrow && thirst_relieved {
            break;
        }
    }
    assert!(
        stressed_reached_burrow,
        "a stressed civilian still reaches the burrow (no tantrum oscillation)"
    );
    assert!(
        thirst_relieved,
        "a thirsty civilian is let out to drink rather than starve under the alarm"
    );
    assert!(sim.dwarves.iter().all(|d| d.alive), "nobody dies under the alarm");
}

#[test]
fn no_alarm_means_business_as_usual() {
    let (mut sim, raws) = fort(8802);
    // Alarm never sounded: nobody shelters.
    for _ in 0..2000 {
        sim.step(&raws);
        assert!(sim.dwarves.iter().all(|d| !matches!(d.task, Task::Shelter { .. })));
    }
}
