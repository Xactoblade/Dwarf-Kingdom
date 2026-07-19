//! Aquifers flood a fort: an opened tile in the wet layer weeps water without
//! end until it is walled off.

mod common;

use dk_agents::Sim;
use dk_world::path::Pos;

fn fort(seed: u64) -> (Sim, dk_raws::Raws) {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, seed);
    let mut sim = Sim::new(map, &raws, rng, 4);
    sim.invasions = false;
    (sim, raws)
}

#[test]
fn an_opened_aquifer_tile_floods() {
    // The mine hook flags an aquifer tile and inserts a spring when it is dug;
    // here we exercise the flood that follows from that spring.
    let (mut sim, raws) = fort(4242);
    let z = sim.dwarves[0].pos.z;
    // A dry, open, unoccupied floor tile.
    let p = (0..sim.map.width as i32)
        .flat_map(|x| (0..sim.map.height as i32).map(move |y| Pos::new(x, y, z)))
        .find(|&q| {
            sim.map.walkable(q)
                && sim.map.water_at(q) == 0
                && !sim.dwarves.iter().any(|d| d.pos == q)
        })
        .expect("an open dry tile exists");

    // Open the aquifer: the spring the mine hook would create.
    sim.water.springs.insert(p);
    sim.water.wake(p);
    for _ in 0..200 {
        sim.step(&raws);
    }
    assert!(sim.map.water_at(p) > 0, "the breached aquifer floods the opened tile");
}
