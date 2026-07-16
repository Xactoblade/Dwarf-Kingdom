//! Bedrooms (BLUEPRINT.md §2.3, "zones: bedrooms…"): a bed is a dwarf's own,
//! they walk to it to sleep, and a bed standing in a room set aside for it is
//! a bedroom — the cheapest happiness a fort can buy.
//!
//! Before this, "having a bed" was a counting trick: the fort's bed total was
//! compared against a dwarf's index. Nobody owned one and nobody walked to one.

mod common;

use dk_agents::{ItemKind, Sim, Task, ThoughtKind};
use dk_world::path::Pos;

fn fort(seed: u64, dwarves: usize) -> (Sim, dk_raws::Raws) {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, seed);
    let mut sim = Sim::new(map, &raws, rng, dwarves);
    sim.invasions = false;
    (sim, raws)
}

/// Drop a bed on reachable flat ground near the fort, on a tile no other bed
/// has taken. (`find_flat_patch` hands back the same patch every time, so walk
/// its cells to find a free one.)
fn put_bed(sim: &mut Sim) -> Pos {
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    let (a, b) = sim.find_flat_patch(cx, cy).expect("bed site");
    let spot = (a.y..=b.y)
        .flat_map(|y| (a.x..=b.x).map(move |x| Pos::new(x, y, a.z)))
        .find(|&p| !sim.items.iter().any(|it| it.active() && it.pos == p))
        .expect("a free tile for the bed");
    sim.debug_spawn_item(ItemKind::Bed, 0, spot);
    spot
}

#[test]
fn a_dwarf_claims_a_bed_and_it_is_theirs_alone() {
    let (mut sim, raws) = fort(8101, 3);
    let a = put_bed(&mut sim);
    let b = put_bed(&mut sim);
    assert_ne!(a, b, "two beds in two places");
    assert!(sim.dwarves.iter().all(|d| d.bed.is_none()), "nobody owns one yet");

    for _ in 0..dk_core::TICKS_PER_DAY * 2 {
        sim.step(&raws);
    }
    let owners: Vec<Option<usize>> = sim.dwarves.iter().map(|d| d.bed).collect();
    let claimed: Vec<usize> = owners.iter().flatten().copied().collect();
    assert_eq!(claimed.len(), 2, "two beds, two owners — the third dwarf goes without");
    assert_ne!(claimed[0], claimed[1], "no two dwarves share a bed");
}

#[test]
fn a_tired_dwarf_walks_to_their_own_bed() {
    let (mut sim, raws) = fort(8102, 1);
    let bed = put_bed(&mut sim);
    // Let them claim it, then send them to bed.
    for _ in 0..dk_core::TICKS_PER_DAY + 1 {
        sim.step(&raws);
    }
    assert!(sim.dwarves[0].bed.is_some(), "claimed the bed");
    sim.dwarves[0].fatigue = 100.0;

    let mut slept_in_it = false;
    for _ in 0..12_000 {
        sim.step(&raws);
        if matches!(sim.dwarves[0].task, Task::Sleep { .. }) && sim.dwarves[0].pos == bed {
            slept_in_it = true;
            break;
        }
    }
    assert!(slept_in_it, "a tired dwarf goes to their own bed and sleeps in it");
}

#[test]
fn a_bed_in_a_bedroom_is_worth_more_than_a_bed_in_a_hall() {
    // Same bed, same sleep — the room is the difference.
    let with_room = {
        let (mut sim, raws) = fort(8103, 1);
        let bed = put_bed(&mut sim);
        sim.add_bedroom(bed, bed);
        sim
            .dwarves
            .first()
            .map(|_| ())
            .expect("a dwarf");
        run_one_sleep(&mut sim, &raws);
        sim.dwarves[0].thoughts.iter().any(|t| t.1 == ThoughtKind::SleptInOwnRoom)
    };
    assert!(with_room, "a bed in a bedroom earns the bedroom thought");

    let without_room = {
        let (mut sim, raws) = fort(8103, 1);
        put_bed(&mut sim);
        run_one_sleep(&mut sim, &raws);
        sim.dwarves[0].thoughts.iter().any(|t| t.1 == ThoughtKind::SleptInBed)
    };
    assert!(without_room, "the same bed out in the open earns only the lesser thought");
}

/// Claim a bed, sleep one full sleep through.
fn run_one_sleep(sim: &mut Sim, raws: &dk_raws::Raws) {
    for _ in 0..dk_core::TICKS_PER_DAY + 1 {
        sim.step(raws);
    }
    sim.dwarves[0].fatigue = 100.0;
    for _ in 0..30_000 {
        sim.step(raws);
        if sim.dwarves[0].thoughts.iter().any(|t| {
            matches!(
                t.1,
                ThoughtKind::SleptInOwnRoom | ThoughtKind::SleptInBed | ThoughtKind::SleptOnFloor
            )
        }) {
            return;
        }
    }
}

#[test]
fn a_dwarf_with_no_bed_sleeps_on_the_stone_and_resents_it() {
    let (mut sim, raws) = fort(8104, 1);
    // No bed anywhere.
    run_one_sleep(&mut sim, &raws);
    assert!(
        sim.dwarves[0].thoughts.iter().any(|t| t.1 == ThoughtKind::SleptOnFloor),
        "no bed, no comfort"
    );
    assert!(
        ThoughtKind::SleptOnFloor.delta() < 0.0,
        "and it is a grievance, not a joy"
    );
}

