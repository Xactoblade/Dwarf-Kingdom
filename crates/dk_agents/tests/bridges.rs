//! Drawbridges (Mechanisms slice S2): a span of floor that raises to an
//! impassable barrier and lowers back to walkable floor, toggled by a linked
//! lever through the generalized trigger->device dispatch (S1).

mod common;

use dk_agents::Sim;
use dk_world::path::Pos;
use dk_world::{Map, Tile};

/// Flat arena: solid bedrock at z0, walkable floor at z1.
fn flat_map(w: usize, h: usize) -> Map {
    let mut m = Map::new_air(w, h, 4, 0);
    for y in 0..h {
        for x in 0..w {
            m.set(x, y, 0, Tile::solid(2)); // granite
            m.set(x, y, 1, Tile::floor(2));
        }
    }
    m
}

#[test]
fn a_drawbridge_raises_and_lowers_a_barrier() {
    let raws = common::test_raws();
    let m = flat_map(20, 20);
    let mut sim = Sim::new(m, &raws, dk_core::rng_from_seed(1), 1);
    sim.water.springs.clear();
    sim.rebuild_caches();

    // A 3-tile span across a corridor. Lowered = plain walkable floor.
    let span: Vec<Pos> = (8..=10).map(|x| Pos::new(x, 5, 1)).collect();
    for &p in &span {
        assert!(sim.map.walkable(p));
    }
    let anchor = sim.add_bridge(span.clone()).expect("the bridge is placed");
    assert_eq!(anchor, Pos::new(8, 5, 1), "the anchor is the min-corner tile");
    assert!(!sim.bridges[&anchor].raised, "a fresh bridge is lowered");
    for &p in &span {
        assert!(sim.map.walkable(p), "a lowered bridge is walkable floor");
    }

    // A lever links to the bridge (nearest linkable device), and pulling it
    // raises the whole span into an impassable barrier.
    let lever = Pos::new(2, 2, 1);
    assert_eq!(sim.add_lever(lever), Some(anchor), "the lever links the bridge anchor");
    assert!(sim.pull_lever(lever));
    assert!(sim.bridges[&anchor].raised, "the bridge is raised");
    for &p in &span {
        assert!(!sim.map.walkable(p), "a raised span blocks the way");
    }

    // Pull again: it lowers back to walkable floor.
    assert!(sim.pull_lever(lever));
    assert!(!sim.bridges[&anchor].raised);
    for &p in &span {
        assert!(sim.map.walkable(p), "a lowered bridge is walkable again");
    }
}

#[test]
fn a_bridge_only_covers_free_plain_floor() {
    let raws = common::test_raws();
    let mut m = flat_map(16, 16);
    m.set(5, 5, 1, Tile::solid(2)); // a wall in the way
    m.set_water(Pos::new(6, 5, 1), 3); // standing water on another tile
    let mut sim = Sim::new(m, &raws, dk_core::rng_from_seed(2), 1);
    sim.water.springs.clear();
    sim.rebuild_caches();

    assert!(sim.add_bridge(vec![Pos::new(5, 5, 1)]).is_none(), "not over a wall");
    assert!(sim.add_bridge(vec![Pos::new(6, 5, 1)]).is_none(), "not over standing water");
    assert!(sim.add_bridge(vec![]).is_none(), "not an empty span");

    // A good span works, and a second bridge can't overlap it.
    let good: Vec<Pos> = (1..=3).map(|x| Pos::new(x, 1, 1)).collect();
    assert!(sim.add_bridge(good.clone()).is_some());
    assert!(sim.add_bridge(vec![Pos::new(2, 1, 1)]).is_none(), "no overlapping bridge");
}
