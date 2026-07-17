//! Squads (combat stage 3): the fort's soldiers, organised under a standing
//! order the player sets. A defending squad sallies out to hunt; a stationed
//! one holds its post and lets a distant foe be; a training one keeps to the
//! barracks. Enlisting musters a squad; dismissal and death empty it.

mod common;

use dk_agents::{Faction, Sim, SquadOrder, Task, SQUAD_MAX};
use dk_world::path::Pos;
use dk_world::{Map, Tile};

fn arena(dwarves: usize) -> (Sim, dk_raws::Raws) {
    let raws = common::test_raws();
    let mut m = Map::new_air(40, 40, 4, 0);
    for y in 0..40 {
        for x in 0..40 {
            m.set(x, y, 0, Tile::solid(2));
            m.set(x, y, 1, Tile::floor(2));
        }
    }
    let rng = dk_core::rng_from_seed(4242);
    let mut sim = Sim::new(m, &raws, rng, dwarves);
    sim.water.springs.clear();
    sim.invasions = false;
    sim.rebuild_caches();
    (sim, raws)
}

/// Arm the fort dwarf at index `i`: enlist and hand the armoury a blade.
fn arm_soldier(sim: &mut Sim, raws: &dk_raws::Raws, i: usize) {
    sim.toggle_soldier(sim.dwarves[i].pos);
    let iron = raws.materials.index_of("hematite").unwrap();
    sim.debug_spawn_item(dk_agents::ItemKind::Weapon, iron, sim.dwarves[i].pos);
}

#[test]
fn enlisting_musters_a_squad_dismissal_empties_it() {
    let (mut sim, _raws) = arena(2);
    assert!(sim.squads.is_empty(), "no soldiers, no squads");

    sim.toggle_soldier(sim.dwarves[0].pos);
    assert_eq!(sim.squads.len(), 1, "the first soldier musters a squad");
    assert_eq!(sim.squads[0].members, vec![0]);
    assert_eq!(sim.squads[0].order, SquadOrder::Defend, "new squads defend by default");

    sim.toggle_soldier(sim.dwarves[1].pos);
    assert_eq!(sim.squads[0].members, vec![0, 1], "the second joins the same squad");

    // Discharge them both; the emptied squad is struck.
    sim.toggle_soldier(sim.dwarves[0].pos);
    sim.toggle_soldier(sim.dwarves[1].pos);
    assert!(sim.squads.is_empty(), "an empty squad does not linger");
}

#[test]
fn an_eleventh_soldier_musters_a_second_squad() {
    let (mut sim, _raws) = arena(SQUAD_MAX + 1);
    for i in 0..(SQUAD_MAX + 1) {
        sim.toggle_soldier(sim.dwarves[i].pos);
    }
    assert_eq!(sim.squads.len(), 2, "a squad caps at ten; the eleventh starts a new one");
    assert_eq!(sim.squads[0].members.len(), SQUAD_MAX);
    assert_eq!(sim.squads[1].members.len(), 1);
}

/// Open arena: a soldier tucked in the northwest corner with a barracks, and a
/// raider in the far southeast corner — same region, reachable. A defending
/// soldier will march out to meet it; a trainee will let it come.
fn open_arena() -> (Sim, dk_raws::Raws, usize) {
    let (mut sim, raws) = arena(1);
    sim.dwarves[0].pos = Pos::new(2, 2, 1);
    sim.add_barracks(Pos::new(4, 4, 1), Pos::new(7, 7, 1));
    sim.rebuild_caches();
    arm_soldier(&mut sim, &raws, 0);
    sim.spawn_raider_at(Pos::new(36, 36, 1), &raws);
    let raider = sim.dwarves.len() - 1;
    (sim, raws, raider)
}

