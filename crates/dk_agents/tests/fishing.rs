//! Fishing exit tests, run headlessly: a fishery over water gives dwarves
//! a renewable food source they work when the larder runs low.

mod common;

use dk_agents::{ItemKind, Sim};
use dk_world::path::Pos;
use dk_world::{Map, Tile};

/// A 24x24 arena: bedrock at z0, floor at z1, with a pond of water in the
/// middle so there are banks to fish from.
fn fishing_arena() -> (Sim, dk_raws::Raws) {
    let raws = common::test_raws();
    let mut m = Map::new_air(24, 24, 4, 0);
    for y in 0..24 {
        for x in 0..24 {
            m.set(x, y, 0, Tile::solid(2));
            m.set(x, y, 1, Tile::floor(2));
        }
    }
    // A 4x4 pond of standing water.
    for y in 10..14 {
        for x in 10..14 {
            m.set_water(Pos::new(x as i32, y as i32, 1), 4);
        }
    }
    let rng = dk_core::rng_from_seed(1501);
    let mut sim = Sim::new(m, &raws, rng, 4);
    sim.water.springs.clear();
    sim.invasions = false;
    sim.rebuild_caches();
    // Stockpiles so caught fish (meals) have somewhere to go.
    sim.place_flat_stockpiles(3, 3, 18);
    (sim, raws)
}

#[test]
fn dwarves_fish_from_a_fishery_when_food_is_low() {
    let (mut sim, raws) = fishing_arena();
    // A fishery covering the pond and its banks (x9..14, y9..14).
    sim.add_fishery(Pos::new(9, 9, 1), Pos::new(14, 14, 1));
    // Fort has no food, so fishing is wanted.
    assert_eq!(sim.count_kind(ItemKind::Meal), 0);

    let mut caught = false;
    for _ in 0..30_000 {
        sim.step(&raws);
        if sim.stats.fish_caught > 0 {
            caught = true;
            break;
        }
    }
    assert!(caught, "a fishery over water should yield fish");
    assert!(sim.count_kind(ItemKind::Meal) > 0, "fish are edible food");
    assert!(
        sim.log.is_empty() || sim.stats.fish_caught >= 1,
        "at least one catch landed"
    );
}

#[test]
fn a_fishery_needs_water() {
    let raws = common::test_raws();
    let mut m = Map::new_air(20, 20, 4, 0);
    for y in 0..20 {
        for x in 0..20 {
            m.set(x, y, 0, Tile::solid(2));
            m.set(x, y, 1, Tile::floor(2));
        }
    }
    let rng = dk_core::rng_from_seed(1502);
    let mut sim = Sim::new(m, &raws, rng, 3);
    sim.water.springs.clear();
    sim.invasions = false;
    sim.rebuild_caches();
    // A fishery on dry land — there is nothing to catch.
    sim.add_fishery(Pos::new(5, 5, 1), Pos::new(9, 9, 1));
    for _ in 0..5_000 {
        sim.step(&raws);
    }
    assert_eq!(sim.stats.fish_caught, 0, "no water, no fish");
}
