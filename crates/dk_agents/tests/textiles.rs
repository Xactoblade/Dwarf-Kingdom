//! Textiles exit tests, run headlessly: adult sheep grow wool that a
//! shepherd shears, and a loom weaves that wool into cloth — a renewable
//! trade good worth far more than the raw fleece.

mod common;

use dk_agents::{item_value, AnimalKind, BuildingKind, ItemKind, Sim};
use dk_core::{DAYS_PER_YEAR, TICKS_PER_DAY};
use dk_world::path::Pos;

fn textile_fort(seed: u64) -> (Sim, dk_raws::Raws) {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, seed);
    let mut sim = Sim::new(map, &raws, rng, 4);
    sim.invasions = false;
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    sim.place_flat_stockpiles(cx, cy, 18);
    let (pa, pb) = sim.find_flat_patch(cx, cy).expect("pasture");
    sim.add_pasture(pa, pb);
    let c = pa;
    sim.add_animal(AnimalKind::Sheep, c, true);
    sim.add_animal(AnimalKind::Sheep, Pos::new(c.x + 1, c.y, c.z), true);
    let (la, _) = sim.find_flat_patch(cx, cy).expect("loom site");
    sim.add_building(BuildingKind::Loom, la);
    (sim, raws)
}

#[test]
fn sheep_grow_wool_that_becomes_cloth() {
    let (mut sim, raws) = textile_fort(1401);
    let mut got_wool = false;
    let mut got_cloth = false;
    // Over half a year: sheep shear at least once, and the loom weaves.
    for _ in 0..(TICKS_PER_DAY * DAYS_PER_YEAR / 2) {
        sim.step(&raws);
        if sim.count_kind(ItemKind::Wool) > 0
            || sim.items.iter().any(|i| i.kind == ItemKind::Wool && i.consumed)
        {
            got_wool = true;
        }
        if sim.count_kind(ItemKind::Cloth) > 0 {
            got_cloth = true;
            break;
        }
    }
    assert!(got_wool, "adult sheep should grow shearable wool");
    assert!(got_cloth, "the loom should weave wool into cloth");
    assert!(sim.stats.cloth_woven > 0);

    // Cloth out-values the raw wool it came from (wool is 4, cloth 18).
    let cloth = sim.items.iter().find(|i| i.active() && i.kind == ItemKind::Cloth).unwrap();
    assert!(item_value(cloth, &raws) > 4, "cloth is worth more than raw wool");
}

#[test]
fn wool_is_renewable_not_a_one_time_drop() {
    let (mut sim, raws) = textile_fort(1402);
    // Remove the loom so wool accumulates instead of being woven.
    sim.buildings.clear();
    let mut shear_events = 0;
    let mut last_seen = 0;
    for _ in 0..(TICKS_PER_DAY * DAYS_PER_YEAR) {
        sim.step(&raws);
        // Count wool that has ever appeared (it may get hauled to stockpiles).
        let wool_now = sim.items.iter().filter(|i| i.kind == ItemKind::Wool).count();
        if wool_now > last_seen {
            shear_events += wool_now - last_seen;
            last_seen = wool_now;
        }
    }
    // Two sheep over a year, shearing on an interval, should yield several
    // fleeces total — proving it recurs, not a single drop.
    assert!(shear_events >= 3, "wool should regrow and be shearable repeatedly ({shear_events})");
}
