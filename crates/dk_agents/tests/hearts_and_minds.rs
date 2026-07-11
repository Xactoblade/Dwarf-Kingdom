//! Phase 5 exit tests (BLUEPRINT.md §5, Phase 5), run headlessly:
//! "a player tells you an unprompted story about a specific dwarf" — made
//! concrete: dwarves develop distinct personalities and friendships, grief
//! and stress have consequences, strange moods yield named artifacts, and
//! the sim can narrate a dwarf's life back to you.

mod common;

use dk_agents::{BuildingKind, Faction, ItemKind, Sim, Task, ThoughtKind};
use dk_core::{DAYS_PER_YEAR, TICKS_PER_DAY};
use dk_world::path::Pos;

fn full_fort(seed: u64) -> (Sim, dk_raws::Raws) {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, seed);
    let mut sim = Sim::new(map, &raws, rng, 7);
    sim.invasions = false;
    sim.add_embark_supplies(&raws);
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    let barley = raws.plants.index_of("barley").unwrap();
    if let Some((fa, fb)) = sim.find_flat_patch(cx, cy) {
        sim.add_farm(fa, fb, barley);
    }
    if let Some((wa, _)) = sim.find_flat_patch(cx, cy) {
        sim.add_building(BuildingKind::Still, wa);
        sim.add_building(BuildingKind::Kitchen, Pos::new(wa.x + 1, wa.y, wa.z));
    }
    sim.place_flat_stockpiles(cx, cy, 27);
    // Boulders for strange moods to claim.
    let granite = raws.materials.index_of("granite").unwrap();
    // Deep mining takes time; seed a few surface boulders instead.
    for d in 0..3 {
        if let Some(z) = sim.map.walk_surface_z((cx + 8 + d) as usize, cy as usize) {
            sim.debug_spawn_boulder(granite, Pos::new(cx + 8 + d, cy, z as i32));
        }
    }
    (sim, raws)
}

#[test]
fn dwarves_are_individuals() {
    let (sim, raws) = full_fort(501);
    // Personalities differ measurably between dwarves.
    let a = &sim.dwarves[0].personality;
    let distinct = sim.dwarves.iter().skip(1).any(|d| {
        (d.personality.cheer - a.cheer).abs() > 10.0
            || (d.personality.social - a.social).abs() > 10.0
    });
    assert!(distinct, "seven dwarves should not share one temperament");
    // Favorites are valid raws indices and vary.
    for d in &sim.dwarves {
        assert!((d.favorite_material as usize) < raws.materials.len());
        assert!((d.favorite_crop as usize) < raws.plants.len());
    }
}

#[test]
fn friendships_form_and_grief_follows() {
    let (mut sim, raws) = full_fort(502);
    let half_year = TICKS_PER_DAY * DAYS_PER_YEAR / 2;
    for _ in 0..half_year {
        sim.step(&raws);
    }
    // Half a year of shared idle time makes friends.
    let friendship = sim
        .dwarves
        .iter()
        .enumerate()
        .find_map(|(i, d)| {
            d.relationships
                .iter()
                .find(|(_, &v)| v >= dk_agents::FRIEND_AT)
                .map(|(&j, _)| (i, j))
        });
    let Some((a, b)) = friendship else {
        panic!("no friendships after half a year of fort life");
    };
    assert!(
        sim.dwarves[a].thoughts.iter().any(|(_, t)| *t == ThoughtKind::PleasantChat),
        "friendship should come from chatting"
    );

    // Losing a friend hurts, and the survivor can say why.
    let stress_before = sim.dwarves[a].stress;
    sim.slay(b);
    assert!(
        sim.dwarves[a]
            .thoughts
            .iter()
            .any(|(_, t)| *t == ThoughtKind::FriendDied),
        "the survivor must grieve"
    );
    assert!(sim.dwarves[a].stress > stress_before, "grief is stressful");
}

#[test]
fn stress_boils_over_into_episodes() {
    let (mut sim, raws) = full_fort(503);
    sim.dwarves[0].stress = 120.0;
    sim.dwarves[0].personality.cheer = 80.0; // tantrum path
    sim.dwarves[1].stress = 120.0;
    sim.dwarves[1].personality.cheer = 20.0; // gloom path
    for _ in 0..50 {
        sim.step(&raws);
    }
    assert!(
        matches!(sim.dwarves[0].task, Task::Tantrum { .. }),
        "a cheerful dwarf under pressure explodes outward"
    );
    assert!(
        matches!(sim.dwarves[1].task, Task::Sulk { .. }),
        "a gloomy dwarf under pressure collapses inward"
    );
    assert!(sim.log.iter().any(|(_, m)| m.contains("tantrum")));

    // Episodes end and relieve pressure.
    for _ in 0..(dk_agents::EPISODE_TICKS as u64 + 100) {
        sim.step(&raws);
    }
    assert!(sim.dwarves[0].stress < 100.0);
    assert!(!matches!(sim.dwarves[0].task, Task::Tantrum { .. }));
}

#[test]
fn strange_moods_produce_named_artifacts() {
    let (mut sim, raws) = full_fort(504);
    // Run up to two years; moods roll a coin each season.
    let two_years = TICKS_PER_DAY * DAYS_PER_YEAR * 2;
    let mut made = false;
    for _ in 0..two_years {
        sim.step(&raws);
        if sim.items.iter().any(|i| i.kind == ItemKind::Artifact) {
            made = true;
            break;
        }
    }
    assert!(made, "two years should see at least one strange mood complete");
    let artifact = sim
        .items
        .iter()
        .find(|i| i.kind == ItemKind::Artifact)
        .unwrap();
    let name = artifact.name.as_ref().expect("artifacts bear names");
    assert!(name.contains("masterwork"), "unexpected artifact name: {name}");
    assert!(
        sim.log.iter().any(|(_, m)| m.contains("strange mood")),
        "the mood should be announced"
    );
    assert!(
        sim.log.iter().any(|(_, m)| m.contains(name.as_str())),
        "the creation should be celebrated in the log"
    );
    let maker = sim
        .dwarves
        .iter()
        .find(|d| d.artifacts_made > 0)
        .expect("someone made it");
    assert!(
        maker.thoughts.iter().any(|(_, t)| *t == ThoughtKind::MadeArtifact),
        "creation should be a life highlight"
    );
}

#[test]
fn the_sim_tells_a_dwarfs_story() {
    let (mut sim, raws) = full_fort(505);
    let half_year = TICKS_PER_DAY * DAYS_PER_YEAR / 2;
    for _ in 0..half_year {
        sim.step(&raws);
    }
    let bio = sim.biography(0, &raws);
    let d = &sim.dwarves[0];
    assert!(bio.contains(&d.name), "the story names its subject: {bio}");
    let trait_word = d.personality.descriptors()[0];
    assert!(bio.contains(trait_word), "the story reflects temperament: {bio}");
    let fav = &raws.materials.get(d.favorite_material).name;
    assert!(bio.contains(fav.as_str()), "the story knows their tastes: {bio}");
    // Every fort dwarf gets a distinct story.
    let bios: Vec<String> = (0..sim.dwarves.len())
        .filter(|&i| sim.dwarves[i].faction == Faction::Fort)
        .map(|i| sim.biography(i, &raws))
        .collect();
    let mut unique = bios.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), bios.len(), "no two dwarves share a biography");
}
