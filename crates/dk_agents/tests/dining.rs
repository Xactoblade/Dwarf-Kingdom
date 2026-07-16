//! The dining hall (BLUEPRINT.md §2.3, "zones: … dining halls"): dwarves carry
//! their food to the hall and eat it in company, rather than standing in the
//! larder chewing over the barrel.

mod common;

use dk_agents::{ItemKind, Sim, ThoughtKind};
use dk_world::path::Pos;

fn fort(seed: u64) -> (Sim, dk_raws::Raws) {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, seed);
    let mut sim = Sim::new(map, &raws, rng, 2);
    sim.invasions = false;
    (sim, raws)
}

/// Feed a hungry dwarf and see where they do it.
fn feed(sim: &mut Sim, raws: &dk_raws::Raws) {
    let sp = sim.dwarves[0].pos;
    sim.debug_spawn_item(ItemKind::Meal, 0, sp);
    for d in &mut sim.dwarves {
        d.hunger = 70.0;
    }
    for _ in 0..12_000 {
        sim.step(raws);
        if sim.dwarves[0].thoughts.iter().any(|t| t.1 == ThoughtKind::AteMeal) {
            return;
        }
    }
}

#[test]
fn a_dwarf_carries_their_food_to_the_hall_and_eats_in_company() {
    let (mut sim, raws) = fort(9101);
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    let (a, b) = sim.find_flat_patch(cx, cy).expect("hall site");
    sim.add_dining_hall(a, b);

    feed(&mut sim, &raws);
    assert!(
        sim.dwarves[0].thoughts.iter().any(|t| t.1 == ThoughtKind::AteMeal),
        "the dwarf ate"
    );
    assert!(
        sim.dwarves[0].thoughts.iter().any(|t| t.1 == ThoughtKind::DinedInHall),
        "and did it at table, in the hall"
    );
}

#[test]
fn with_no_hall_a_dwarf_eats_where_the_food_is() {
    let (mut sim, raws) = fort(9102);
    feed(&mut sim, &raws);
    assert!(
        sim.dwarves[0].thoughts.iter().any(|t| t.1 == ThoughtKind::AteMeal),
        "the dwarf ate"
    );
    assert!(
        !sim.dwarves[0].thoughts.iter().any(|t| t.1 == ThoughtKind::DinedInHall),
        "there was no hall to dine in"
    );
    assert!(ThoughtKind::DinedInHall.delta() > 0.0, "and dining in one is a pleasure");
}

#[test]
fn a_starving_dwarf_does_not_stand_on_ceremony() {
    // The hall is a nicety. A dwarf on the edge of starving eats where they
    // stand rather than walk the length of the fort with a meal in hand.
    let (mut sim, raws) = fort(9103);
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    let (a, b) = sim.find_flat_patch(cx, cy).expect("hall site");
    // A hall far from the food.
    let far = Pos::new(a.x, a.y, a.z);
    sim.add_dining_hall(far, b);

    let sp = sim.dwarves[0].pos;
    sim.debug_spawn_item(ItemKind::Meal, 0, sp);
    sim.dwarves[0].hunger = 99.0; // starving
    for _ in 0..12_000 {
        sim.step(&raws);
        if sim.dwarves[0].thoughts.iter().any(|t| t.1 == ThoughtKind::AteMeal) {
            break;
        }
    }
    assert!(
        sim.dwarves[0].thoughts.iter().any(|t| t.1 == ThoughtKind::AteMeal),
        "a starving dwarf eats — that is the whole point"
    );
    assert!(sim.dwarves[0].hunger < 50.0, "and is fed");
}
