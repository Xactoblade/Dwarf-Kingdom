//! Phase 7 (BLUEPRINT.md §5, Phase 7): "sites that develop offscreen".
//!
//! The world does not hold still while you dig. For every year a fortress
//! lives, the world outside lives one too: wars grind on, sites burn, named
//! figures die of old age, and word of it all reaches the gates.

mod common;

use dk_agents::{Sim, SiegeLeader, SiegeRoster};
use dk_core::{DAYS_PER_YEAR, TICKS_PER_DAY};
use dk_history::World;

/// A fort wired to a world exactly as the app wires it at embark.
fn fort_in_world(seed: u64) -> (Sim, World, dk_raws::Raws) {
    let world = World::generate(seed, 48, 48, 80);
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, seed);
    let mut sim = Sim::new(map, &raws, rng, 5);
    let region = (24, 24);
    sim.home_region = Some(region);
    sim.embark_world_year = world.years_simulated;
    sim.trade_partner = world
        .nearest_friendly_civ(region.0, region.1)
        .map(|c| c.name.clone());
    if let Some((civ_name, leaders)) = world.siege_pack(region.0, region.1) {
        sim.siege_roster = Some(SiegeRoster {
            civ_name,
            leaders: leaders
                .into_iter()
                .map(|(name, grudge)| SiegeLeader { name, grudge })
                .collect(),
        });
    }
    (sim, world, raws)
}

/// Fast-forward the fort's calendar without simulating every tick — the
/// world's clock keys off the fort's year, and a real stepped year is
/// covered by `a_played_year_ages_the_world_outside` below.
fn skip_years(sim: &mut Sim, years: u64) {
    sim.clock.tick += TICKS_PER_DAY * DAYS_PER_YEAR * years;
}

#[test]
fn a_played_year_ages_the_world_outside() {
    let (mut sim, mut world, raws) = fort_in_world(2027);
    let started = world.years_simulated;

    // A full year at the fort, stepped honestly, tick by tick.
    for _ in 0..TICKS_PER_DAY * DAYS_PER_YEAR {
        sim.step(&raws);
        dk_agents::sync_world(&mut sim, &mut world);
    }

    assert_eq!(sim.clock.year(), 2, "a year has passed at the fort");
    assert_eq!(
        world.years_simulated,
        started + 1,
        "and exactly one year has passed in the world outside"
    );
}

#[test]
fn the_world_outside_keeps_its_own_history_while_the_fort_lives() {
    let (mut sim, mut world, _raws) = fort_in_world(2027);
    let started = world.years_simulated;
    let events_before = world.events.len();

    skip_years(&mut sim, 30);
    dk_agents::sync_world(&mut sim, &mut world);

    assert_eq!(world.years_simulated, started + 30, "thirty years of fort, thirty of world");
    assert!(
        world.events.len() > events_before,
        "those thirty years wrote fresh history"
    );
    assert!(
        world.legends_lines().iter().any(|l| {
            l.split_whitespace()
                .nth(1)
                .and_then(|y| y.trim_end_matches(':').parse::<u32>().ok())
                .is_some_and(|y| y > started)
        }),
        "the new history reads back in Legends, stamped after the embark year"
    );
}

#[test]
fn news_of_the_fort_s_own_neighbors_reaches_the_gates() {
    // Sweep seeds: not every world happens to make news about the two civs a
    // given fort actually knows, but the mechanism must work somewhere.
    let heard = (0..12u64).any(|seed| {
        let (mut sim, mut world, _raws) = fort_in_world(seed);
        skip_years(&mut sim, 40);
        dk_agents::sync_world(&mut sim, &mut world);
        sim.log
            .iter()
            .any(|(_, m)| m.starts_with("Word arrives from afar:"))
    });
    assert!(heard, "a fort hears word of the peoples it trades with and fights");
}

