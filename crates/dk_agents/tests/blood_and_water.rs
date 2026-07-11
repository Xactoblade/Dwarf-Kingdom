//! Phase 3 exit tests (BLUEPRINT.md §5, Phase 3), run headlessly:
//! "a player drowns a goblin siege with a lever-operated moat, and a
//! survivor is stitched up" — here: raiders lured into a chamber, sealed in
//! by one floodgate, drowned by opening another; and a wounded defender
//! heals up by resting.

mod common;

use dk_agents::{BuildingKind, Faction, Sim};
use dk_world::path::Pos;
use dk_world::{Map, Tile};

/// Flat arena: solid bedrock at z0, walkable floor at z1.
fn flat_map(w: usize, h: usize) -> Map {
    let mut m = Map::new_air(w, h, 4, 0);
    for y in 0..w.max(h) {
        for x in 0..w {
            if y < h {
                m.set(x, y, 0, Tile::solid(2)); // granite
                m.set(x, y, 1, Tile::floor(2));
            }
        }
    }
    m
}

fn wall(m: &mut Map, x: usize, y: usize) {
    m.set(x, y, 1, Tile::solid(2));
}

#[test]
fn raiders_drown_in_the_flood_chamber() {
    let raws = common::test_raws();
    let mut m = flat_map(24, 24);

    // Killing chamber: interior x6..=14, y11..=13, walls around it, with a
    // west entrance at (5,12) and a flood inlet at (10,14)->(10,15).
    for x in 5..=15 {
        wall(&mut m, x, 10);
        if x != 10 {
            wall(&mut m, x, 14);
        }
    }
    for y in 10..=14 {
        if y != 12 {
            wall(&mut m, 5, y);
        }
        wall(&mut m, 15, y);
    }

    // Water tank north of the chamber, interior x8..=12 y16..=19, sealed,
    // draining through a gate at (10,15). Two stories of water — the flood
    // must reach drowning depth (5+) across chamber + corridor + tank floor,
    // so volume matters: 20 tiles x 7 x 2 = 280 units over ~48 tiles ≈ 5.8.
    for x in 7..=13 {
        if x != 10 {
            wall(&mut m, x, 15);
        }
        m.set(x, 15, 2, Tile::solid(2)); // z2 lip everywhere, incl. above gate
        wall(&mut m, x, 20);
        m.set(x, 20, 2, Tile::solid(2));
    }
    for y in 15..=20 {
        wall(&mut m, 7, y);
        m.set(7, y, 2, Tile::solid(2));
        wall(&mut m, 13, y);
        m.set(13, y, 2, Tile::solid(2));
    }
    for y in 16..=19 {
        for x in 8..=12 {
            m.set_water(Pos::new(x, y, 1), 7);
            m.set_water(Pos::new(x, y, 2), 7);
        }
    }

    // Bait box: a lone dwarf sealed east of the chamber — unreachable, so
    // raiders fall back to greedy approach and walk into the chamber.
    for x in 16..=20 {
        wall(&mut m, x, 10);
        wall(&mut m, x, 14);
    }
    for y in 10..=14 {
        wall(&mut m, 16, y);
        wall(&mut m, 20, y);
    }

    let rng = dk_core::rng_from_seed(3333);
    let mut sim = Sim::new(m, &raws, rng, 1);
    sim.water.springs.clear(); // no natural spring in the arena
    sim.rebuild_caches();
    sim.invasions = false;
    sim.dwarves[0].pos = Pos::new(18, 12, 1); // the bait

    // Two floodgates: chamber entrance (opened for now) and tank drain
    // (closed, holding the water back). One lever linked to the tank gate.
    let entrance = Pos::new(5, 12, 1);
    let drain = Pos::new(10, 15, 1);
    assert!(sim.add_building(BuildingKind::Floodgate, entrance));
    sim.toggle_floodgate(entrance); // open for our guests
    assert!(sim.add_building(BuildingKind::Floodgate, drain));
    let lever_pos = Pos::new(21, 21, 1);
    assert_eq!(sim.add_lever(lever_pos), Some(drain), "lever links nearest gate");

    // The raiding party arrives at the west edge.
    for p in [Pos::new(1, 12, 1), Pos::new(1, 11, 1), Pos::new(2, 12, 1)] {
        sim.spawn_raider_at(p);
    }
    assert_eq!(sim.alive_hostiles(), 3);

    // Wait for every raider to walk into the chamber (greedy approach).
    let in_chamber = |sim: &Sim| {
        sim.dwarves
            .iter()
            .filter(|d| d.alive && d.faction == Faction::Hostile)
            .all(|d| d.pos.x >= 6 && d.pos.x <= 14 && d.pos.y >= 11 && d.pos.y <= 13)
    };
    let mut ticks = 0;
    while !in_chamber(&sim) && ticks < 10_000 {
        sim.step(&raws);
        ticks += 1;
    }
    assert!(in_chamber(&sim), "raiders should stalk toward the bait, into the chamber");

    // Seal the entrance behind them, then pull the lever to open the drain.
    assert!(sim.toggle_floodgate(entrance));
    assert!(sim.pull_lever(lever_pos));

    let mut extra = 0;
    while sim.alive_hostiles() > 0 && extra < 30_000 {
        sim.step(&raws);
        extra += 1;
    }
    assert_eq!(sim.alive_hostiles(), 0, "the flood must drown every raider");
    assert_eq!(sim.stats.drownings, 3);
    assert!(sim.dwarves[0].alive, "the bait dwarf never got touched");
    assert!(
        sim.log.iter().any(|(_, m)| m.contains("drowned")),
        "the log should tell the story"
    );
}

#[test]
fn defenders_win_a_brawl_and_heal_by_resting() {
    let raws = common::test_raws();
    let m = flat_map(20, 20);
    let rng = dk_core::rng_from_seed(77);
    let mut sim = Sim::new(m, &raws, rng, 3);
    sim.water.springs.clear();
    sim.rebuild_caches();
    sim.invasions = false;

    sim.spawn_raider_at(Pos::new(10, 12, 1));

    let mut ticks = 0;
    while sim.alive_hostiles() > 0 && ticks < 30_000 {
        sim.step(&raws);
        ticks += 1;
    }
    assert_eq!(sim.alive_hostiles(), 0, "three dwarves should beat one raider");
    assert!(sim.alive_dwarves() >= 2, "the fort should survive the brawl");
    assert!(
        sim.log.iter().any(|(_, m)| m.contains("strikes")),
        "combat must be narrated"
    );
    assert_eq!(sim.stats.raiders_slain + sim.stats.drownings, 1);

    // Someone took wounds; resting heals them back to full.
    let wounded_exists = sim
        .dwarves
        .iter()
        .any(|d| d.alive && d.faction == Faction::Fort && d.is_wounded());
    assert!(wounded_exists, "a 1v3 brawl should leave scratches");

    let mut heal_ticks = 0u64;
    loop {
        // Keep the wounded napping so rest-healing applies.
        for d in &mut sim.dwarves {
            if d.alive && d.faction == Faction::Fort && d.is_wounded() && d.is_idle() {
                d.fatigue = 100.0;
            }
        }
        sim.step(&raws);
        heal_ticks += 1;
        let all_healed = sim
            .dwarves
            .iter()
            .filter(|d| d.alive && d.faction == Faction::Fort)
            .all(|d| !d.is_wounded());
        if all_healed {
            break;
        }
        assert!(heal_ticks < 200_000, "wounds should heal within a season of rest");
    }
}
