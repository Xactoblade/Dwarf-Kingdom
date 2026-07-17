//! Vermin: the thing that eats a well-stored larder.
//!
//! Spoilage only punishes a fort that never built a stockpile — food in a pile
//! keeps forever, which is Dwarf Fortress's rule. Vermin are what DF puts on
//! the *tidy* fort, and they are the other half of why a barrel is worth a log:
//! a cask will not stop food ROTTING (only a stockpile does that), but vermin
//! "attempt to eat EXPOSED food", and a cask is a rat-proof box.
//!
//! Which vermin you get is the country's doing — evil ground breeds demon rats,
//! good ground the fluffy wambler, savage ground rhino lizards.
//!
//! Two numbers here are OURS, not DF's: the rate they creep in at and how much
//! they eat. The wiki documents neither anywhere.

mod common;

use dk_agents::{AnimalKind, ItemKind, ItemState, Sim, VerminKind};
use dk_world::path::Pos;

fn fort(seed: u64, kind: Option<VerminKind>) -> (Sim, dk_raws::Raws) {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, seed);
    let mut sim = Sim::new(map, &raws, rng, 2);
    sim.invasions = false;
    sim.vermin_kind = kind;
    (sim, raws)
}

#[test]
fn a_fort_with_no_vermin_in_its_country_is_never_troubled() {
    // The gate: `vermin_kind` is set only by the app at embark, so every
    // headless fort — and every fort on a glacier — is vermin-free and draws no
    // RNG here.
    let (mut sim, raws) = fort(9301, None);
    let sp = sim.dwarves[0].pos;
    for _ in 0..4 {
        sim.debug_spawn_item(ItemKind::Meal, 0, sp);
    }
    for _ in 0..dk_core::TICKS_PER_DAY * 4 {
        sim.step(&raws);
    }
    assert!(sim.vermin.is_empty(), "no vermin creep into a country that has none");
    assert_eq!(sim.stats.food_gnawed, 0, "and nothing eats the stores");
}

#[test]
fn vermin_creep_in_and_eat_food_left_exposed() {
    let (mut sim, raws) = fort(9302, Some(VerminKind::Rat));
    let sp = sim.dwarves[0].pos;
    for _ in 0..20 {
        sim.debug_spawn_item(ItemKind::Meal, 0, sp);
    }
    let mut gnawed = false;
    for _ in 0..dk_core::TICKS_PER_DAY * 8 {
        // Keep the dwarves fed: otherwise THEY eat the larder and the rats
        // never get a look in, which proves nothing either way.
        for d in &mut sim.dwarves {
            d.hunger = 0.0;
            d.thirst = 0.0;
        }
        sim.step(&raws);
        if sim.stats.food_gnawed > 0 {
            gnawed = true;
            break;
        }
    }
    assert!(gnawed, "rats find food left out and eat it");
    assert!(!sim.vermin.is_empty(), "and they are here, visibly");
}

#[test]
fn food_in_a_cask_is_food_a_rat_cannot_reach() {
    // The payoff. A cask is no pantry — it will not stop rot — but it is a
    // rat-proof box, and that is the other half of why a fort wants one.
    let (mut sim, raws) = fort(9303, Some(VerminKind::DemonRat));
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    sim.place_flat_stockpiles(cx, cy, 24);
    let cell = sim.stockpiles[0].cells().next().unwrap();
    sim.debug_spawn_item(ItemKind::Barrel, 0, cell);
    let barrel = sim.items.len() - 1;
    sim.items[barrel].state = ItemState::Stored { stockpile: 0 };
    for _ in 0..8 {
        sim.debug_spawn_item(ItemKind::Meal, 0, cell);
        let m = sim.items.len() - 1;
        sim.items[m].state = ItemState::Inside { container: barrel };
    }
    assert_eq!(sim.contents_of(barrel).len(), 8);

    for _ in 0..dk_core::TICKS_PER_DAY * 8 {
        sim.step(&raws);
    }
    assert_eq!(
        sim.contents_of(barrel).len(),
        8,
        "not even a demon rat gets into a closed cask"
    );
    assert_eq!(sim.stats.food_gnawed, 0);
}

