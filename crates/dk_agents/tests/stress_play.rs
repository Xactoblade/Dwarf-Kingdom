//! Not an exit test — a long unattended play harness to hunt a crash the
//! player hit "after a few minutes". It builds a real fort (real raws,
//! invasions on, a living world behind it), musters squads with every order and
//! uniform, forges ammo, and steps for many in-game years across several seeds
//! — mirroring the app's `run_sim` (step + sync_world) exactly. A panic here
//! prints the exact site with RUST_BACKTRACE=1.

use dk_agents::{SiegeLeader, SiegeRoster, Sim, SquadOrder, Uniform};
use dk_history::World;
use dk_world::path::Pos;
use std::path::Path;

fn real_raws() -> dk_raws::Raws {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
    dk_raws::Raws::load(&dir).expect("load real raws")
}

/// A fort much like the app's embark, with a living world wired behind it so the
/// per-year `sync_world` path runs too.
fn play_fort(raws: &dk_raws::Raws, seed: u64) -> (Sim, World) {
    let world = World::generate(seed, 48, 48, 80);
    let region = (24usize, 24usize);
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 96, 96, 32, seed);
    let mut sim = Sim::new(map, raws, rng, 7);
    sim.add_embark_supplies(raws);
    sim.add_starting_dogs();
    sim.add_starting_cat();
    sim.plant_trees(120, raws);
    sim.plant_shrubs(60);
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
    sim.embark_world_year = world.years_simulated;
    sim.rebuild_caches();
    (sim, world)
}

/// Enlist survivors and split them across squads with a mix of orders and
/// uniforms — exercising the whole military system the way a player would.
/// Safe to call repeatedly as the fort loses and gains soldiers.
fn muster(sim: &mut Sim) {
    let ps: Vec<Pos> = sim
        .dwarves
        .iter()
        .filter(|d| d.alive && !d.soldier)
        .take(5)
        .map(|d| d.pos)
        .collect();
    for p in &ps {
        sim.toggle_soldier(*p);
    }
    // Split off a marksdwarf squad, if there's more than one soldier to spare.
    if let Some(last) = sim
        .dwarves
        .iter()
        .rposition(|d| d.alive && d.soldier)
    {
        if let Some(new) = sim.split_to_new_squad(last) {
            sim.set_squad_uniform(new, Uniform::Ranged);
            let z = sim.dwarves[last].pos.z;
            sim.set_squad_order(new, SquadOrder::Station(Pos::new(48, 48, z)));
        }
    }
    sim.bolts = sim.bolts.max(300);
    if !sim.squads.is_empty() {
        let z = sim.dwarves[0].pos.z;
        sim.set_squad_order(0, SquadOrder::Patrol(Pos::new(20, 48, z), Pos::new(76, 48, z)));
    }
}

/// Slow (minutes) — a regression guard, not part of the normal suite. Run it on
/// demand with `cargo test -p dk_agents --release -- --ignored stress`.
#[ignore = "long-running crash/soak guard; run with --ignored"]
#[test]
fn a_fort_survives_a_long_unattended_watch() {
    let raws = real_raws();
    for seed in [1u64, 42, 777, 31337] {
        let (mut sim, mut world) = play_fort(&raws, seed);
        muster(&mut sim);
        // ~400k ticks ≈ 3 in-game years, well past several invasion waves.
        for tick in 0..400_000u64 {
            sim.step(&raws);
            // Exactly what the app's run_sim does every tick.
            dk_agents::sync_world(&mut sim, &mut world);
            if tick % 50_000 == 49_999 {
                muster(&mut sim);
            }
        }
    }
}
