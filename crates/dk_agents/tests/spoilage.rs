//! Food spoils (BLUEPRINT.md §2.4, item decay): what the fort does not put
//! away, it loses.
//!
//! The rules are Dwarf Fortress's, and the important one is a myth-killer: a
//! barrel does NOT preserve food. "It does not matter if the food is in a
//! container; a barrel or large pot full of meat left in a corridor will rot."
//! What keeps food is being in a STOCKPILE — a cask helps only because the
//! cask stands in one.

mod common;

use dk_agents::{ItemKind, ItemState, Sim, StockCategory, StockFilter, ThoughtKind, SHELF_LIFE_DAYS};
use dk_core::TICKS_PER_DAY;
use dk_world::path::Pos;

fn fort(seed: u64) -> (Sim, dk_raws::Raws) {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, seed);
    let mut sim = Sim::new(map, &raws, rng, 2);
    sim.invasions = false;
    (sim, raws)
}

/// Let a shelf life and a day pass, without dwarves eating the evidence.
fn wait_out_the_shelf_life(sim: &mut Sim, raws: &dk_raws::Raws) {
    for _ in 0..(SHELF_LIFE_DAYS + 2) * TICKS_PER_DAY {
        // Keep them fed so nobody eats the specimen.
        for d in &mut sim.dwarves {
            d.hunger = 0.0;
            d.thirst = 0.0;
        }
        sim.step(raws);
    }
}

#[test]
fn food_left_lying_about_goes_bad() {
    let (mut sim, raws) = fort(9201);
    let far = Pos::new(2, 2, sim.map.walk_surface_z(2, 2).unwrap() as i32);
    sim.debug_spawn_item(ItemKind::Meal, 0, far);
    let meal = sim.items.len() - 1;
    assert!(sim.items[meal].active());

    wait_out_the_shelf_life(&mut sim, &raws);
    assert!(!sim.items[meal].active(), "a meal left on the floor rots away");
    assert!(sim.stats.food_spoiled > 0);
}

#[test]
fn food_in_a_stockpile_keeps_forever() {
    let (mut sim, raws) = fort(9202);
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    sim.place_flat_stockpiles(cx, cy, 24);
    let cell = sim.stockpiles[0].cells().next().unwrap();
    sim.debug_spawn_item(ItemKind::Meal, 0, cell);
    let meal = sim.items.len() - 1;
    sim.items[meal].state = ItemState::Stored { stockpile: 0 };

    wait_out_the_shelf_life(&mut sim, &raws);
    assert!(sim.items[meal].active(), "a pile keeps food indefinitely");
}

#[test]
fn a_barrel_in_a_corridor_preserves_nothing() {
    // The myth, killed. A cask standing out in the open is not a pantry: the
    // meat in it rots exactly as it would on the floor beside it.
    let (mut sim, raws) = fort(9203);
    let corridor = Pos::new(2, 2, sim.map.walk_surface_z(2, 2).unwrap() as i32);
    sim.debug_spawn_item(ItemKind::Barrel, 0, corridor);
    let barrel = sim.items.len() - 1;
    // The cask is NOT in a stockpile — it's just standing there.
    assert!(sim.stockpile_at(corridor).is_none());
    sim.debug_spawn_item(ItemKind::Meal, 0, corridor);
    let meal = sim.items.len() - 1;
    sim.items[meal].state = ItemState::Inside { container: barrel };

    wait_out_the_shelf_life(&mut sim, &raws);
    assert!(
        !sim.items[meal].active(),
        "a barrel full of meat left in a corridor rots — the cask is no pantry"
    );
}

