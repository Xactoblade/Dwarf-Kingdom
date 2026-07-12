//! Ghosts & burials exit tests, run headlessly: the dead leave remains,
//! neglect raises restless ghosts who torment the living, and a proper
//! tomb burial grants everyone peace.

mod common;

use dk_agents::{BuildingKind, ItemKind, Sim, ThoughtKind, GHOST_AFTER_DAYS};
use dk_core::TICKS_PER_DAY;
use dk_world::path::Pos;

fn fort(seed: u64) -> (Sim, dk_raws::Raws) {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, seed);
    let mut sim = Sim::new(map, &raws, rng, 5);
    sim.invasions = false;
    sim.add_embark_supplies(&raws);
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    sim.place_flat_stockpiles(cx, cy, 18);
    (sim, raws)
}

#[test]
fn the_unburied_dead_rise_and_torment() {
    let (mut sim, raws) = fort(1001);
    let victim = 0;
    let name = sim.dwarves[victim].name.clone();
    sim.slay(victim);

    // The body remains where they fell.
    assert!(
        sim.items.iter().any(|it| {
            it.active()
                && it.kind == ItemKind::Corpse
                && it.name.as_deref().is_some_and(|n| n.contains(&name))
        }),
        "a citizen's death leaves remains"
    );

    // Without a tomb, the corpse has nowhere to go; the ghost rises.
    let wait = (GHOST_AFTER_DAYS + 3) * TICKS_PER_DAY;
    for _ in 0..wait {
        sim.step(&raws);
    }
    assert!(sim.dwarves[victim].ghost, "neglected dead do not stay quiet");
    assert!(
        sim.log.iter().any(|(_, m)| m.contains("restless ghost")),
        "the haunting is announced"
    );

    // The living suffer for it.
    let mut haunted = sim
        .dwarves
        .iter()
        .filter(|d| d.alive)
        .flat_map(|d| d.thoughts.iter())
        .filter(|(_, t)| *t == ThoughtKind::Haunted)
        .count();
    let mut extra = 0;
    while haunted == 0 && extra < 10 * TICKS_PER_DAY {
        sim.step(&raws);
        extra += 1;
        haunted = sim
            .dwarves
            .iter()
            .filter(|d| d.alive)
            .flat_map(|d| d.thoughts.iter())
            .filter(|(_, t)| *t == ThoughtKind::Haunted)
            .count();
    }
    assert!(haunted > 0, "ghosts torment the living");
}

#[test]
fn burial_grants_peace() {
    let (mut sim, raws) = fort(1002);
    // Build a tomb on open ground before anyone dies.
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    let (ta, _) = sim.find_flat_patch(cx, cy).expect("tomb site");
    assert!(sim.add_building(BuildingKind::Tomb, ta));

    let victim = 0;
    let name = sim.dwarves[victim].name.clone();
    sim.slay(victim);

    // A hauler carries the remains to the tomb and buries them.
    let mut buried = false;
    for _ in 0..(GHOST_AFTER_DAYS + 6) * TICKS_PER_DAY {
        sim.step(&raws);
        if sim.log.iter().any(|(_, m)| m.contains("laid to rest")) {
            buried = true;
            break;
        }
    }
    assert!(buried, "the fort buries its dead: {name}");
    assert!(
        sim.buildings.iter().any(|b| b.kind == BuildingKind::Tomb && b.occupied),
        "the tomb is occupied"
    );
    // No corpse left above ground; the dead never rise.
    assert!(
        !sim.items.iter().any(|it| it.active() && it.kind == ItemKind::Corpse),
        "the remains are gone from the surface"
    );
    assert!(!sim.dwarves[victim].ghost, "the buried rest quietly");
}

#[test]
fn a_late_burial_lays_the_ghost_to_rest() {
    let (mut sim, raws) = fort(1003);
    let victim = 0;
    sim.slay(victim);

    // Let the ghost rise first.
    for _ in 0..(GHOST_AFTER_DAYS + 3) * TICKS_PER_DAY {
        sim.step(&raws);
    }
    assert!(sim.dwarves[victim].ghost);

    // Now build the tomb; the fort makes amends.
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    let (ta, _) = sim.find_flat_patch(cx, cy).expect("tomb site");
    assert!(sim.add_building(BuildingKind::Tomb, ta));

    let mut at_peace = false;
    for _ in 0..20 * TICKS_PER_DAY {
        sim.step(&raws);
        if !sim.dwarves[victim].ghost {
            at_peace = true;
            break;
        }
    }
    assert!(at_peace, "burial quiets the restless dead");
    assert!(
        sim.log.iter().any(|(_, m)| m.contains("at peace")),
        "the peace is recorded"
    );
}
