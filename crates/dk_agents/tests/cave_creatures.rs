//! Cave creatures: hostile beasts that lurk in the cavern and menace a fort
//! that digs into the deep.

mod common;

use dk_agents::{Faction, Sim};
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
fn cave_creatures_lurk_on_the_cavern_floor() {
    let (mut sim, raws) = fort(77);
    let z = sim.dwarves[0].pos.z;
    for x in 0..14 {
        for y in 0..14 {
            let p = Pos::new(x, y, z);
            if sim.map.walkable(p) {
                sim.cavern_floors.insert(p);
            }
        }
    }
    let before = sim.dwarves.len();
    sim.populate_caverns(3, &raws);
    let spawned: Vec<usize> = (before..sim.dwarves.len()).collect();
    assert_eq!(spawned.len(), 3, "three cave creatures spawned");
    for &i in &spawned {
        assert_eq!(sim.dwarves[i].faction, Faction::Hostile, "cave creatures are hostile");
        assert!(sim.dwarves[i].beast, "and are beasts (cannot dodge, but tough)");
        assert!(sim.dwarves[i].alive, "and alive");
        assert!(sim.cavern_floors.contains(&sim.dwarves[i].pos), "spawned on the cavern floor");
        assert!(sim.dwarves[i].name.starts_with("a "), "named for its kind: {}", sim.dwarves[i].name);
    }
    // Deterministic.
    let (mut a, ra) = fort(77);
    let (mut b, rb) = fort(77);
    let z2 = a.dwarves[0].pos.z;
    for x in 0..14 {
        for y in 0..14 {
            let p = Pos::new(x, y, z2);
            if a.map.walkable(p) {
                a.cavern_floors.insert(p);
                b.cavern_floors.insert(p);
            }
        }
    }
    a.populate_caverns(3, &ra);
    b.populate_caverns(3, &rb);
    let names = |s: &Sim| s.dwarves.iter().map(|d| d.name.clone()).collect::<Vec<_>>();
    assert_eq!(names(&a), names(&b), "spawning is deterministic");
}
