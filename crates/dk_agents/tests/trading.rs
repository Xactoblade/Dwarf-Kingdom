//! Trading exit tests, run headlessly: caravans from a named friendly civ
//! arrive on schedule, fair trades exchange goods, unfair trades are
//! refused, caravans depart, and killing a trader has consequences.

mod common;

use dk_agents::{item_value, Faction, ItemKind, Sim, TRADE_MARGIN};
use dk_core::{DAYS_PER_SEASON, DAYS_PER_YEAR, TICKS_PER_DAY};

fn trading_fort(seed: u64) -> (Sim, dk_raws::Raws) {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, seed);
    let mut sim = Sim::new(map, &raws, rng, 5);
    sim.invasions = false;
    sim.add_embark_supplies(&raws);
    sim.trade_partner = Some("the Amber Banners".to_string());
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    sim.place_flat_stockpiles(cx, cy, 27);
    // Trade stock: a pile of valuable ore boulders.
    let ore = raws.materials.index_of("hematite").unwrap();
    for d in 0..6 {
        if let Some(z) = sim.map.walk_surface_z((cx + d) as usize, (cy + 3) as usize) {
            sim.debug_spawn_boulder(ore, dk_world::path::Pos::new(cx + d, cy + 3, z as i32));
        }
    }
    (sim, raws)
}

/// Step until a caravan is present (or panic after two seasons).
fn wait_for_caravan(sim: &mut Sim, raws: &dk_raws::Raws) {
    let limit = TICKS_PER_DAY * DAYS_PER_SEASON * 2;
    for _ in 0..limit {
        sim.step(raws);
        if sim.caravan.is_some() {
            return;
        }
    }
    panic!("no caravan arrived within two seasons");
}

#[test]
fn caravans_come_trade_and_go() {
    let (mut sim, raws) = trading_fort(701);
    wait_for_caravan(&mut sim, &raws);

    let caravan = sim.caravan.as_ref().unwrap();
    assert_eq!(caravan.civ_name, "the Amber Banners", "the caravan is from our partner");
    assert!(!caravan.goods.is_empty(), "the wagon carries goods");
    assert!(
        caravan.traders.iter().all(|&t| sim.dwarves[t].faction == Faction::Visitor),
        "traders are visitors, not citizens"
    );
    assert!(sim.log.iter().any(|(_, m)| m.contains("caravan from the Amber Banners")));

    // Buy the cheapest good with enough of our ore to satisfy the margin.
    let (want_idx, want_value) = caravan
        .goods
        .iter()
        .enumerate()
        .map(|(g, it)| (g, item_value(it, &raws)))
        .min_by_key(|&(_, v)| v)
        .unwrap();
    let asked = (want_value as f32 * TRADE_MARGIN).ceil() as u32;
    let mut offer = Vec::new();
    let mut offered = 0u32;
    for (i, it) in sim.items.iter().enumerate() {
        if it.active() && it.kind == ItemKind::Boulder && it.reserved_by.is_none() {
            offer.push(i);
            offered += item_value(it, &raws);
            if offered >= asked {
                break;
            }
        }
    }
    assert!(offered >= asked, "test fort should be rich enough");

    // Lowball first: same request, no payment — the merchants refuse.
    let refusal = sim.execute_trade(&[], &[want_idx], &raws);
    assert!(refusal.is_err(), "free goods should be refused");

    let items_before = sim.items.iter().filter(|i| i.active()).count();
    sim.execute_trade(&offer, &[want_idx], &raws).expect("fair trade accepted");
    assert_eq!(sim.stats.trades_completed, 1);
    // We paid N items and received 1.
    let items_after = sim.items.iter().filter(|i| i.active()).count();
    assert_eq!(items_after, items_before - offer.len() + 1);
    assert!(
        sim.log.iter().any(|(_, m)| m.contains("Trade completed")),
        "the ledger records the deal"
    );

    // The caravan eventually leaves, traders and all.
    let leaves_at = sim.caravan.as_ref().unwrap().leaves_at;
    while sim.clock.tick <= leaves_at {
        sim.step(&raws);
    }
    assert!(sim.caravan.is_none(), "the caravan departs on schedule");
    assert!(sim.log.iter().any(|(_, m)| m.contains("departed")));
}

#[test]
fn killing_a_trader_bans_trade_for_a_year() {
    let (mut sim, raws) = trading_fort(702);
    wait_for_caravan(&mut sim, &raws);

    let victim = sim.caravan.as_ref().unwrap().traders[0];
    sim.slay(victim);
    sim.step(&raws);

    assert!(sim.caravan.is_none(), "the survivors flee");
    assert!(
        sim.log.iter().any(|(_, m)| m.contains("was killed")),
        "the outrage is recorded"
    );
    assert!(sim.stats.deaths == 0, "a dead visitor is not a lost citizen");

    // No caravan for a year...
    let ban = sim.trade_ban_until;
    assert!(ban > sim.clock.tick);
    assert!(
        ban - sim.clock.tick >= TICKS_PER_DAY * (DAYS_PER_YEAR - 1),
        "the grudge lasts about a year"
    );
    // ...and indeed none shows up next season.
    let next_season = TICKS_PER_DAY * DAYS_PER_SEASON;
    for _ in 0..next_season {
        sim.step(&raws);
        assert!(sim.caravan.is_none(), "banned civs send no caravans");
    }
}

#[test]
fn visitors_take_no_jobs_and_arent_citizens() {
    let (mut sim, raws) = trading_fort(703);
    let citizens_before = sim.alive_dwarves();
    wait_for_caravan(&mut sim, &raws);
    assert_eq!(sim.alive_dwarves(), citizens_before, "traders don't inflate the census");
    // Let them mill about a while: they must never pick up fort work.
    for _ in 0..5_000 {
        sim.step(&raws);
    }
    for &t in &sim.caravan.as_ref().unwrap().traders {
        let d = &sim.dwarves[t];
        assert!(
            d.is_idle() || matches!(d.task, dk_agents::Task::Fight { .. }),
            "trader {} took fort work: {}",
            d.name,
            d.task_name()
        );
    }
}