#[test]
fn a_barrel_standing_in_a_stockpile_keeps_its_food() {
    // ...and the same cask, in a pile, keeps everything. Not because it is a
    // cask — because it is in a pile.
    let (mut sim, raws) = fort(9204);
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    sim.place_flat_stockpiles(cx, cy, 24);
    let cell = sim.stockpiles[0].cells().next().unwrap();
    sim.debug_spawn_item(ItemKind::Barrel, 0, cell);
    let barrel = sim.items.len() - 1;
    sim.items[barrel].state = ItemState::Stored { stockpile: 0 };
    sim.debug_spawn_item(ItemKind::Meal, 0, cell);
    let meal = sim.items.len() - 1;
    sim.items[meal].state = ItemState::Inside { container: barrel };

    wait_out_the_shelf_life(&mut sim, &raws);
    assert!(sim.items[meal].active(), "the larder's cask keeps the larder's meals");
}

#[test]
fn drink_and_seed_never_spoil() {
    // Booze keeping forever is the whole reason a fort brews its harvest
    // rather than eating it.
    let (mut sim, raws) = fort(9205);
    let far = Pos::new(2, 2, sim.map.walk_surface_z(2, 2).unwrap() as i32);
    sim.debug_spawn_item(ItemKind::Drink, 0, far);
    let drink = sim.items.len() - 1;
    sim.debug_spawn_item(ItemKind::Seed, 0, far);
    let seed = sim.items.len() - 1;

    wait_out_the_shelf_life(&mut sim, &raws);
    assert!(sim.items[drink].active(), "booze keeps, out in the rain or not");
    assert!(sim.items[seed].active(), "and so do seeds");
}

#[test]
fn a_rotting_meal_stinks_but_a_withered_crop_does_not() {
    // Two fates: meals rot and foul the air; plants merely wither.
    let (mut sim, raws) = fort(9206);
    let spot = sim.dwarves[0].pos;
    sim.debug_spawn_item(ItemKind::Meal, 0, spot);
    // A month is a long time to loiter: keep the dwarf by the heap so they are
    // there to smell it turn (they would otherwise have wandered off).
    for _ in 0..(SHELF_LIFE_DAYS + 2) * TICKS_PER_DAY {
        for d in &mut sim.dwarves {
            d.hunger = 0.0;
            d.thirst = 0.0;
        }
        sim.dwarves[0].pos = spot;
        sim.step(&raws);
    }
    assert!(
        sim.dwarves[0].thoughts.iter().any(|t| t.1 == ThoughtKind::SmelledRot),
        "a dwarf beside rotting food gags on it"
    );
    assert!(sim.log.iter().any(|(_, m)| m.contains("fouls the air")), "and the fort is told");
    assert!(ThoughtKind::SmelledRot.delta() < 0.0, "and resents it");

    let (mut sim2, raws2) = fort(9207);
    let spot2 = sim2.dwarves[0].pos;
    sim2.debug_spawn_item(ItemKind::Crop, 0, spot2);
    let crop = sim2.items.len() - 1;
    wait_out_the_shelf_life(&mut sim2, &raws2);
    assert!(!sim2.items[crop].active(), "the crop withered away");
    assert!(
        !sim2.dwarves[0].thoughts.iter().any(|t| t.1 == ThoughtKind::SmelledRot),
        "but a shrivelled plant ruins nobody's day"
    );
}

#[test]
fn a_food_pile_is_what_saves_the_larder() {
    // The player-facing lesson: tell a pile it is for food, and the food lives.
    let (mut sim, raws) = fort(9208);
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    let (a, b) = sim.find_flat_patch(cx, cy).expect("pile ground");
    sim.add_filtered_stockpile(a, b, StockFilter::only(&[StockCategory::Food]));

    let sp = sim.dwarves[0].pos;
    for _ in 0..4 {
        sim.debug_spawn_item(ItemKind::Meal, 0, sp);
    }
    // Haulers put them away long before the shelf life runs out.
    wait_out_the_shelf_life(&mut sim, &raws);
    assert_eq!(
        sim.count_kind(ItemKind::Meal),
        4,
        "food the fort put away is food the fort still has"
    );
}
