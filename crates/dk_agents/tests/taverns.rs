//! Tavern exit tests, run headlessly: a stressed dwarf seeks the tavern,
//! unwinds (shedding stress and gaining a good thought), and socializes
//! with fellow patrons.

mod common;

use dk_agents::{ItemKind, Sim, Task, ThoughtKind, TAVERN_STRESS_AT};
use dk_core::TICKS_PER_DAY;
use dk_world::path::Pos;

fn tavern_fort(seed: u64) -> (Sim, dk_raws::Raws) {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, seed);
    let mut sim = Sim::new(map, &raws, rng, 4);
    sim.invasions = false;
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    sim.place_flat_stockpiles(cx, cy, 18);
    // A tavern on flat ground, stocked with a few drinks.
    let (ta, tb) = sim.find_flat_patch(cx, cy).expect("tavern site");
    sim.add_tavern(ta, tb);
    for c in [ta, Pos::new(ta.x + 1, ta.y, ta.z), Pos::new(ta.x + 2, ta.y, ta.z)] {
        sim.debug_spawn_drink(c);
    }
    (sim, raws)
}

#[test]
fn a_stressed_dwarf_unwinds_at_the_tavern() {
    let (mut sim, raws) = tavern_fort(1301);
    // Wind one dwarf up tight.
    sim.dwarves[0].stress = TAVERN_STRESS_AT + 30.0;
    let stress_before = sim.dwarves[0].stress;

    // They should head to the tavern and relax.
    let mut relaxed = false;
    for _ in 0..20_000 {
        sim.step(&raws);
        if matches!(sim.dwarves[0].task, Task::Relax { .. }) {
            relaxed = true;
        }
        if sim.dwarves[0]
            .thoughts
            .iter()
            .any(|(_, t)| *t == ThoughtKind::RelaxedAtTavern)
        {
            break;
        }
    }
    assert!(relaxed, "a stressed dwarf should seek the tavern");
    assert!(
        sim.dwarves[0].stress < stress_before,
        "unwinding sheds stress ({} -> {})",
        stress_before,
        sim.dwarves[0].stress
    );
    assert!(
        sim.dwarves[0]
            .thoughts
            .iter()
            .any(|(_, t)| *t == ThoughtKind::RelaxedAtTavern),
        "a good thought comes of it"
    );
}

#[test]
fn tavern_goers_drink_and_socialize() {
    let (mut sim, raws) = tavern_fort(1302);
    // Two stressed dwarves — they should meet at the tavern and bond.
    sim.dwarves[0].stress = TAVERN_STRESS_AT + 40.0;
    sim.dwarves[1].stress = TAVERN_STRESS_AT + 40.0;
    let drinks_before = sim.count_kind(ItemKind::Drink);

    for _ in 0..(TICKS_PER_DAY / 2) {
        sim.step(&raws);
    }
    // Some drink was consumed at the tavern.
    assert!(
        sim.count_kind(ItemKind::Drink) < drinks_before,
        "tavern-goers drink the stock"
    );
    // The two built at least a little rapport.
    let rapport = sim.dwarves[0].relationships.get(&1).copied().unwrap_or(0)
        + sim.dwarves[1].relationships.get(&0).copied().unwrap_or(0);
    assert!(rapport > 0, "sharing the tavern builds relationships");
}

#[test]
fn no_tavern_means_no_relaxing() {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(1303);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, 1303);
    let mut sim = Sim::new(map, &raws, rng, 3);
    sim.invasions = false;
    sim.dwarves[0].stress = TAVERN_STRESS_AT + 30.0;
    for _ in 0..3_000 {
        sim.step(&raws);
        assert!(
            !matches!(sim.dwarves[0].task, Task::Relax { .. }),
            "without a tavern there is nowhere to unwind"
        );
    }
}
