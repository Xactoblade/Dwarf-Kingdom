//! Leather exit tests, run headlessly: a tanner's shop tans raw hides into
//! leather, and butchering saves a hide ONLY once a tanner stands (so a fort
//! without one butchers exactly as before). The chain: butcher -> hide ->
//! tanner -> leather.

mod common;

use dk_agents::{AnimalKind, BuildingKind, ItemKind, Sim};
use dk_world::path::Pos;

fn tan_fort(seed: u64) -> (Sim, dk_raws::Raws) {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, seed);
    let mut sim = Sim::new(map, &raws, rng, 4);
    sim.invasions = false;
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    sim.add_embark_supplies(&raws);
    sim.place_flat_stockpiles(cx, cy, 24);
    (sim, raws)
}

#[test]
fn a_tanner_tans_hides_into_leather() {
    let (mut sim, raws) = tan_fort(1201);
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    let (ta, _) = sim.find_flat_patch(cx, cy).expect("tanner site");
    assert!(sim.add_building(BuildingKind::Tanner, ta));
    let sp = sim.dwarves[0].pos;
    for _ in 0..6 {
        sim.debug_spawn_hide(sp);
    }
    assert_eq!(sim.count_kind(ItemKind::Leather), 0, "nothing tanned yet");

    let mut tanned = false;
    for _ in 0..12_000 {
        sim.step(&raws);
        if sim.stats.leather_tanned > 0 {
            tanned = true;
            break;
        }
    }
    assert!(tanned, "the tanner should tan a hide into leather");
    assert!(sim.count_kind(ItemKind::Leather) > 0, "leather exists in the fort");
}

/// Butcher a sheep and return how many hides ended up in the fort.
fn hides_from_butchering(seed: u64, with_tanner: bool) -> u32 {
    let (mut sim, raws) = tan_fort(seed);
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    if with_tanner {
        let (ta, _) = sim.find_flat_patch(cx, cy).expect("tanner site");
        sim.add_building(BuildingKind::Tanner, ta);
    }
    // A sheep beside the dwarves, marked for slaughter.
    let sp = sim.dwarves[0].pos;
    let ap = Pos::new(sp.x + 1, sp.y, sp.z);
    sim.add_animal(AnimalKind::Sheep, ap, true);
    sim.mark_nearest_animal(ap);
    for _ in 0..12_000 {
        sim.step(&raws);
        if sim.stats.animals_butchered > 0 {
            break;
        }
    }
    assert!(sim.stats.animals_butchered > 0, "the sheep should be butchered");
    sim.count_kind(ItemKind::Hide) as u32 + sim.stats.leather_tanned
}

#[test]
fn butchering_saves_a_hide_only_with_a_tanner() {
    assert!(
        hides_from_butchering(1202, true) >= 1,
        "with a tanner, butchering yields a hide"
    );
    assert_eq!(
        hides_from_butchering(1202, false),
        0,
        "with no tanner, the hide is not saved (fort butchers as before)"
    );
}
