//! Mason / furniture exit tests, run headlessly: a mason's workshop works stone
//! into beds, and a dwarf who owns a bed sleeps more soundly than one on the
//! bare floor — mending faster. Beds are claimed by citizens in index order,
//! the same proven model as the weapon/armor armory. A fort with no beds
//! sleeps exactly as before.

mod common;

use dk_agents::{BuildingKind, ItemKind, Sim, Task};
use dk_world::path::Pos;

/// A one-dwarf fort with a single wounded, sleeping citizen — optionally given a
/// bed. One citizen so that, at a fixed seed, the ONLY difference between the
/// bedded and bare runs is the rest bonus: nothing else can perturb the rng.
fn convalescent(seed: u64, bedded: bool) -> (Sim, dk_raws::Raws) {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, seed);
    let mut sim = Sim::new(map, &raws, rng, 1);
    sim.invasions = false;
    let sp = sim.dwarves[0].pos;
    // Open a wound with room to heal, and lay the dwarf down for a long sleep.
    let torso = sim.dwarves[0]
        .body
        .iter_mut()
        .find(|p| p.max_hp >= 40)
        .expect("a torso to wound");
    torso.hp = torso.max_hp - 20;
    sim.dwarves[0].task = Task::Sleep { remaining: 60_000 };
    if bedded {
        sim.debug_spawn_bed(0, sp); // no rng — keeps the streams aligned
    }
    (sim, raws)
}

fn total_hp(sim: &Sim, i: usize) -> i32 {
    sim.dwarves[i].body.iter().map(|p| p.hp as i32).sum()
}

#[test]
fn a_bed_mends_its_owner_faster() {
    let (mut bedded, rb) = convalescent(6301, true);
    let (mut bare, rbare) = convalescent(6301, false);
    let start = total_hp(&bare, 0);

    for _ in 0..1_500 {
        bedded.step(&rb);
        bare.step(&rbare);
    }

    let bedded_hp = total_hp(&bedded, 0);
    let bare_hp = total_hp(&bare, 0);
    assert!(bare_hp > start, "even on the floor a sleeper heals a little");
    assert!(
        bedded_hp > bare_hp,
        "a bed should mend its owner faster (bedded {bedded_hp} vs bare {bare_hp})"
    );
}

#[test]
fn a_mason_builds_beds_from_stone() {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(6302);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, 6302);
    let mut sim = Sim::new(map, &raws, rng, 4);
    sim.invasions = false;
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;

    sim.add_embark_supplies(&raws);
    let (ma, _) = sim.find_flat_patch(cx, cy).expect("mason site");
    assert!(sim.add_building(BuildingKind::Mason, ma));
    sim.place_flat_stockpiles(cx, cy, 24);
    let sp = sim.dwarves[0].pos;
    for _ in 0..10 {
        sim.debug_spawn_boulder(0, sp);
    }
    assert_eq!(sim.count_kind(ItemKind::Bed), 0, "no beds built yet");

    let mut made = false;
    for _ in 0..12_000 {
        sim.step(&raws);
        if sim.stats.furniture_made > 0 {
            made = true;
            break;
        }
    }
    assert!(made, "the mason should work a boulder into a bed");
    assert!(sim.count_kind(ItemKind::Bed) > 0, "a bed exists in the fort");
}
