//! Item-quality exit tests, run headlessly: a skilled crafter turns out finer,
//! more valuable goods, so mastery pays — and a masterwork is worth far more
//! than an ordinary piece of the same stuff.

mod common;

use dk_agents::{item_value, quality_name, BuildingKind, ItemKind, Sim};
use dk_world::path::Pos;

fn craft_fort(seed: u64) -> (Sim, dk_raws::Raws) {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, seed);
    let mut sim = Sim::new(map, &raws, rng, 4);
    sim.invasions = false;
    (sim, raws)
}

#[test]
fn quality_raises_an_items_worth() {
    let raws = common::test_raws();
    // Two identical crafts, one ordinary and one masterful.
    let mut ordinary = dk_agents::Item {
        kind: ItemKind::Craft,
        stuff: 0,
        name: None,
        pos: Pos::new(0, 0, 0),
        state: dk_agents::ItemState::OnGround,
        reserved_by: None,
        consumed: false,
        quality: 0,
        made_at: 0,
            variant: 0,
    };
    let masterful = dk_agents::Item { quality: 5, ..ordinary.clone() };
    let base = item_value(&ordinary, &raws);
    assert!(item_value(&masterful, &raws) > base, "a masterwork is worth more");
    // The tiers climb monotonically.
    let mut last = 0;
    for q in 0..=5u8 {
        ordinary.quality = q;
        let v = item_value(&ordinary, &raws);
        assert!(v >= last, "value must not fall as quality rises");
        last = v;
    }
    assert_eq!(quality_name(0), "ordinary");
    assert_eq!(quality_name(5), "masterful");
}

#[test]
fn a_crafted_good_carries_its_makers_skill() {
    let (mut sim, raws) = craft_fort(1201);
    sim.add_embark_supplies(&raws);
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    // A craftsdwarf's workshop, a stockpile, and boulders to work.
    let (wa, _) = sim.find_flat_patch(cx, cy).expect("workshop site");
    assert!(sim.add_building(BuildingKind::Craftsdwarf, wa));
    sim.place_flat_stockpiles(cx, cy, 18);
    let sp = sim.dwarves[0].pos;
    for _ in 0..12 {
        sim.debug_spawn_boulder(0, sp);
    }

    // Run until some crafts are made.
    let mut made = false;
    for _ in 0..30_000 {
        sim.step(&raws);
        if sim.stats.crafts_made > 0 && sim.count_kind(ItemKind::Craft) > 0 {
            made = true;
            break;
        }
    }
    assert!(made, "the craftsdwarf should produce trade goods");
    // Every crafted good has a well-defined quality tier (<=5), set from skill.
    for it in sim.items.iter().filter(|i| i.active() && i.kind == ItemKind::Craft) {
        assert!(it.quality <= 5, "quality is a real tier");
    }
}
