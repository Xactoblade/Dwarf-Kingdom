//! Engraving exit tests, run headlessly: a mason smooths a wall and carves
//! into its face a scene from the fortress's own history, and the wall still
//! stands afterward.

mod common;

use dk_agents::{DesignationKind, Sim};
use dk_world::path::Pos;

fn build_sim(seed: u64) -> (Sim, dk_raws::Raws) {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, seed);
    let mut sim = Sim::new(map, &raws, rng, 5);
    sim.invasions = false;
    (sim, raws)
}

#[test]
fn only_bare_walls_can_be_engraved() {
    let (mut sim, _raws) = build_sim(8801);
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    let wz = sim.map.walk_surface_z(cx as usize, cy as usize).unwrap() as i32;

    // The surface floor is walkable — not an engravable wall.
    let floor = Pos::new(cx, cy, wz);
    assert_eq!(sim.designate_rect(DesignationKind::Smooth, floor, floor), 0);

    // A tile deep in the stone is solid — a fine canvas.
    let wall = Pos::new(cx, cy, (wz - 4).max(1));
    assert!(sim.map.tile_at(wall).unwrap().is_solid());
    assert_eq!(sim.designate_rect(DesignationKind::Smooth, wall, wall), 1);
}

#[test]
fn a_mason_engraves_the_fortress_history_into_a_wall() {
    let (mut sim, raws) = build_sim(8802);
    sim.add_embark_supplies(&raws); // food for the long dig
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    let wz = sim.map.walk_surface_z(cx as usize, cy as usize).unwrap() as i32;

    // Dig a stairway and a small room so a wall face is exposed underground.
    let room_z = (wz - 3).max(2);
    for z in room_z..=wz {
        let p = Pos::new(cx, cy, z);
        sim.designate_rect(DesignationKind::Stairs, p, p);
    }
    let room_a = Pos::new(cx + 1, cy - 1, room_z);
    let room_b = Pos::new(cx + 3, cy + 1, room_z);
    sim.designate_rect(DesignationKind::Mine, room_a, room_b);

    // Let the miners open the room.
    let mut t = 0;
    while sim.pending_designations() > 0 && t < 80_000 {
        sim.step(&raws);
        t += 1;
    }
    assert_eq!(sim.pending_designations(), 0, "the room should be dug out");

    // A solid wall bordering the mined room, reachable from its floor.
    let wall = Pos::new(cx + 4, cy, room_z);
    assert!(sim.map.tile_at(wall).unwrap().is_solid(), "the border is stone");
    assert_eq!(
        sim.designate_rect(DesignationKind::Smooth, wall, wall),
        1,
        "the wall is marked for engraving"
    );

    // A mason walks down and carves the scene.
    let mut engraved = None;
    for _ in 0..80_000 {
        sim.step(&raws);
        if let Some(scene) = sim.engravings.get(&wall) {
            engraved = Some(scene.clone());
            break;
        }
    }
    let scene = engraved.expect("the wall should be engraved");
    assert!(scene.starts_with("an engraving of"), "a described scene: {scene}");
    assert!(
        sim.map.tile_at(wall).unwrap().is_solid(),
        "smoothing leaves the wall standing"
    );
    assert!(
        sim.log.iter().any(|(_, m)| m.contains("engraves a wall")),
        "the engraving is recorded in the annals"
    );
}
