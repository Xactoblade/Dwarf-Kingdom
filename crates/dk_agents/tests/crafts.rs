//! Craftsdwarf's workshop exit tests, run headlessly: surplus stone is
//! worked into trade goods worth several times the raw boulder, closing
//! the mine → craft → sell loop.

mod common;

use dk_agents::{item_value, BuildingKind, ItemKind, Sim};
use dk_world::path::Pos;

fn craft_fort(seed: u64) -> (Sim, dk_raws::Raws) {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, seed);
    let mut sim = Sim::new(map, &raws, rng, 4);
    sim.invasions = false;
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    sim.place_flat_stockpiles(cx, cy, 27);
    // A craftsdwarf's workshop and a heap of granite to work.
    let (wa, _) = sim.find_flat_patch(cx, cy).expect("workshop site");
    assert!(sim.add_building(BuildingKind::Craftsdwarf, wa));
    let granite = raws.materials.index_of("granite").unwrap();
    for d in 0..10 {
        if let Some(z) = sim.map.walk_surface_z((cx + d % 5) as usize, (cy + 3 + d / 5) as usize) {
            sim.debug_spawn_boulder(granite, Pos::new(cx + d % 5, cy + 3 + d / 5, z as i32));
        }
    }
    (sim, raws)
}

#[test]
fn surplus_stone_is_worked_into_trade_goods() {
    let (mut sim, raws) = craft_fort(1201);
    let boulders_before = sim.count_kind(ItemKind::Boulder);
    assert!(boulders_before >= 10);

    let mut made = false;
    for _ in 0..40_000 {
        sim.step(&raws);
        if sim.count_kind(ItemKind::Craft) > 0 {
            made = true;
            break;
        }
    }
    assert!(made, "a workshop with spare stone should produce crafts");
    assert!(sim.stats.crafts_made > 0);
    assert!(
        sim.count_kind(ItemKind::Boulder) < boulders_before,
        "crafting consumes boulders"
    );
    // A craft is worth several times its raw stone.
    let craft = sim.items.iter().find(|i| i.active() && i.kind == ItemKind::Craft).unwrap();
    let boulder = sim.items.iter().find(|i| i.active() && i.kind == ItemKind::Boulder);
    let craft_val = item_value(craft, &raws);
    if let Some(b) = boulder {
        assert!(
            craft_val > item_value(b, &raws),
            "worked goods should out-value raw stone ({} vs {})",
            craft_val,
            item_value(b, &raws)
        );
    }
    assert!(craft_val > 10, "a craft carries real trade value");
}

#[test]
fn skeletonized_bones_are_carved_into_trinkets() {
    // The battlefield's leavings, once picked clean to bone, are a craftsdwarf's
    // free trade stock: no stone spent, just gore turned to goods.
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(1203);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, 1203);
    let mut sim = Sim::new(map, &raws, rng, 4);
    sim.invasions = false;
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    sim.place_flat_stockpiles(cx, cy, 27);
    let (wa, _) = sim.find_flat_patch(cx, cy).expect("workshop site");
    assert!(sim.add_building(BuildingKind::Craftsdwarf, wa));
    // A scatter of clean bones on the ground — and not a single boulder.
    let mut bones = 0;
    for d in 0..8 {
        if let Some(z) = sim.map.walk_surface_z((cx + d % 4) as usize, (cy + 3 + d / 4) as usize) {
            sim.debug_spawn_bone(Pos::new(cx + d % 4, cy + 3 + d / 4, z as i32));
            bones += 1;
        }
    }
    assert!(bones >= 4, "placed a handful of bones");
    assert_eq!(sim.count_kind(ItemKind::Boulder), 0, "no stone in this fort");

    let mut made = false;
    for _ in 0..40_000 {
        sim.step(&raws);
        if sim.count_kind(ItemKind::BoneCraft) > 0 {
            made = true;
            break;
        }
    }
    assert!(made, "a craftsdwarf should carve loose bones into trinkets");
    assert!(sim.stats.crafts_made > 0);
    // Carving consumed at least one bone.
    assert!(
        sim.count_kind(ItemKind::BodyPart) < bones,
        "carving consumes bones"
    );
    // A bone trinket carries real, if modest, trade value.
    let trinket = sim
        .items
        .iter()
        .find(|i| i.active() && i.kind == ItemKind::BoneCraft)
        .unwrap();
    assert!(item_value(trinket, &raws) > 0, "a bone trinket sells for something");
}

#[test]
fn a_reserve_of_stone_is_kept_for_building() {
    // With only a few boulders the fort should NOT craft them all away —
    // it keeps a reserve. Give exactly the threshold-ish amount.
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(1202);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, 1202);
    let mut sim = Sim::new(map, &raws, rng, 4);
    sim.invasions = false;
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    sim.place_flat_stockpiles(cx, cy, 18);
    let (wa, _) = sim.find_flat_patch(cx, cy).unwrap();
    sim.add_building(BuildingKind::Craftsdwarf, wa);
    let granite = raws.materials.index_of("granite").unwrap();
    for d in 0..4 {
        if let Some(z) = sim.map.walk_surface_z((cx + d) as usize, (cy + 3) as usize) {
            sim.debug_spawn_boulder(granite, Pos::new(cx + d, cy + 3, z as i32));
        }
    }
    for _ in 0..20_000 {
        sim.step(&raws);
    }
    assert!(
        sim.count_kind(ItemKind::Boulder) > 0,
        "the fort should not craft its last stones away"
    );
}
