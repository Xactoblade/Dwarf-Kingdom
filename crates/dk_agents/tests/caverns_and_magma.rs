//! Caverns & magma exit tests, run headlessly: the deep world exists,
//! magma flows and obeys floodgates, meeting water makes obsidian, and
//! standing in magma is exactly as survivable as it sounds.

mod common;

use dk_agents::Sim;
use dk_world::path::Pos;
use dk_world::{Map, Tile};

#[test]
fn deep_maps_have_caverns_and_magma() {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(801);
    let map = dk_world::generate(&raws.materials, &mut rng, 96, 96, 32, 801);

    // Caverns: open walkable galleries deep below the surface.
    let cav_lo = 32 / 8;
    let cav_hi = 32 / 4;
    let mut cavern_floor = 0;
    let mut magma_tiles = 0;
    for z in 1..=cav_hi {
        for y in 0..96 {
            for x in 0..96 {
                let t = map.get(x, y, z);
                if (cav_lo..=cav_hi).contains(&z) && t.shape.is_walkable() && t.magma == 0 {
                    cavern_floor += 1;
                }
                if t.magma > 0 {
                    magma_tiles += 1;
                }
            }
        }
    }
    assert!(cavern_floor > 200, "expected real cavern galleries, got {cavern_floor} floor tiles");
    assert!(magma_tiles > 100, "expected magma pockets at depth, got {magma_tiles}");

    // Magma tiles are never walkable.
    for z in 0..32 {
        for y in 0..96 {
            for x in 0..96 {
                if map.get(x, y, z).magma > 0 {
                    assert!(
                        !map.walkable(Pos::new(x as i32, y as i32, z as i32)),
                        "magma at ({x},{y},{z}) must not be pathable"
                    );
                }
            }
        }
    }
}

/// Flat arena at z1 over bedrock.
fn arena() -> Map {
    let mut m = Map::new_air(16, 16, 4, 0);
    for y in 0..16 {
        for x in 0..16 {
            m.set(x, y, 0, Tile::solid(2));
            m.set(x, y, 1, Tile::floor(2));
        }
    }
    m
}

#[test]
fn water_meets_magma_and_becomes_obsidian() {
    let raws = common::test_raws();
    let mut m = arena();
    // A pool of magma and a head of water beside it.
    m.set_magma(Pos::new(8, 8, 1), 7);
    for x in 4..7 {
        m.set_water(Pos::new(x, 8, 1), 7);
    }
    let rng = dk_core::rng_from_seed(802);
    let mut sim = Sim::new(m, &raws, rng, 1);
    sim.water.springs.clear();
    sim.invasions = false;
    sim.rebuild_caches();

    for _ in 0..2_000 {
        sim.step(&raws);
        let obsidian_formed = sim
            .log
            .iter()
            .any(|(_, msg)| msg.contains("obsidian"));
        if obsidian_formed {
            break;
        }
    }
    assert!(
        sim.log.iter().any(|(_, msg)| msg.contains("obsidian")),
        "water flowing into magma must forge obsidian"
    );
    // Somewhere near the meeting line there is now new solid stone at z1.
    let mut new_walls = 0;
    for x in 4..=9 {
        if sim.map.get(x, 8, 1).is_solid() {
            new_walls += 1;
        }
    }
    assert!(new_walls > 0, "the obsidian should be real, mineable stone");
}

#[test]
fn obsidian_never_seals_in_a_tree_or_shrub() {
    // Regression (tree-regrowth review): a tile hardening to obsidian is another
    // way a surface square turns solid, so it must clear any tree/shrub standing
    // there -- else a phantom is sealed inside unmineable rock.
    let raws = common::test_raws();
    let mut m = arena();
    m.set_magma(Pos::new(8, 8, 1), 7);
    for x in 4..7 {
        m.set_water(Pos::new(x, 8, 1), 7);
    }
    let rng = dk_core::rng_from_seed(804);
    let mut sim = Sim::new(m, &raws, rng, 1);
    sim.water.springs.clear();
    sim.invasions = false;
    sim.rebuild_caches();
    // Blanket the meeting line with vegetation (cap stays 0 -> no regrowth).
    let oak = raws.materials.indices_in_category(dk_raws::MaterialCategory::Wood)[0];
    for x in 4..=9 {
        sim.trees.insert(Pos::new(x, 8, 1), oak);
        sim.shrubs.insert(Pos::new(x, 8, 1));
    }

    for _ in 0..2_000 {
        sim.step(&raws);
        if sim.log.iter().any(|(_, msg)| msg.contains("obsidian")) {
            break;
        }
    }
    assert!(
        sim.log.iter().any(|(_, msg)| msg.contains("obsidian")),
        "obsidian must form for this test to mean anything"
    );
    for &p in sim.trees.keys() {
        assert!(
            !sim.map.get(p.x as usize, p.y as usize, p.z as usize).is_solid(),
            "a tree is sealed inside solid obsidian at {p:?}"
        );
    }
    for &p in &sim.shrubs {
        assert!(
            !sim.map.get(p.x as usize, p.y as usize, p.z as usize).is_solid(),
            "a shrub is sealed inside solid obsidian at {p:?}"
        );
    }
}

#[test]
fn magma_is_lethal_and_gates_hold_it_back() {
    let raws = common::test_raws();
    let mut m = arena();
    // Magma chamber west of a wall, gate at (8,8,1), victim east.
    for y in 0..16 {
        m.set(8, y, 1, Tile::solid(2));
    }
    m.set(
        8, 8, 1,
        Tile { material: 2, shape: dk_world::TileShape::Gate, water: 0, magma: 0 },
    );
    for y in 3..=13 {
        for x in 4..8 {
            m.set_magma(Pos::new(x, y, 1), 7);
        }
    }
    // Wall the citizen into a safe box (13..15, 1..3) so the raider can't
    // reach them and get itself killed in a brawl before the magma comes.
    for x in 12..16 {
        m.set(x, 0, 1, Tile::solid(2));
        m.set(x, 4, 1, Tile::solid(2));
    }
    for y in 0..5 {
        m.set(12, y, 1, Tile::solid(2));
    }
    let rng = dk_core::rng_from_seed(803);
    let mut sim = Sim::new(m, &raws, rng, 1);
    sim.water.springs.clear();
    sim.invasions = false;
    sim.rebuild_caches();
    sim.dwarves[0].pos = Pos::new(14, 2, 1); // safe in the box
    sim.spawn_raider_at(Pos::new(10, 8, 1), &raws);
    let raider = sim.dwarves.len() - 1;

    // Sealed: the raider is safe for a while.
    for _ in 0..2_000 {
        sim.step(&raws);
    }
    assert!(sim.dwarves[raider].alive, "a closed gate must hold the magma");
    assert_eq!(sim.map.magma_at(Pos::new(9, 8, 1)), 0);

    // Open the gate: fire finds a way.
    assert!(sim.toggle_floodgate(Pos::new(8, 8, 1)));
    let mut ticks = 0;
    while sim.dwarves[raider].alive && ticks < 30_000 {
        sim.step(&raws);
        ticks += 1;
    }
    assert!(!sim.dwarves[raider].alive, "magma should reach and incinerate the raider");
    assert!(
        sim.log.iter().any(|(_, m)| m.contains("incinerated")),
        "the death should be narrated"
    );
    assert_eq!(sim.stats.raiders_slain, 1, "incinerated raiders count as slain");
    assert!(sim.dwarves[0].alive, "the bystander was never at risk");
}
