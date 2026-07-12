//! Animal husbandry exit tests, run headlessly: pastured livestock breed
//! and grow the herd, butchering a marked beast yields meat, and a full
//! pasture stops multiplying.

mod common;

use dk_agents::{AnimalKind, ItemKind, Sim, HERD_CAP};
use dk_core::{DAYS_PER_YEAR, TICKS_PER_DAY};
use dk_world::path::Pos;

fn pasture_fort(seed: u64) -> (Sim, dk_raws::Raws, dk_agents::Rect) {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, seed);
    let mut sim = Sim::new(map, &raws, rng, 5);
    sim.invasions = false;
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    sim.place_flat_stockpiles(cx, cy, 18);
    // A pasture on flat ground.
    let (pa, pb) = sim.find_flat_patch(cx, cy).expect("pasture site");
    // widen it a touch by adding a second adjacent flat patch worth of area
    sim.add_pasture(pa, pb);
    let rect = sim.pastures[0].clone();
    (sim, raws, rect)
}

#[test]
fn pastured_livestock_breed() {
    let (mut sim, raws, rect) = pasture_fort(1101);
    // Two adult cows in the pasture.
    let c = rect.center();
    sim.add_animal(AnimalKind::Cow, c, true);
    sim.add_animal(AnimalKind::Cow, Pos::new(c.x + 1, c.y, c.z), true);
    let start = sim.alive_animals();
    assert_eq!(start, 2);

    // Over a season, the pair should produce at least one calf.
    let horizon = TICKS_PER_DAY * (DAYS_PER_YEAR / 2);
    for _ in 0..horizon {
        sim.step(&raws);
    }
    assert!(
        sim.alive_animals() > start,
        "a breeding pair should grow the herd (had {start}, now {})",
        sim.alive_animals()
    );
    assert!(sim.log.iter().any(|(_, m)| m.contains("is born")));
}

#[test]
fn one_conception_yields_exactly_one_calf() {
    // Regression: the mate must not also give birth (double-birth bug).
    let (mut sim, raws, rect) = pasture_fort(1105);
    let c = rect.center();
    sim.add_animal(AnimalKind::Cow, c, true);
    sim.add_animal(AnimalKind::Cow, Pos::new(c.x + 1, c.y, c.z), true);

    // Run just past one gestation (20 days) plus slack, but stop before a
    // second conception could complete, and count births from the log.
    let horizon = TICKS_PER_DAY * 26;
    for _ in 0..horizon {
        sim.step(&raws);
    }
    let births = sim.log.iter().filter(|(_, m)| m.contains("is born")).count();
    assert_eq!(births, 1, "one conception must produce exactly one calf, not two");
    assert_eq!(sim.alive_animals(), 3, "two adults plus one calf");
}

#[test]
fn a_lone_animal_does_not_breed() {
    let (mut sim, raws, rect) = pasture_fort(1102);
    sim.add_animal(AnimalKind::Cow, rect.center(), true);
    for _ in 0..(TICKS_PER_DAY * 60) {
        sim.step(&raws);
    }
    assert_eq!(sim.alive_animals(), 1, "one animal cannot breed alone");
}

#[test]
fn butchering_a_marked_beast_yields_meat() {
    let (mut sim, raws, rect) = pasture_fort(1103);
    let c = rect.center();
    let cow = sim.add_animal(AnimalKind::Cow, c, true);
    let meals_before = sim.count_kind(ItemKind::Meal);

    // The herder marks it; an idle dwarf should carry out the slaughter.
    assert_eq!(sim.mark_nearest_animal(c), Some(cow));
    let mut done = false;
    for _ in 0..20_000 {
        sim.step(&raws);
        if sim.stats.animals_butchered > 0 {
            done = true;
            break;
        }
    }
    assert!(done, "a marked animal should be butchered");
    assert!(!sim.animals[cow].alive, "the cow is gone");
    assert!(
        sim.count_kind(ItemKind::Meal) > meals_before,
        "butchering fills the larder"
    );
    assert!(sim.log.iter().any(|(_, m)| m.contains("butchered")));
}

#[test]
fn a_full_pasture_stops_breeding() {
    let (mut sim, raws, rect) = pasture_fort(1104);
    let c = rect.center();
    // Fill the pasture to the cap with adults.
    for k in 0..HERD_CAP {
        let p = Pos::new(c.x + (k as i32 % 3) - 1, c.y + (k as i32 / 3) - 1, c.z);
        sim.add_animal(AnimalKind::Sheep, p, true);
    }
    assert_eq!(sim.alive_animals(), HERD_CAP);
    for _ in 0..(TICKS_PER_DAY * 40) {
        sim.step(&raws);
    }
    assert!(
        sim.alive_animals() <= HERD_CAP,
        "a capped herd must not overrun the pasture (now {})",
        sim.alive_animals()
    );
}
