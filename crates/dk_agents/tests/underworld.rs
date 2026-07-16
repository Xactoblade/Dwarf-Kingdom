//! The underworld, run headlessly: mining a hollow adamantine cap breaches the
//! abyss and looses a horde of demons. A fort that never digs one is untouched
//! — the whole mechanic is gated on a breach existing (adamantine is seeded
//! only at app embark), so it draws no rng and changes nothing without one.

mod common;

use dk_agents::{DesignationKind, Faction, Sim};
use dk_world::path::Pos;

fn deep_fort(seed: u64) -> (Sim, dk_raws::Raws) {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 40, 40, 16, seed);
    let mut sim = Sim::new(map, &raws, rng, 6);
    sim.invasions = false;
    sim.add_embark_supplies(&raws);
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    sim.place_flat_stockpiles(cx, cy, 27);
    (sim, raws)
}

#[test]
fn digging_a_breach_looses_the_underworld() {
    let (mut sim, raws) = deep_fort(5501);
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    let wz = sim.map.walk_surface_z(cx as usize, cy as usize).unwrap() as i32;
    // Stair down a few levels, then mark a stone tile beside the shaft as a
    // hollow adamantine cap and designate it to be mined.
    let bz = (wz - 3).max(2);
    for z in bz..=wz {
        sim.designate_rect(DesignationKind::Stairs, Pos::new(cx, cy, z), Pos::new(cx, cy, z));
    }
    let breach = Pos::new(cx + 1, cy, bz);
    sim.adamantine_breaches.insert(breach);
    sim.designate_rect(DesignationKind::Mine, breach, breach);

    let mut breached = false;
    for _ in 0..40_000 {
        sim.step(&raws);
        if sim.stats.demons_loosed > 0 {
            breached = true;
            break;
        }
    }
    assert!(breached, "mining the hollow cap should loose demons");
    assert!(
        sim.dwarves
            .iter()
            .any(|d| d.alive && d.faction == Faction::Hostile && d.beast),
        "demons now walk the deep"
    );
    assert!(sim.adamantine_breaches.is_empty(), "the cap is spent once broken");
}

#[test]
fn a_fort_that_never_digs_a_breach_is_untouched() {
    let (mut sim, raws) = deep_fort(5502);
    assert!(sim.adamantine_breaches.is_empty());
    for _ in 0..2_000 {
        sim.step(&raws);
    }
    assert_eq!(sim.stats.demons_loosed, 0, "no breach, no demons");
    assert!(!sim.dwarves.iter().any(|d| d.beast), "and nothing rose from below");
}