#[test]
fn a_fort_hears_nothing_of_peoples_it_has_never_met() {
    let (mut sim, mut world, _raws) = fort_in_world(2027);
    // A fort that knows no one abroad: no partner, no enemy.
    sim.trade_partner = None;
    sim.siege_roster = None;
    skip_years(&mut sim, 40);
    dk_agents::sync_world(&mut sim, &mut world);

    assert!(
        !sim.log
            .iter()
            .any(|(_, m)| m.starts_with("Word arrives from afar:")),
        "the world's noise never reaches a fort with no neighbors it knows"
    );
    assert!(
        world.years_simulated > 80,
        "but the world turned all the same, whether the fort heard it or not"
    );
}

#[test]
fn a_trade_partner_razed_out_of_existence_stops_sending_caravans() {
    let (mut sim, mut world, _raws) = fort_in_world(2027);
    // Trade with a friendly people that still stands, so razing it below is a
    // real fall — worldgen's beasts and wars may already have wiped the civ the
    // fort would otherwise have picked.
    let civ = world
        .civs
        .iter()
        .position(|c| !c.race.hostile() && c.sites.iter().any(|&s| !world.sites[s].ruined))
        .expect("some friendly people still stands");
    sim.trade_partner = Some(world.civs[civ].name.clone());

    // The wars abroad go badly for them: every site falls.
    for s in world.civs[civ].sites.clone() {
        world.sites[s].ruined = true;
    }

    skip_years(&mut sim, 1);
    dk_agents::sync_world(&mut sim, &mut world);

    assert!(sim.trade_partner.is_none(), "a fallen people sends no more wagons");
    assert!(
        sim.log.iter().any(|(_, m)| m.contains("has been destroyed")),
        "and the fort is told why the caravans stopped coming"
    );
}

#[test]
fn the_fort_always_keeps_a_named_enemy() {
    // Sieges are the fort's clock of doom; a refresh must never silently
    // leave it with no one to fear.
    for seed in 0..8u64 {
        let (mut sim, mut world, _raws) = fort_in_world(seed);
        let had_enemy = sim.siege_roster.is_some();
        skip_years(&mut sim, 60);
        dk_agents::sync_world(&mut sim, &mut world);
        if had_enemy && !world.civ_fallen(&sim.siege_roster.clone().map(|r| r.civ_name).unwrap_or_default()) {
            let roster = sim.siege_roster.as_ref().expect("a living enemy civ still musters leaders");
            assert!(!roster.leaders.is_empty(), "seed {seed}: the roster is never emptied");
        }
    }
}

#[test]
fn the_fort_hears_of_a_city_burning_in_the_world_it_knows() {
    // The payoff of a living world: a place the fort trades with is ground
    // down by a war the player never sees, and word of it reaches the gates.
    //
    // Swept rather than pinned to one lucky seed: not every world burns a city
    // the fort happens to know within twenty years, and a worldgen change would
    // otherwise break this test for no good reason.
    let heard = (0..14u64).any(|seed| {
        let (mut sim, mut world, _raws) = fort_in_world(seed);
        skip_years(&mut sim, 20);
        dk_agents::sync_world(&mut sim, &mut world);
        sim.log
            .iter()
            .any(|(_, m)| m.starts_with("Word arrives from afar:") && m.contains("razed"))
    });
    assert!(heard, "a city of a people the fort knows burns, and the fort hears of it");
}

#[test]
fn the_same_seed_lives_the_same_sixty_years() {
    let run = |seed: u64| {
        let (mut sim, mut world, _raws) = fort_in_world(seed);
        skip_years(&mut sim, 60);
        dk_agents::sync_world(&mut sim, &mut world);
        (world.legends_lines(), sim.log.clone())
    };
    let (legends_a, log_a) = run(2027);
    let (legends_b, log_b) = run(2027);
    assert_eq!(legends_a, legends_b, "a world's live history is deterministic in its seed");
    assert_eq!(log_a, log_b, "and so is the news the fort hears");
}
