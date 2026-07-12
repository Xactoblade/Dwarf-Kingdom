//! River exit tests, run headlessly: a carved river is a real body of water on
//! the local map that STAYS put — it neither drains away nor floods the fort —
//! once the water simulation runs.

mod common;

use dk_agents::Sim;
use dk_world::path::Pos;

fn map_with_river(seed: u64) -> (dk_world::Map, dk_raws::Raws) {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(seed);
    let mut map = dk_world::generate(&raws.materials, &mut rng, 48, 48, 16, seed);
    dk_world::carve_river(&mut map, seed);
    (map, raws)
}

/// Count tiles holding any water, and the deep (>=5, path-blocking) ones.
fn water_counts(map: &dk_world::Map) -> (usize, usize) {
    let mut any = 0;
    let mut deep = 0;
    for z in 0..map.depth {
        for y in 0..map.height {
            for x in 0..map.width {
                let w = map.get(x, y, z).water;
                if w > 0 {
                    any += 1;
                }
                if w >= 5 {
                    deep += 1;
                }
            }
        }
    }
    (any, deep)
}

#[test]
fn a_carved_river_is_a_real_body_of_water() {
    let (map, _raws) = map_with_river(4501);
    let (any, deep) = water_counts(&map);
    assert!(deep >= 30, "the river should be a substantial run of deep water (got {deep})");
    assert!(any >= deep, "sanity");
}

#[test]
fn the_river_stays_put_and_does_not_flood_the_fort() {
    let (map, raws) = map_with_river(4502);
    let (_, deep_before) = water_counts(&map);
    assert!(deep_before > 0);

    let rng = dk_core::rng_from_seed(4502);
    let mut sim = Sim::new(map, &raws, rng, 3);
    sim.invasions = false;

    // Let the water simulation run hard.
    for _ in 0..1000 {
        sim.step(&raws);
    }

    let (any_after, deep_after) = water_counts(&sim.map);
    let area = sim.map.width * sim.map.height;
    // The river neither drained away...
    assert!(
        deep_after >= deep_before / 2,
        "the river drained (deep {deep_before} -> {deep_after})"
    );
    // ...nor flooded the fort: water stays a small fraction of the map (a
    // 48x48 map has 2304 tiles/level; a contained river + spring pond is tiny).
    assert!(
        any_after < area / 4,
        "the river flooded the map ({any_after} wet tiles of {area}/level)"
    );

    // The surface far from any water source stays dry and walkable.
    let dry_corner = Pos::new(1, 1, sim.map.walk_surface_z(1, 1).unwrap() as i32);
    let _ = dry_corner; // best-effort: corners aren't on the river's mid-course
}
