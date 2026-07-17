//! Line of sight for ranged combat: a bolt (or an eye) travels an
//! unobstructed straight line, and a wall or shut gate between two points
//! blocks it. Same-level only — a bolt does not arc between z-layers.

use dk_world::path::Pos;
use dk_world::{Map, Tile, TileShape};

fn open_floor(w: usize, h: usize) -> Map {
    let mut m = Map::new_air(w, h, 3, 0);
    for y in 0..h {
        for x in 0..w {
            m.set(x, y, 1, Tile::floor(1));
        }
    }
    m
}

#[test]
fn an_open_field_has_a_clear_shot() {
    let m = open_floor(20, 20);
    assert!(m.clear_shot(Pos::new(2, 2, 1), Pos::new(15, 9, 1)));
    // And symmetric — a shot back the other way is just as clear.
    assert!(m.clear_shot(Pos::new(15, 9, 1), Pos::new(2, 2, 1)));
}

#[test]
fn a_wall_between_blocks_the_shot() {
    let mut m = open_floor(20, 20);
    // Drop a wall column midway between shooter (2,10) and target (17,10).
    for y in 0..20 {
        m.set(9, y, 1, Tile::solid(1));
    }
    assert!(!m.clear_shot(Pos::new(2, 10, 1), Pos::new(17, 10, 1)));
}

#[test]
fn a_shut_gate_stops_a_bolt() {
    let mut m = open_floor(20, 20);
    let mut gate = Tile::floor(1);
    gate.shape = TileShape::Gate;
    m.set_at(Pos::new(9, 5, 1), gate);
    assert!(!m.clear_shot(Pos::new(9, 2, 1), Pos::new(9, 9, 1)));
}

#[test]
fn a_bolt_does_not_travel_between_levels() {
    let m = open_floor(20, 20);
    assert!(!m.clear_shot(Pos::new(2, 2, 1), Pos::new(2, 2, 2)));
}

#[test]
fn the_endpoints_themselves_do_not_block() {
    // Shooter and target stand on solid-adjacent tiles; a wall exactly at the
    // target must not count as blocking the shot that reaches it.
    let mut m = open_floor(20, 20);
    m.set(2, 2, 1, Tile::solid(1)); // "shooter" tile made solid
    m.set(9, 2, 1, Tile::solid(1)); // "target" tile made solid
    // Everything between is open, so the shot connects the two endpoints.
    assert!(m.clear_shot(Pos::new(2, 2, 1), Pos::new(9, 2, 1)));
}
