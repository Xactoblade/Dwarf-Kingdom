//! Blood spatter: wounds drip and deaths pool blood on the ground, which dries
//! and fades away over the following day.

mod common;

use dk_agents::{Sim, BLOOD_DRY, BLOOD_MAX};
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
fn blood_pools_where_spilled_and_splashes_around() {
    let (mut sim, _raws) = fort(88);
    let p = sim.dwarves[0].pos;
    assert!(sim.blood.is_empty(), "no blood before a wound");
    sim.spatter_blood(p, BLOOD_MAX);
    assert_eq!(sim.blood.get(&p).copied(), Some(BLOOD_MAX), "a fresh pool where it fell");
    // At least one walkable neighbour caught a splash.
    let splashed = [(1, 0), (-1, 0), (0, 1), (0, -1)]
        .iter()
        .filter(|(dx, dy)| sim.blood.contains_key(&Pos::new(p.x + dx, p.y + dy, p.z)))
        .count();
    assert!(splashed > 0, "blood splashes onto the ground around it");
}

#[test]
fn blood_dries_and_fades_away() {
    let (mut sim, _raws) = fort(89);
    let p = sim.dwarves[0].pos;
    sim.spatter_blood(p, BLOOD_MAX);
    // One drying pass thins it; enough passes clear it entirely.
    sim.dry_blood();
    assert!(sim.blood.get(&p).copied().unwrap_or(0) < BLOOD_MAX, "blood dries a little each pass");
    for _ in 0..(BLOOD_MAX / BLOOD_DRY + 2) {
        sim.dry_blood();
    }
    assert!(!sim.blood.contains_key(&p), "and eventually fades away entirely");
}

#[test]
fn a_death_leaves_a_pool_of_blood() {
    let (mut sim, raws) = fort(90);
    let z = sim.dwarves[0].pos.z;
    let spot = (0..sim.map.width as i32)
        .flat_map(|x| (0..sim.map.height as i32).map(move |y| Pos::new(x, y, z)))
        .find(|&q| sim.map.walkable(q) && !sim.dwarves.iter().any(|d| d.pos == q))
        .expect("an open tile");
    sim.spawn_raider_at(spot, &raws);
    let idx = sim.dwarves.len() - 1;
    // A gaping, bleeding wound and almost no blood left: it bleeds out fast.
    sim.dwarves[idx].blood = 1.0;
    for part in &mut sim.dwarves[idx].body {
        part.bleeding = 40;
    }
    assert!(sim.blood.is_empty(), "no blood before the death");

    for _ in 0..40 {
        sim.step(&raws);
        if !sim.dwarves[idx].alive {
            break;
        }
    }
    assert!(!sim.dwarves[idx].alive, "the raider bled out");
    assert!(!sim.blood.is_empty(), "and its death left blood on the ground");
}
