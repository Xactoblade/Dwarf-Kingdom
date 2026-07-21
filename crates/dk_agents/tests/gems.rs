//! Gem exit tests, run headlessly: mining occasionally strikes a rough gem,
//! and a jeweler cuts it into a brilliant, high-value trade good.

mod common;

use dk_agents::{item_value, BuildingKind, DesignationKind, ItemKind, Sim};
use dk_world::path::Pos;

fn mining_fort(seed: u64) -> (Sim, dk_raws::Raws) {
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

/// Dig a big block of stone below the surface so many boulders are mined.
fn designate_big_dig(sim: &mut Sim) {
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    let wz = sim.map.walk_surface_z(cx as usize, cy as usize).unwrap() as i32;
    // Stair down, then a wide mined room in the stone.
    let room_z = (wz - 4).max(2);
    for z in room_z..=wz {
        sim.designate_rect(DesignationKind::Stairs, Pos::new(cx, cy, z), Pos::new(cx, cy, z));
    }
    // A wide room: enough boulders that the 1-in-22 gem strike is near-certain
    // for any seed (13x13 tiles => P(no gem) well under a percent), so the test
    // doesn't hinge on one lucky seed's exact rng stream.
    sim.designate_rect(
        DesignationKind::Mine,
        Pos::new(cx - 6, cy - 6, room_z),
        Pos::new(cx + 6, cy + 6, room_z),
    );
}

#[test]
fn mining_sometimes_strikes_gems_and_the_jeweler_cuts_them() {
    let (mut sim, raws) = mining_fort(1801);
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    // A jeweler ready to cut whatever turns up.
    let (ja, _) = sim.find_flat_patch(cx, cy).expect("jeweler site");
    sim.add_building(BuildingKind::Jeweler, ja);
    designate_big_dig(&mut sim);

    let mut found = false;
    let mut cut = false;
    for _ in 0..120_000 {
        sim.step(&raws);
        if sim.stats.gems_found > 0 {
            found = true;
        }
        if sim.count_kind(ItemKind::CutGem) > 0 {
            cut = true;
            break;
        }
        // Keep the dig going by re-designating if it all completed with no
        // gem yet (unlucky seed) — but the room is large, so this is rare.
        if sim.pending_designations() == 0 && !found {
            designate_big_dig(&mut sim);
        }
    }
    assert!(found, "a big dig should turn up at least one rough gem");
    assert!(cut, "the jeweler should cut a rough gem into a brilliant one");
    assert!(sim.stats.gems_cut > 0);

    // A cut gem is the fort's most valuable ordinary trade good.
    let cut_gem = sim.items.iter().find(|i| i.active() && i.kind == ItemKind::CutGem).unwrap();
    assert!(
        item_value(cut_gem, &raws) > 40,
        "a cut gem carries premium trade value"
    );
    // The gem keeps its variety (a valid gem index).
    assert!((cut_gem.stuff as usize) < raws.gems.len());
}

/// A cut gem stores its variety by index into the (now data-driven) gem
/// registry; a save records the gem id-manifest and remaps it on load, so the
/// gem survives a round trip.
#[test]
fn a_cut_gem_survives_a_save_and_reload() {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(9);
    let map = dk_world::generate(&raws.materials, &mut rng, 24, 24, 12, 9);
    let mut sim = Sim::new(map, &raws, rng, 2);
    let diamond = raws.gems.index_of("diamond").expect("diamond is a base gem");
    let pos = sim.dwarves[0].pos;
    sim.debug_spawn_item(ItemKind::CutGem, diamond, pos);

    let path = std::env::temp_dir().join(format!("dk_gem_{}.bin", std::process::id()));
    dk_agents::save_sim(&sim, &path, &raws).expect("save");
    let loaded = dk_agents::load_sim(&path, &raws).expect("load");

    let gem = loaded
        .items
        .iter()
        .find(|i| i.active() && i.kind == ItemKind::CutGem)
        .expect("the cut gem is still there");
    assert_eq!(raws.gems.name(gem.stuff), "diamond", "still a diamond after reload");
    let _ = std::fs::remove_file(&path);
}
