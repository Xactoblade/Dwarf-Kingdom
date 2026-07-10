//! Phase 1 exit test (BLUEPRINT.md §5, Phase 1), run headlessly:
//! "designate a 3-level staircase and a stockpile; dwarves dig it out and
//! haul the stone with zero babysitting."

mod common;

use dk_agents::{DesignationKind, ItemKind, Sim};
use dk_raws::Raws;
use dk_world::path::Pos;
use dk_world::TileShape;

fn build_sim(seed: u64) -> (Sim, Raws) {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, seed);
    let sim = Sim::new(map, &raws, rng, 5);
    (sim, raws)
}

/// Apply the canonical Phase 1 scenario near the map center: a staircase down
/// into the stone layers, a mined room at the bottom, and a stockpile on the
/// surface. The room depth is computed from the terrain so every mined tile
/// is stone (soil digs away without dropping a boulder).
fn designate_scenario(sim: &mut Sim) -> (Pos, Pos) {
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    let wz = sim.map.walk_surface_z(cx as usize, cy as usize).unwrap() as i32;

    // Room rect, placed 3+ tiles below the lowest surface it spans → stone.
    let (rx0, rx1, ry0, ry1) = (cx + 1, cx + 4, cy - 1, cy + 1);
    let min_surface = (ry0..=ry1)
        .flat_map(|y| (rx0..=rx1).map(move |x| (x, y)))
        .chain(std::iter::once((cx, cy)))
        .map(|(x, y)| sim.map.surface_z(x as usize, y as usize).unwrap() as i32)
        .min()
        .unwrap();
    let room_z = (min_surface - 3).max(2);

    // Staircase column from the surface floor down to the room level.
    let stair_top = Pos::new(cx, cy, wz);
    for z in room_z..=wz {
        let p = Pos::new(cx, cy, z);
        let added = sim.designate_rect(DesignationKind::Stairs, p, p);
        assert_eq!(added, 1, "stair designation at {p:?} must be workable");
    }

    let room_a = Pos::new(rx0, ry0, room_z);
    let room_b = Pos::new(rx1, ry1, room_z);
    let added = sim.designate_rect(DesignationKind::Mine, room_a, room_b);
    assert!(added >= 10, "room should designate mostly solid tiles, got {added}");

    // Stockpiles on the surface: flat 3x3 patches (one item per cell) until
    // capacity comfortably exceeds the expected boulder count.
    let cells = sim.place_flat_stockpiles(cx, cy, 27);
    assert!(cells >= 18, "not enough flat stockpile sites near spawn ({cells} cells)");
    (stair_top, Pos::new(cx, cy, room_z))
}

#[test]
fn dwarves_dig_staircase_and_haul_to_stockpile() {
    let (mut sim, raws) = build_sim(99);
    let (_, stair_bottom) = designate_scenario(&mut sim);
    assert!(sim.pending_designations() >= 13);

    let mut ticks = 0u64;
    while sim.pending_designations() > 0 && ticks < 60_000 {
        sim.step(&raws);
        ticks += 1;
    }
    assert_eq!(
        sim.pending_designations(),
        0,
        "all designations should complete within {ticks} ticks"
    );

    // The staircase connects the surface to the room level.
    assert_eq!(
        sim.map.tile_at(stair_bottom).unwrap().shape,
        TileShape::Stairs
    );

    // Stones dropped from the stone layers below the soil.
    let boulders = sim.count_kind(ItemKind::Boulder);
    assert!(boulders > 0, "mining stone tiles must drop boulders");

    // Give the haulers time to finish (carried boulders count as unfinished).
    let mut extra = 0u64;
    while extra < 60_000 {
        sim.step(&raws);
        extra += 1;
        if sim.stored_items() >= boulders {
            break;
        }
    }
    let stored = sim.stored_items();
    assert!(
        stored >= boulders,
        "all {boulders} boulders should be stored, got {stored} (after {extra} extra ticks)"
    );
}

#[test]
fn simulation_is_deterministic() {
    let (mut a, raws) = build_sim(1234);
    let (mut b, _) = build_sim(1234);
    designate_scenario(&mut a);
    designate_scenario(&mut b);
    for _ in 0..10_000 {
        a.step(&raws);
        b.step(&raws);
    }
    assert_eq!(
        bincode::serialize(&a.dwarves).unwrap(),
        bincode::serialize(&b.dwarves).unwrap(),
        "same seed must produce identical simulations"
    );
    assert_eq!(
        bincode::serialize(&a.items).unwrap(),
        bincode::serialize(&b.items).unwrap()
    );
}

#[test]
fn save_roundtrip_preserves_sim() {
    let (mut sim, raws) = build_sim(7);
    designate_scenario(&mut sim);
    sim.add_embark_supplies(&raws);
    for _ in 0..2_000 {
        sim.step(&raws);
    }
    let path = std::env::temp_dir().join("dk_agents_test").join("sim.bin");
    dk_agents::save_sim(&sim, &path, &raws).unwrap();
    let mut loaded = dk_agents::load_sim(&path, &raws).unwrap();
    assert_eq!(loaded.dwarves.len(), sim.dwarves.len());
    assert_eq!(loaded.items.len(), sim.items.len());
    assert_eq!(loaded.pending_designations(), sim.pending_designations());
    // And it keeps running deterministically from the restored state.
    for _ in 0..1_000 {
        sim.step(&raws);
        loaded.step(&raws);
    }
    assert_eq!(
        bincode::serialize(&sim.dwarves).unwrap(),
        bincode::serialize(&loaded.dwarves).unwrap()
    );
}
