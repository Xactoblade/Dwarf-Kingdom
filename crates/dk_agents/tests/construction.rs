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