#[test]
fn a_bed_in_a_bedroom_is_claimed_before_one_out_in_the_open() {
    let (mut sim, raws) = fort(8105, 1);
    let open = put_bed(&mut sim);
    let roomed = put_bed(&mut sim);
    sim.add_bedroom(roomed, roomed);
    let _ = open;

    for _ in 0..dk_core::TICKS_PER_DAY * 2 {
        sim.step(&raws);
    }
    let bed = sim.dwarves[0].bed.expect("claimed a bed");
    assert!(
        sim.bedroom_at(sim.items[bed].pos),
        "the bed in the room a player troubled to build is the one that gets claimed"
    );
}

#[test]
fn a_bed_does_not_travel_with_its_owner() {
    // `Dwarf.bed` is an index into the item vec, and travelling to a new
    // region clears that vec. A bed index carried across would point at
    // whatever now sits at that slot — or off the end of it.
    let (mut sim, raws) = fort(8107, 1);
    let sp = sim.dwarves[0].pos;
    for _ in 0..12 {
        sim.debug_spawn_item(ItemKind::Boulder, 0, sp);
    }
    put_bed(&mut sim);
    for _ in 0..dk_core::TICKS_PER_DAY + 1 {
        sim.step(&raws);
    }
    assert!(sim.dwarves[0].bed.is_some(), "claimed a bed at home");

    sim.begin_adventure(&raws);
    let mut rng2 = dk_core::rng_from_seed(99);
    let new_map = dk_world::generate(&raws.materials, &mut rng2, 32, 32, 16, 99);
    sim.relocate_player(new_map, &raws);

    assert_eq!(
        sim.dwarves[0].bed, None,
        "the bed stayed behind in the old land, and so did its index"
    );
    assert!(
        sim.dwarves.iter().all(|d| d
            .bed
            .is_none_or(|b| sim.items.get(b).is_some_and(|i| i.kind == ItemKind::Bed))),
        "no dwarf owns a 'bed' that is not a bed"
    );
    // And the sim keeps running rather than indexing off the end of the vec.
    sim.dwarves[0].fatigue = 100.0;
    for _ in 0..200 {
        sim.step(&raws);
    }
}

#[test]
fn a_bed_that_is_destroyed_is_given_up_and_another_claimed() {
    // A claim nobody reaps is worse than no claim: its owner is skipped by the
    // assigner forever ("they have a bed") while sleeping on stone every night.
    let (mut sim, raws) = fort(8108, 1);
    put_bed(&mut sim);
    for _ in 0..dk_core::TICKS_PER_DAY + 1 {
        sim.step(&raws);
    }
    let first = sim.dwarves[0].bed.expect("claimed the first bed");

    // The bed is sold to a caravan, or burns. Either way it is gone.
    sim.items[first].consumed = true;
    // The mason builds a replacement.
    let second = put_bed(&mut sim);
    let _ = second;

    for _ in 0..dk_core::TICKS_PER_DAY * 2 {
        sim.step(&raws);
    }
    let now = sim.dwarves[0].bed.expect("claimed the replacement");
    assert_ne!(now, first, "the dead bed was given up");
    assert!(sim.items[now].active(), "and a real one taken in its place");
}

#[test]
fn a_bedroom_designated_later_still_rehouses_its_dwarf() {
    // A player builds beds first and designates rooms afterwards — the usual
    // order. Without an upgrade pass, everyone stays in whatever bed they
    // grabbed on day one and the bedrooms stand empty forever.
    let (mut sim, raws) = fort(8109, 1);
    let open = put_bed(&mut sim);
    let roomed = put_bed(&mut sim);
    for _ in 0..dk_core::TICKS_PER_DAY + 1 {
        sim.step(&raws);
    }
    let first = sim.dwarves[0].bed.expect("claimed a bed");
    assert!(!sim.bedroom_at(sim.items[first].pos), "and it is not in any room yet");
    let _ = open;

    // Now the player builds a bedroom around the other bed.
    sim.add_bedroom(roomed, roomed);
    for _ in 0..dk_core::TICKS_PER_DAY * 2 {
        sim.step(&raws);
    }
    let now = sim.dwarves[0].bed.expect("still has a bed");
    assert!(
        sim.bedroom_at(sim.items[now].pos),
        "the dwarf moved into the room built for them"
    );
}

#[test]
fn a_dead_dwarfs_bed_passes_to_the_living() {
    let (mut sim, raws) = fort(8106, 2);
    put_bed(&mut sim);
    for _ in 0..dk_core::TICKS_PER_DAY * 2 {
        sim.step(&raws);
    }
    let owner = sim
        .dwarves
        .iter()
        .position(|d| d.bed.is_some())
        .expect("someone claimed the one bed");
    let bed = sim.dwarves[owner].bed.unwrap();

    sim.debug_kill_dwarf(owner);
    for _ in 0..dk_core::TICKS_PER_DAY * 2 {
        sim.step(&raws);
    }
    assert_eq!(sim.dwarves[owner].bed, None, "the dead claim no beds");
    let heir = sim
        .dwarves
        .iter()
        .enumerate()
        .find(|(j, d)| *j != owner && d.alive && d.bed == Some(bed));
    assert!(heir.is_some(), "and the survivor inherits it");
}