#[test]
fn exposed_means_exposed() {
    let (mut sim, _raws) = fort(9304, Some(VerminKind::Rat));
    let sp = sim.dwarves[0].pos;
    // On the floor: exposed.
    sim.debug_spawn_item(ItemKind::Meal, 0, sp);
    let loose = sim.items.len() - 1;
    assert!(sim.food_exposed(loose), "food on the ground is exposed");

    // Stored loose in a pile: still exposed — a stockpile stops rot, not rats.
    sim.debug_spawn_item(ItemKind::Meal, 0, sp);
    let stored = sim.items.len() - 1;
    sim.items[stored].state = ItemState::Stored { stockpile: 0 };
    assert!(sim.food_exposed(stored), "a pile is no protection from vermin");

    // In a cask: not exposed.
    sim.debug_spawn_item(ItemKind::Barrel, 0, sp);
    let barrel = sim.items.len() - 1;
    sim.debug_spawn_item(ItemKind::Meal, 0, sp);
    let packed = sim.items.len() - 1;
    sim.items[packed].state = ItemState::Inside { container: barrel };
    assert!(!sim.food_exposed(packed), "a cask keeps the rats out");

    // Carried: in someone's hands, not on the floor.
    sim.debug_spawn_item(ItemKind::Meal, 0, sp);
    let held = sim.items.len() - 1;
    sim.items[held].state = ItemState::Carried { by: 0 };
    assert!(!sim.food_exposed(held));

    // And a cask that was destroyed protects nothing.
    sim.items[barrel].consumed = true;
    assert!(sim.food_exposed(packed), "a cask that is gone guards nothing");
}

#[test]
fn a_cat_kills_the_vermin() {
    let (mut sim, raws) = fort(9305, Some(VerminKind::Rat));
    let sp = sim.dwarves[0].pos;
    for _ in 0..6 {
        sim.debug_spawn_item(ItemKind::Meal, 0, sp);
    }
    // Let the rats arrive.
    for _ in 0..dk_core::TICKS_PER_DAY * 4 {
        sim.step(&raws);
        if !sim.vermin.is_empty() {
            break;
        }
    }
    assert!(!sim.vermin.is_empty(), "rats first");

    // Now a cat, right on top of one.
    let rat = sim.vermin[0].pos;
    sim.add_animal(AnimalKind::Cat, rat, true);
    for _ in 0..dk_core::TICKS_PER_DAY {
        sim.step(&raws);
        if sim.stats.vermin_slain > 0 {
            break;
        }
    }
    assert!(sim.stats.vermin_slain > 0, "the cat earns its keep");
}

#[test]
fn a_fort_with_cats_loses_less_than_one_without() {
    // The whole point of a cat, measured.
    let losses = |cats: usize| -> u32 {
        let (mut sim, raws) = fort(9306, Some(VerminKind::Rat));
        let sp = sim.dwarves[0].pos;
        for _ in 0..40 {
            sim.debug_spawn_item(ItemKind::Meal, 0, sp);
        }
        for i in 0..cats {
            sim.add_animal(AnimalKind::Cat, Pos::new(sp.x + i as i32, sp.y, sp.z), true);
        }
        for _ in 0..dk_core::TICKS_PER_DAY * 10 {
            sim.step(&raws);
        }
        sim.stats.food_gnawed
    };
    let without = losses(0);
    let with = losses(4);
    assert!(without > 0, "an uncatted fort loses food to rats ({without})");
    assert!(
        with < without,
        "a catted fort loses less ({with} with cats vs {without} without)"
    );
}

#[test]
fn the_seven_eaters_are_dwarf_fortresss_seven() {
    // DF has 131 vermin and exactly seven carry [VERMIN_EATER]. These are they,
    // with their PENETRATEPOWER — 1, 2, or 3, against a roll of 0-100.
    for (k, p) in [
        (VerminKind::DemonRat, 3),
        (VerminKind::Rat, 2),
        (VerminKind::Hamster, 2),
        (VerminKind::LargeRoach, 2),
        (VerminKind::RhinoLizard, 2),
        (VerminKind::Lizard, 1),
        (VerminKind::FluffyWambler, 1),
    ] {
        assert_eq!(k.penetrate_power(), p, "{}", k.name());
        assert!(!k.name().is_empty());
    }
    // The demon rat is the worst of them.
    assert!(
        VerminKind::DemonRat.penetrate_power() > VerminKind::Rat.penetrate_power(),
        "evil ground breeds a better burglar"
    );
}
