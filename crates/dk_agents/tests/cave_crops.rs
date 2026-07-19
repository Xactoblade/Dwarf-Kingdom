//! Cave mushrooms: the fort's food in the deep, sown across the cavern floor
//! and gathered like the surface berry shrubs.

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
fn cave_mushrooms_grow_on_the_cavern_floor_only() {
    let (mut sim, _raws) = fort(31);
    // Stand in for a cavern floor with a patch of walkable, dry ground.
    let z = sim.dwarves[0].pos.z;
    let mut floor = Vec::new();
    for x in 0..16 {
        for y in 0..16 {
            let p = Pos::new(x, y, z);
            if sim.map.walkable(p) && sim.map.water_at(p) == 0 {
                sim.cavern_floors.insert(p);
                floor.push(p);
            }
        }
    }
    assert!(floor.len() >= 30, "enough cavern floor to sow ({})", floor.len());
    assert_eq!(sim.shrubs.len(), 0, "no shrubs before sowing");

    sim.plant_cave_mushrooms(20);
    assert!(!sim.shrubs.is_empty(), "mushrooms grew in the cavern");
    // Every mushroom sits on the cavern floor — none stray to the surface.
    for p in sim.shrubs.iter() {
        assert!(sim.cavern_floors.contains(p), "a mushroom grew off the cavern floor");
        assert!(sim.shrub_at(*p), "and is a gatherable shrub");
    }
    // Deterministic.
    let (mut a, _) = fort(31);
    let (mut b, _) = fort(31);
    for s in &sim.cavern_floors {
        a.cavern_floors.insert(*s);
        b.cavern_floors.insert(*s);
    }
    a.plant_cave_mushrooms(20);
    b.plant_cave_mushrooms(20);
    assert_eq!(a.shrubs, b.shrubs, "sowing is deterministic");
}
