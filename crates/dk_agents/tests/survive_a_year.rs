//! Phase 2 exit test (BLUEPRINT.md §5, Phase 2), run headlessly:
//! "a 7-dwarf embark survives to year 2 with a working food industry, and a
//! new player can tell why a dwarf is unhappy" (thoughts exist and explain
//! happiness).

mod common;

use dk_agents::{BuildingKind, Sim};
use dk_core::{DAYS_PER_YEAR, TICKS_PER_DAY};
use dk_world::path::Pos;

#[test]
fn embark_survives_to_year_two() {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(2026);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, 2026);
    let mut sim = Sim::new(map, &raws, rng, 7);
    sim.invasions = false; // peace: this test is about the food economy
    sim.add_embark_supplies(&raws);

    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;

    // Two farms (one per crop), workshops beside them, stockpiles nearby.
    let barley = raws.plants.index_of("barley").unwrap();
    let potato = raws.plants.index_of("potato").unwrap();
    let (fa, fb) = sim.find_flat_patch(cx, cy).expect("farm site");
    assert!(sim.add_farm(fa, fb, barley) >= 6);
    let (fa2, fb2) = sim.find_flat_patch(cx, cy).expect("second farm site");
    assert!(sim.add_farm(fa2, fb2, potato) >= 6);

    let (wa, _) = sim.find_flat_patch(cx, cy).expect("workshop site");
    assert!(sim.add_building(BuildingKind::Still, wa));
    assert!(sim.add_building(BuildingKind::Kitchen, Pos::new(wa.x + 1, wa.y, wa.z)));

    sim.place_flat_stockpiles(cx, cy, 27);

    let original = sim.alive_dwarves();
    assert_eq!(original, 7);

    // One full in-game year.
    let year = TICKS_PER_DAY * DAYS_PER_YEAR;
    for _ in 0..year {
        sim.step(&raws);
    }

    assert_eq!(sim.clock.year(), 2, "a year should have passed");
    assert_eq!(
        sim.stats.deaths, 0,
        "no dwarf should die (harvested {}, cooked {}, brewed {})",
        sim.stats.crops_harvested, sim.stats.meals_cooked, sim.stats.drinks_brewed
    );
    assert!(sim.alive_dwarves() >= 7, "everyone alive, plus any migrants");

    // The food industry actually ran.
    assert!(sim.stats.crops_harvested > 0, "crops were harvested");
    assert!(sim.stats.meals_cooked > 0, "meals were cooked");
    assert!(sim.stats.drinks_brewed > 0, "drinks were brewed");

    // Emotional legibility: every original dwarf has thoughts a player can
    // read, and happiness stayed in range.
    for d in sim.dwarves.iter().take(original) {
        assert!(!d.thoughts.is_empty(), "{} has no thoughts", d.name);
        assert!((0.0..=100.0).contains(&d.happiness));
    }

    // Skills grew from the year's work.
    let total_xp: u32 = sim
        .dwarves
        .iter()
        .flat_map(|d| d.skills.values())
        .sum();
    assert!(total_xp > 0, "somebody should have learned something");
}

#[test]
fn without_food_dwarves_starve() {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(13);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, 13);
    let mut sim = Sim::new(map, &raws, rng, 5);
    // No supplies, no farm: the stakes are real.
    let half_year = TICKS_PER_DAY * DAYS_PER_YEAR / 2;
    for _ in 0..half_year {
        sim.step(&raws);
    }
    assert!(sim.stats.deaths > 0, "starvation must be lethal");
    // And the dying told us why.
    let starved = sim
        .dwarves
        .iter()
        .filter(|d| !d.alive)
        .all(|d| d.thoughts.iter().any(|(_, t)| {
            matches!(t, dk_agents::ThoughtKind::Starving | dk_agents::ThoughtKind::Dehydrated)
        }));
    assert!(starved, "dead dwarves should have starvation/dehydration thoughts");
}
