//! Construction exit tests, run headlessly: a dwarf hauls a boulder to a
//! planned tile and raises a wall there — turning open floor into solid stone
//! the fort can wall itself in with.

mod common;

use dk_agents::Sim;
use dk_world::path::Pos;
use dk_world::TileShape;

fn build_fort(seed: u64) -> (Sim, dk_raws::Raws) {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, seed);
    let mut sim = Sim::new(map, &raws, rng, 4);
    sim.invasions = false;
    (sim, raws)
}

#[test]
fn only_open_floor_can_be_walled() {
    let (mut sim, _raws) = build_fort(5501);
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    let wz = sim.map.walk_surface_z(cx as usize, cy as usize).unwrap() as i32;
    // The open surface floor is a valid site.
    let floor = Pos::new(cx, cy, wz);
    assert!(sim.designate_construction(floor), "floor is buildable");
    // Not twice.
    assert!(!sim.designate_construction(floor), "no double-planning");
    // Solid rock below is not a construction site (it's already a wall).
    let rock = Pos::new(cx, cy, (wz - 4).max(1));
    assert!(!sim.designate_construction(rock), "solid rock isn't a build site");
}

#[test]
fn a_mason_raises_a_wall_from_a_boulder() {
    let (mut sim, raws) = build_fort(5502);
    sim.add_embark_supplies(&raws); // food for the workers
    // Boulders on a walkable tile for the builder to fetch.
    let sp = sim.dwarves[0].pos;
    for _ in 0..4 {
        sim.debug_spawn_boulder(0, sp);
    }
    // Plan a wall on a nearby open floor tile (not where anyone stands).
    let candidate = Pos::new(sp.x + 2, sp.y, sp.z);
    let site = if sim.map.walkable(candidate) {
        candidate
    } else {
        Pos::new(sp.x, sp.y + 2, sp.z)
    };
    assert!(sim.map.walkable(site), "the site is open floor");
    assert!(sim.designate_construction(site), "the wall is planned");
    assert!(sim.map.tile_at(site).unwrap().shape != TileShape::Solid);

    let mut raised = false;
    for _ in 0..30_000 {
        sim.step(&raws);
        if sim.map.tile_at(site).unwrap().shape == TileShape::Solid {
            raised = true;
            break;
        }
    }
    assert!(raised, "the mason should raise the wall");
    assert!(sim.map.tile_at(site).unwrap().is_solid(), "the tile is now stone");
    assert!(
        !sim.constructions.contains_key(&site),
        "the plan is cleared once built"
    );
    assert!(
        sim.log.iter().any(|(_, m)| m.contains("raises a wall")),
        "the deed is recorded"
    );
}

#[test]
fn adjacent_walls_and_walls_over_boulders_still_raise() {
    // Two regressions from the construction review: two adjacent plans must
    // not deadlock (each builder trying to stand on the other's site), and a
    // wall planned on a tile that holds a boulder must not livelock.
    let (mut sim, raws) = build_fort(5503);
    sim.add_embark_supplies(&raws);
    let sp = sim.dwarves[0].pos;
    for _ in 0..8 {
        sim.debug_spawn_boulder(0, sp);
    }
    // A guaranteed flat patch gives same-z, adjacent, walkable tiles.
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    let (pa, pb) = sim.find_flat_patch(cx, cy).expect("a flat patch");
    assert!(pb.x - pa.x >= 2 && pb.y - pa.y >= 2, "patch is at least 3x3");
    let occupied = |sim: &Sim, p: Pos| sim.dwarves.iter().any(|d| d.alive && d.pos == p);

    // Two horizontally-adjacent INTERIOR sites (each keeps free neighbours to
    // stand on): the deadlock case. Plus a corner site with a boulder resting
    // on it: the livelock case.
    let a = Pos::new(pa.x + 1, pa.y + 1, pa.z);
    let b = Pos::new(pa.x + 2, pa.y + 1, pa.z);
    let d = Pos::new(pb.x, pb.y, pa.z);
    let mut planned = Vec::new();
    for &site in &[a, b] {
        if sim.map.walkable(site) && !occupied(&sim, site) && sim.designate_construction(site) {
            planned.push(site);
        }
    }
    assert_eq!(planned.len(), 2, "two adjacent interior walls planned");
    if sim.map.walkable(d) && !occupied(&sim, d) && d.manhattan(a) > 2 && d.manhattan(b) > 2 {
        sim.debug_spawn_boulder(0, d); // a wall planned over a boulder
        if sim.designate_construction(d) {
            planned.push(d);
        }
    }

    for _ in 0..60_000 {
        sim.step(&raws);
        if planned.iter().all(|p| sim.map.tile_at(*p).unwrap().is_solid()) {
            break;
        }
    }
    for p in &planned {
        assert!(
            sim.map.tile_at(*p).unwrap().is_solid(),
            "every planned wall (incl. adjacent / over-a-boulder) is raised, no deadlock"
        );
    }
}
