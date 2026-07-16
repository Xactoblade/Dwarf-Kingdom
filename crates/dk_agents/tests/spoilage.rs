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
fn food_bought_from_a_caravan_is_as_fresh_as_the_day_it_was_bought() {
    // A caravan's wagon keeps its own clock. Stamp a purchase with the wagon's
    // reckoning and the meal is born a month old, to rot at the very next dawn
    // — the fort punished with the stench of rot for going shopping.
    let (mut sim, raws) = fort(9209);
    sim.trade_partner = Some("the Amber Banners".to_string());
    // The FIRST caravan comes before a shelf life has elapsed, so it cannot
    // spring the trap. Wait for one that arrives after — the second, in the
    // fort's second season.
    let mut arrived = false;
    for _ in 0..dk_core::TICKS_PER_DAY * dk_core::DAYS_PER_SEASON * 4 {
        // This fort embarked with nothing; keep them alive long enough to shop
        // (no living dwarves, no caravan).
        for d in &mut sim.dwarves {
            d.hunger = 0.0;
            d.thirst = 0.0;
        }
        sim.step(&raws);
        if sim.caravan.is_some() && sim.clock.tick > SHELF_LIFE_DAYS * TICKS_PER_DAY {
            arrived = true;
            break;
        }
    }
    assert!(
        arrived,
        "a caravan arriving after a shelf life has elapsed is needed — that is the trap"
    );

    // Put a meal on the wagon, stamped the way `maybe_caravan` stamps its
    // goods — `made_at: 0`, the wagon's own reckoning. (A wagon's contents are
    // random, so relying on it to carry food would make this test pass by
    // silently skipping.)
    let here = sim.dwarves[0].pos;
    sim.caravan.as_mut().unwrap().goods.push(dk_agents::Item {
        kind: ItemKind::Meal,
        stuff: 0,
        name: None,
        pos: here,
        state: ItemState::OnGround,
        reserved_by: None,
        consumed: false,
        quality: 0,
        made_at: 0,
    });
    let want = sim.caravan.as_ref().unwrap().goods.len() - 1;

    // Pay for it with rock.
    for _ in 0..40 {
        sim.debug_spawn_item(ItemKind::Boulder, 0, here);
    }
    let offer: Vec<usize> = sim
        .items
        .iter()
        .enumerate()
        .filter(|(_, it)| it.active() && it.kind == ItemKind::Boulder && it.reserved_by.is_none())
        .map(|(i, _)| i)
        .take(40)
        .collect();
    sim.execute_trade(&offer, &[want], &raws)
        .expect("the merchants take rock for a meal");
    let bought = sim.items.len() - 1;
    assert_eq!(sim.items[bought].kind, ItemKind::Meal, "we bought the meal");

    // One day later it must still be there.
    for _ in 0..TICKS_PER_DAY + 2 {
        sim.step(&raws);
    }
    assert!(
        sim.items[bought].active(),
        "a meal bought today does not rot tomorrow"
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