#[test]
fn a_defending_squad_marches_out_to_meet_the_raider() {
    // A defender takes the initiative: it crosses the map toward the raider
    // rather than waiting to be attacked in the corner.
    let (mut sim, raws, _raider) = open_arena();
    let home = sim.dwarves[0].pos;
    sim.set_squad_order(0, SquadOrder::Defend);
    let mut sallied = false;
    for _ in 0..4_000 {
        sim.step(&raws);
        // Far from its corner — it took the initiative and marched out to fight.
        if sim.dwarves[0].alive && sim.dwarves[0].pos.manhattan(home) >= 15 {
            sallied = true;
            break;
        }
        if !sim.dwarves[0].alive {
            break;
        }
    }
    assert!(sallied, "a defending squad marches out to meet the raider");
}

#[test]
fn a_training_squad_lets_the_raider_come_to_it() {
    // A trainee never marches out — it holds in its corner (the raider must
    // come to it). Contrast with the defender, who crosses the map.
    let (mut sim, raws, raider) = open_arena();
    let home = sim.dwarves[0].pos;
    sim.set_squad_order(0, SquadOrder::Train);
    let mut marched_out = false;
    for _ in 0..4_000 {
        sim.step(&raws);
        // A trainee stays near its corner; the raider must come to it. It never
        // ranges far from home to go hunting.
        if sim.dwarves[0].alive && sim.dwarves[0].pos.manhattan(home) >= 15 {
            marched_out = true;
            break;
        }
        // Once the threat is dealt with (raider dead or soldier fallen) the
        // test is decided — a trainee never sallied while the enemy stood.
        if !sim.dwarves[0].alive || !sim.dwarves[raider].alive {
            break;
        }
    }
    assert!(!marched_out, "a training soldier does not march across the map to hunt");
}

#[test]
fn a_stationed_squad_holds_its_post_but_strikes_what_comes_near() {
    let (mut sim, raws, raider) = open_arena();
    // Kill the wandering corner raider off first so it can't confound the post
    // test by strolling into range.
    sim.debug_kill_dwarf(raider);
    let post = Pos::new(12, 12, 1);
    sim.set_squad_order(0, SquadOrder::Station(post));

    // The soldier marches to its post and holds there.
    let mut reached_post = false;
    for _ in 0..3_000 {
        sim.step(&raws);
        if sim.dwarves[0].pos.manhattan(post) <= 1 {
            reached_post = true;
            break;
        }
    }
    assert!(reached_post, "the soldier marched to its post");

    // It holds the post, never wandering far from it.
    for _ in 0..1_000 {
        sim.step(&raws);
        assert!(
            sim.dwarves[0].pos.manhattan(post) <= 2,
            "a stationed soldier holds near its post"
        );
    }
    assert!(matches!(sim.dwarves[0].task, Task::Station { .. }));

    // A raider appears far beyond engagement range: for the moment it takes to
    // begin closing, the soldier stays put rather than bolting across the map.
    // (Post (12,12) to (38,38) is 52 tiles; well outside the 10-tile reach.)
    sim.debug_spawn_raider_at(Pos::new(38, 38, 1), &raws);
    for _ in 0..120 {
        sim.step(&raws);
        assert!(
            sim.dwarves[0].pos.manhattan(post) <= 3,
            "a distant foe does not draw a stationed soldier off its post"
        );
    }
    // Now drop one right beside the post: the stationed soldier engages it.
    let near = sim.debug_spawn_raider_at(Pos::new(post.x + 1, post.y, 1), &raws);
    let mut engaged = false;
    for _ in 0..4_000 {
        sim.step(&raws);
        if !sim.dwarves[near].alive {
            engaged = true;
            break;
        }
        if !sim.dwarves[0].alive {
            break;
        }
    }
    assert!(engaged, "a foe that nears the post is cut down");
}

#[test]
fn a_fallen_soldier_leaves_the_muster_rolls() {
    let (mut sim, _raws) = arena(1);
    sim.toggle_soldier(sim.dwarves[0].pos);
    assert_eq!(sim.squads.len(), 1);
    // Kill the lone soldier outright; the emptied squad is struck.
    sim.debug_kill_dwarf(0);
    assert!(!sim.dwarves[0].alive);
    assert!(sim.squads.is_empty(), "a dead soldier's squad does not haunt the rolls");
    let _ = Faction::Fort; // silence unused import
}
