//! Wild-foraging tests, run headlessly: a berry shrub standing on the surface
//! can be designated for gathering, and a forager walks to it, picks it clean,
//! and leaves a heap of edible berries where it stood. Berries are eaten
//! straight, with no kitchen. A tended patch reseeds itself over time. A map
//! with no shrubs is untouched — the whole system is gated on shrubs existing
//! (placed only at embark), so headless forts stay byte-identical.

mod common;

use dk_agents::{DesignationKind, ItemKind, Sim};

fn forage_fort(seed: u64, n: usize) -> (Sim, dk_raws::Raws) {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, seed);
    let mut sim = Sim::new(map, &raws, rng, n);
    sim.invasions = false;
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    sim.add_embark_supplies(&raws);
    sim.place_flat_stockpiles(cx, cy, 18);
    (sim, raws)
}

#[test]
fn a_forager_gathers_a_shrub_for_berries() {
    let (mut sim, raws) = forage_fort(4401, 4);
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;

    // A berry shrub on a reachable flat tile, marked to be gathered.
    let (shrub, _) = sim.find_flat_patch(cx, cy).expect("a flat tile for a shrub");
    sim.shrubs.insert(shrub);
    assert!(sim.shrub_at(shrub));
    assert_eq!(sim.designate_rect(DesignationKind::Gather, shrub, shrub), 1);
    assert_eq!(sim.count_kind(ItemKind::Berry), 0, "nothing gathered yet");

    let mut gathered = false;
    for _ in 0..12_000 {
        sim.step(&raws);
        if sim.stats.foraged > 0 {
            gathered = true;
            break;
        }
    }
    assert!(gathered, "a forager should gather the marked shrub");
    assert!(!sim.shrub_at(shrub), "the picked shrub is gone");
    assert!(sim.count_kind(ItemKind::Berry) > 0, "berries lie where the shrub stood");
}

#[test]
fn cancelling_a_gather_spares_the_shrub() {
    // Regression guard mirroring the chop-cancel fix: cancelling must actually
    // stop the forager, leaving the shrub standing and no berries picked.
    let (mut sim, raws) = forage_fort(4404, 4);
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    let (shrub, _) = sim.find_flat_patch(cx, cy).expect("a flat tile for a shrub");
    sim.shrubs.insert(shrub);
    sim.designate_rect(DesignationKind::Gather, shrub, shrub);

    // Let a forager start walking, then cancel.
    for _ in 0..40 {
        sim.step(&raws);
    }
    sim.cancel_rect(shrub, shrub);
    for _ in 0..2_000 {
        sim.step(&raws);
    }
    assert!(sim.shrub_at(shrub), "the cancelled shrub still stands");
    assert_eq!(sim.stats.foraged, 0, "nothing was foraged");
    assert_eq!(sim.count_kind(ItemKind::Berry), 0, "no berries picked");
}

#[test]
fn berries_are_eaten_straight_no_kitchen() {
    // A hungry dwarf on a heap of berries eats them on the spot — berries need
    // no cooking, so hunger is satisfied with no kitchen and no other food
    // about. (A bare fort with no embark meals, so berries are the only food.)
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(4402);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, 4402);
    let mut sim = Sim::new(map, &raws, rng, 2);
    sim.invasions = false;

    let dpos = sim.dwarves[0].pos;
    sim.dwarves[0].hunger = 95.0;
    sim.debug_spawn_item(ItemKind::Berry, 0, dpos);
    assert_eq!(sim.count_kind(ItemKind::Berry), 1);

    let mut ate = false;
    for _ in 0..4_000 {
        sim.step(&raws);
        if sim.count_kind(ItemKind::Berry) == 0 {
            ate = true;
            break;
        }
    }
    assert!(ate, "the hungry dwarf should eat the berries");
    assert!(sim.dwarves[0].hunger < 50.0, "eating the berries eased the hunger");
}

#[test]
fn a_fort_with_no_shrubs_never_forages() {
    let (mut sim, raws) = forage_fort(4403, 4);
    for _ in 0..2_000 {
        sim.step(&raws);
    }
    assert_eq!(sim.stats.foraged, 0, "no shrubs, no foraging");
    assert_eq!(sim.count_kind(ItemKind::Berry), 0, "and no berries appear");
}

#[test]
fn a_tended_berry_patch_reseeds_itself() {
    // Regrowth: planted shrubs spread to open neighbours over the days, up to
    // the patch's ceiling — so berries are a renewable food, not a one-time
    // strip.
    let (mut sim, raws) = forage_fort(4405, 4);
    sim.plant_shrubs(30);
    let initial = sim.shrubs.len();
    assert!(initial > 0, "the patch was planted");
    // Give the patch room to spread well beyond its planted size.
    sim.shrub_cap = initial + 60;

    for _ in 0..(dk_core::TICKS_PER_DAY as usize * 200) {
        sim.step(&raws);
    }
    assert!(
        sim.shrubs.len() > initial,
        "the patch reseeded itself: {} -> {}",
        initial,
        sim.shrubs.len()
    );
}
